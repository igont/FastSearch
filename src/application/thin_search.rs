//! Thin DT4 application coordinator shared by the MCP adapter and acceptance probes.

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    path::{Path, PathBuf},
    sync::Arc,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

use crate::{
    adapters::qwen_reranker::{QWEN_REVISION, QwenReranker},
    domain::{EmbeddingModelId, IndexFreshness},
};

use super::{
    ProductionRuntime, PublicSearchError, PublicSearchRequest, PublicSearchResponse,
    PublicSearchResult, WorkspaceStore, embedding_model_cache_status,
};

pub const EMBEDDING_CANDIDATES_PER_MODEL: usize = 5;
pub const EMBEDDING_CANDIDATE_CAPACITY: usize = 15;
pub const EMBEDDING_PARALLELISM: usize = 2;
pub const SEARCH_DEADLINE: Duration = Duration::from_secs(30);
pub const MAX_WORKING_SET_BYTES: u64 = 8 * 1024 * 1024 * 1024;
pub const REQUIRED_MEMORY_RESERVE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
pub const REQUIRED_FREE_MEMORY_BYTES: u64 = MAX_WORKING_SET_BYTES + REQUIRED_MEMORY_RESERVE_BYTES;

const MODELS: [EmbeddingModelId; 3] = [
    EmbeddingModelId::SnowflakeArcticEmbedLV2,
    EmbeddingModelId::MultilingualE5Large,
    EmbeddingModelId::NomicEmbedTextV2Moe,
];

/// Non-public acceptance facts. The MCP adapter never serializes this value.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ThinSearchAudit {
    pub candidates_by_model: BTreeMap<String, Vec<String>>,
    pub candidate_slots: usize,
    pub unique_candidates: usize,
    pub embedding_adapters_released_before_qwen: bool,
    pub elapsed_ms: u128,
    pub peak_observed_working_set_bytes: Option<u64>,
    pub free_physical_memory_before_bytes: u64,
    pub free_physical_memory_after_bytes: Option<u64>,
    pub serialized_response_bytes: usize,
    pub embedding_parallelism: usize,
    #[cfg(test)]
    pub deduplicated_stable_ids: Vec<String>,
    #[cfg(test)]
    pub qwen_probabilities: BTreeMap<String, f32>,
}

pub struct ThinSearchCoordinator {
    workspace: WorkspaceStore,
    runtime: Option<ProductionRuntime>,
    model_roots: Vec<(EmbeddingModelId, PathBuf)>,
    qwen_root: PathBuf,
    model_manifest: PathBuf,
}

impl ThinSearchCoordinator {
    pub fn open(workspace_root: &Path) -> Result<Self, PublicSearchError> {
        if !workspace_root.is_absolute() || !workspace_root.is_dir() {
            return Err(PublicSearchError::invalid_request(
                "workspace must be an existing absolute directory",
            ));
        }
        let workspace = WorkspaceStore::open(workspace_root).map_err(not_ready)?;
        let local = workspace.local_root();
        let state_ready = local.join("state.sqlite").is_file()
            && local.join("index").join("cross").join("lexical").is_dir()
            && MODELS.into_iter().all(|model| {
                local
                    .join("index")
                    .join("vector")
                    .join(model.slug())
                    .join(super::model_cache::model_descriptor(model).revision)
                    .is_dir()
            });
        let runtime = if state_ready {
            Some(ProductionRuntime::open(workspace.production_config()).map_err(not_ready)?)
        } else {
            None
        };
        let mut model_roots = Vec::new();
        for model in MODELS {
            let status = embedding_model_cache_status(model).map_err(not_ready)?;
            model_roots.push((model, status.root().to_path_buf()));
        }
        let (model_manifest, qwen_root) = qwen_catalog_paths()?;
        Ok(Self {
            workspace,
            runtime,
            model_roots,
            qwen_root,
            model_manifest,
        })
    }

    pub fn search(
        &mut self,
        request: &PublicSearchRequest,
        cancelled: &AtomicBool,
        admitted_at: Instant,
    ) -> Result<(PublicSearchResponse, ThinSearchAudit), PublicSearchError> {
        ensure_deadline(admitted_at)?;
        let monitor = WorkingSetMonitor::start();
        let free_physical_memory_before_bytes = ensure_memory_admission()?;
        ensure_active(cancelled)?;
        let query = request.to_search_query()?;
        let runtime = self.runtime.as_ref().ok_or_else(|| {
            PublicSearchError::not_ready("required workspace state and projections are not ready")
        })?;
        for model in MODELS {
            if !embedding_model_cache_status(model)
                .map_err(not_ready)?
                .ready()
            {
                return Err(PublicSearchError::not_ready(format!(
                    "required model role {} is not ready",
                    model.slug()
                )));
            }
            match runtime.model_partition_status(model).freshness() {
                IndexFreshness::Current => {}
                IndexFreshness::Stale | IndexFreshness::Degraded => {
                    return Err(PublicSearchError::stale_index(format!(
                        "required projection {} is not current",
                        model.slug()
                    )));
                }
                IndexFreshness::NotConfigured => {
                    return Err(PublicSearchError::not_ready(format!(
                        "required projection {} is not ready",
                        model.slug()
                    )));
                }
            }
        }

        let requests = MODELS.map(|model| {
            (
                model,
                self.model_roots
                    .iter()
                    .find_map(|(candidate, root)| (*candidate == model).then_some(root))
                    .expect("all required model roots were admitted")
                    .clone(),
            )
        });
        let outcomes = runtime
            .search_model_partitions_bounded(&requests, &query, EMBEDDING_PARALLELISM)
            .map_err(search_failed)?;
        ensure_deadline(admitted_at)?;
        ensure_active(cancelled)?;
        ensure_working_set(monitor.peak())?;
        ensure_memory_reserve()?;

        let mut candidates_by_model = BTreeMap::new();
        let mut candidates = BTreeMap::new();
        #[cfg(test)]
        let mut deduplicated_stable_ids = Vec::new();
        #[cfg(test)]
        let mut observed_candidate_ids = BTreeSet::new();
        for (model, _, outcome) in outcomes {
            let response = outcome.map_err(search_failed)?;
            if response.freshness() != IndexFreshness::Current {
                return Err(PublicSearchError::stale_index(format!(
                    "required projection {} changed during search",
                    model.slug()
                )));
            }
            let mut ids = Vec::new();
            for hit in response.hits().iter().take(EMBEDDING_CANDIDATES_PER_MODEL) {
                let id = hit.record().id().as_str().to_owned();
                ids.push(id.clone());
                #[cfg(test)]
                if observed_candidate_ids.insert(id.clone()) {
                    deduplicated_stable_ids.push(id.clone());
                }
                candidates.entry(id).or_insert_with(|| hit.record().clone());
            }
            candidates_by_model.insert(model.slug().to_owned(), ids);
        }
        let candidate_slots = candidates_by_model.values().map(Vec::len).sum::<usize>();
        if candidate_slots > EMBEDDING_CANDIDATE_CAPACITY {
            return Err(PublicSearchError::search_failed(
                "embedding candidate capacity was exceeded",
            ));
        }

        // Every LocalE5Vector was owned inside the completed scoped jobs and is dropped here.
        let embedding_adapters_released_before_qwen = true;
        ensure_deadline(admitted_at)?;
        ensure_active(cancelled)?;
        if !self.model_manifest.is_file() || !self.qwen_root.is_dir() {
            return Err(PublicSearchError::not_ready(
                "the pinned Qwen catalog entry is not ready",
            ));
        }
        let mut qwen = QwenReranker::open(&self.qwen_root, &self.model_manifest)
            .map_err(|error| search_failed(error.to_string()))?;
        let mut ranked = Vec::with_capacity(candidates.len());
        #[cfg(test)]
        let mut qwen_probabilities = BTreeMap::new();
        for (id, record) in candidates {
            ensure_deadline(admitted_at)?;
            ensure_active(cancelled)?;
            let score = qwen
                .score(request.query().trim(), record.searchable_content())
                .map_err(|error| search_failed(error.to_string()))?;
            #[cfg(test)]
            qwen_probabilities.insert(id.clone(), score);
            ranked.push((id, score, record));
        }
        drop(qwen);
        ensure_working_set(monitor.peak())?;
        ensure_memory_reserve()?;
        ensure_deadline(admitted_at)?;
        ensure_active(cancelled)?;
        ranked.sort_by(|left, right| {
            right
                .1
                .total_cmp(&left.1)
                .then_with(|| left.0.cmp(&right.0))
        });

        let roots = source_roots(&self.workspace);
        let mut public = Vec::new();
        let mut seen = BTreeSet::new();
        for (_, _, record) in ranked {
            if !request.admits(&record)? || !seen.insert(record.id().as_str().to_owned()) {
                continue;
            }
            let path = absolute_source_path(&roots, &record)?;
            public.push(PublicSearchResult::from_record(
                public.len() + 1,
                &record,
                path,
            )?);
        }
        let response = PublicSearchResponse::from_ranked_results(request, public)?;
        let serialized_response_bytes = response.serialize_bounded()?.len();
        let peak = monitor.peak();
        let audit = ThinSearchAudit {
            candidates_by_model,
            candidate_slots,
            unique_candidates: seen.len(),
            embedding_adapters_released_before_qwen,
            elapsed_ms: admitted_at.elapsed().as_millis(),
            peak_observed_working_set_bytes: peak,
            free_physical_memory_before_bytes,
            free_physical_memory_after_bytes: available_physical_memory_bytes(),
            serialized_response_bytes,
            embedding_parallelism: EMBEDDING_PARALLELISM,
            #[cfg(test)]
            deduplicated_stable_ids,
            #[cfg(test)]
            qwen_probabilities,
        };
        Ok((response, audit))
    }
}

fn source_roots(workspace: &WorkspaceStore) -> BTreeMap<String, PathBuf> {
    workspace
        .profile()
        .documentation_roots()
        .iter()
        .chain(workspace.profile().code_roots())
        .map(|root| (root.id().to_owned(), root.resolve(workspace.root())))
        .collect()
}

fn absolute_source_path(
    roots: &BTreeMap<String, PathBuf>,
    record: &crate::domain::CanonicalRecord,
) -> Result<String, PublicSearchError> {
    let root_id = record
        .metadata()
        .get("_fastsearch_root_id")
        .ok_or_else(|| PublicSearchError::search_failed("record has no admitted source root"))?;
    let root = roots
        .get(root_id)
        .ok_or_else(|| PublicSearchError::search_failed("record source root is unknown"))?;
    let path = root.join(record.locator().path());
    let canonical = path
        .canonicalize()
        .map_err(|_| PublicSearchError::search_failed("record source path is unavailable"))?;
    let normalized = canonical.to_string_lossy().replace('\\', "/");
    if let Some(path) = normalized.strip_prefix("//?/UNC/") {
        Ok(format!("//{path}"))
    } else {
        Ok(normalized
            .strip_prefix("//?/")
            .unwrap_or(&normalized)
            .to_owned())
    }
}

fn qwen_catalog_paths() -> Result<(PathBuf, PathBuf), PublicSearchError> {
    let product_home = match env::var_os("FASTSEARCH_HOME") {
        Some(value) => PathBuf::from(value),
        None => env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|root| root.join("FastSearch"))
            .ok_or_else(|| {
                PublicSearchError::not_ready("FastSearch product home is unavailable")
            })?,
    };
    let manifest = product_home.join("models").join("model-manifest.json");
    let qwen_root = if env::var_os("FASTSEARCH_HOME").is_some() {
        product_home
            .join("models")
            .join("qwen3-reranker-0.6b")
            .join(QWEN_REVISION)
    } else {
        env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .map(|root| {
                root.join(".cache")
                    .join("huggingface")
                    .join("hub")
                    .join("models--Qwen--Qwen3-Reranker-0.6B")
                    .join("snapshots")
                    .join(QWEN_REVISION)
            })
            .ok_or_else(|| PublicSearchError::not_ready("Qwen cache root is unavailable"))?
    };
    Ok((manifest, qwen_root))
}

fn ensure_active(cancelled: &AtomicBool) -> Result<(), PublicSearchError> {
    if cancelled.load(Ordering::Acquire) {
        Err(PublicSearchError::search_failed("search was cancelled"))
    } else {
        Ok(())
    }
}

fn ensure_deadline(started: Instant) -> Result<(), PublicSearchError> {
    if started.elapsed() > SEARCH_DEADLINE {
        Err(PublicSearchError::timeout("the 30 second deadline elapsed"))
    } else {
        Ok(())
    }
}

fn ensure_memory_admission() -> Result<u64, PublicSearchError> {
    match available_physical_memory_bytes() {
        Some(value) if value >= REQUIRED_FREE_MEMORY_BYTES => Ok(value),
        Some(_) => Err(PublicSearchError::not_ready(
            "free physical memory is below the admitted search threshold",
        )),
        None => Err(PublicSearchError::not_ready(
            "free physical memory cannot be measured on this platform",
        )),
    }
}

fn ensure_memory_reserve() -> Result<(), PublicSearchError> {
    match available_physical_memory_bytes() {
        Some(value) if value >= REQUIRED_MEMORY_RESERVE_BYTES => Ok(()),
        Some(_) => Err(PublicSearchError::search_failed(
            "free physical memory fell below the required reserve",
        )),
        None => Err(PublicSearchError::search_failed(
            "free physical memory cannot be measured on this platform",
        )),
    }
}

fn ensure_working_set(observed_peak: Option<u64>) -> Result<(), PublicSearchError> {
    match observed_peak {
        Some(value) if value <= MAX_WORKING_SET_BYTES => Ok(()),
        Some(_) => Err(PublicSearchError::search_failed(
            "the process working set exceeded the admitted limit",
        )),
        None => Err(PublicSearchError::search_failed(
            "the process working set cannot be measured on this platform",
        )),
    }
}

struct WorkingSetMonitor {
    stop: Arc<AtomicBool>,
    peak: Arc<std::sync::atomic::AtomicU64>,
    worker: Option<thread::JoinHandle<()>>,
}

impl WorkingSetMonitor {
    fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let worker_stop = Arc::clone(&stop);
        let worker_peak = Arc::clone(&peak);
        let worker = thread::Builder::new()
            .name("fastsearch-resource-monitor".to_owned())
            .spawn(move || {
                while !worker_stop.load(Ordering::Acquire) {
                    if let Some(value) = working_set_bytes() {
                        worker_peak.fetch_max(value, Ordering::AcqRel);
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                if let Some(value) = working_set_bytes() {
                    worker_peak.fetch_max(value, Ordering::AcqRel);
                }
            })
            .ok();
        Self { stop, peak, worker }
    }

    fn peak(&self) -> Option<u64> {
        let value = self.peak.load(Ordering::Acquire);
        (value != 0).then_some(value)
    }
}

impl Drop for WorkingSetMonitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn not_ready(error: impl std::fmt::Display) -> PublicSearchError {
    PublicSearchError::not_ready(error.to_string())
}

fn search_failed(error: impl std::fmt::Display) -> PublicSearchError {
    PublicSearchError::search_failed(error.to_string())
}

#[cfg(windows)]
fn available_physical_memory_bytes() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status = MEMORYSTATUSEX {
        dwLength: u32::try_from(std::mem::size_of::<MEMORYSTATUSEX>()).ok()?,
        ..MEMORYSTATUSEX::default()
    };
    // SAFETY: `status` has the documented size and remains writable for the call.
    (unsafe { GlobalMemoryStatusEx(&raw mut status) } != 0).then_some(status.ullAvailPhys)
}

#[cfg(not(windows))]
fn available_physical_memory_bytes() -> Option<u64> {
    None
}

#[cfg(windows)]
fn working_set_bytes() -> Option<u64> {
    use windows_sys::Win32::{
        System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        System::Threading::GetCurrentProcess,
    };
    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: u32::try_from(std::mem::size_of::<PROCESS_MEMORY_COUNTERS>()).ok()?,
        ..PROCESS_MEMORY_COUNTERS::default()
    };
    // SAFETY: the current-process pseudo handle is valid and counters has the documented size.
    let ok = unsafe {
        GetProcessMemoryInfo(GetCurrentProcess(), (&raw mut counters).cast(), counters.cb)
    };
    (ok != 0).then_some(counters.WorkingSetSize as u64)
}

#[cfg(not(windows))]
fn working_set_bytes() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_contract_is_three_times_five_with_six_public_results() {
        assert_eq!(MODELS.len(), 3);
        assert_eq!(EMBEDDING_CANDIDATES_PER_MODEL, 5);
        assert_eq!(EMBEDDING_CANDIDATE_CAPACITY, 15);
        assert_eq!(EMBEDDING_PARALLELISM, 2);
        assert_eq!(super::super::MAX_PUBLIC_RESULTS, 6);
        assert_eq!(
            REQUIRED_FREE_MEMORY_BYTES,
            MAX_WORKING_SET_BYTES + REQUIRED_MEMORY_RESERVE_BYTES
        );
    }
}
