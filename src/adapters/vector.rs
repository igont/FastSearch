//! Local-only multilingual-E5 derived vector projection.
//!
//! The adapter receives canonical records from the SQLite authority.  It never
//! writes them back, and a model/content identity mismatch makes its projection
//! stale until the caller supplies the authoritative record set again.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Instant,
};

#[cfg(test)]
use std::{fs, fs::File, io::Read};

mod partition;
mod verified_provider;

use verified_provider::VerifiedProvider;

use crate::{
    domain::{
        BackendKind, CanonicalRecord, Capability, CapabilityStatus, EmbeddingModelId, ErrorKind,
        ExecutionDevice, FastSearchError, IndexFreshness, LifecycleStatus, ModelIdentity,
        ProjectionProvenance, RetrievalChannel, SearchHit, SearchQuery, SearchResponse,
    },
    ports::VectorRetrieval,
};

/// Ensures the selected runtime can download, open and produce a finite vector.
/// This readiness probe never reads or indexes workspace content.
pub fn prepare_embedding_model(
    model_id: EmbeddingModelId,
    cache_root: &std::path::Path,
    show_download_progress: bool,
) -> Result<(), FastSearchError> {
    let mut provider =
        VerifiedProvider::acquire(cache_root, model_id, show_download_progress, true)?;
    let vector = provider.embed_query("FastSearch model readiness probe")?;
    if vector.len() != model_id.dimension() {
        return Err(FastSearchError::new(
            ErrorKind::CapabilityUnavailable {
                capability: Capability::VectorRetrieval,
            },
            format!(
                "embedding model {} returned {} dimensions instead of {}",
                model_id.slug(),
                vector.len(),
                model_id.dimension()
            ),
        ));
    }
    Ok(())
}

/// Probes one concrete execution device without reading workspace content.
pub fn probe_embedding_model_device(
    model_id: EmbeddingModelId,
    cache_root: &Path,
    device: ExecutionDevice,
) -> Result<(), FastSearchError> {
    let mut provider =
        VerifiedProvider::acquire_on_device(cache_root, model_id, false, true, device)?;
    let vector = provider.embed_query("FastSearch execution device probe")?;
    if vector.len() != model_id.dimension() {
        return Err(FastSearchError::new(
            ErrorKind::CapabilityUnavailable {
                capability: Capability::VectorRetrieval,
            },
            format!(
                "{} probe returned {} dimensions instead of {}",
                device.label(),
                vector.len(),
                model_id.dimension()
            ),
        ));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct EmbeddingBatchMeasurement {
    batch_size: usize,
    duration_ms: u128,
    documents_per_second: f64,
    working_set_bytes: Option<u64>,
}

impl EmbeddingBatchMeasurement {
    #[must_use]
    pub const fn batch_size(&self) -> usize {
        self.batch_size
    }

    #[must_use]
    pub const fn duration_ms(&self) -> u128 {
        self.duration_ms
    }

    #[must_use]
    pub const fn documents_per_second(&self) -> f64 {
        self.documents_per_second
    }

    #[must_use]
    pub const fn working_set_bytes(&self) -> Option<u64> {
        self.working_set_bytes
    }
}

/// Benchmarks CPU inference with one loaded runtime. Every candidate is
/// measured three times and reported by median wall-clock duration.
pub fn benchmark_embedding_batches(
    model_id: EmbeddingModelId,
    cache_root: &Path,
    texts: &[String],
    batch_sizes: &[usize],
) -> Result<Vec<EmbeddingBatchMeasurement>, FastSearchError> {
    benchmark_embedding_batches_on_device(
        model_id,
        cache_root,
        texts,
        batch_sizes,
        ExecutionDevice::Cpu,
    )
}

pub fn benchmark_embedding_batches_on_device(
    model_id: EmbeddingModelId,
    cache_root: &Path,
    texts: &[String],
    batch_sizes: &[usize],
    device: ExecutionDevice,
) -> Result<Vec<EmbeddingBatchMeasurement>, FastSearchError> {
    if texts.is_empty() || batch_sizes.is_empty() || batch_sizes.contains(&0) {
        return Err(FastSearchError::new(
            ErrorKind::InvalidContent,
            "batch benchmark requires texts and positive batch sizes",
        ));
    }
    let mut provider =
        VerifiedProvider::acquire_on_device(cache_root, model_id, false, true, device)?;
    let warmup_count = texts.len().min(8);
    provider.embed_benchmark_texts(&texts[..warmup_count], 1)?;

    let mut measurements = Vec::with_capacity(batch_sizes.len());
    for &batch_size in batch_sizes {
        let mut rounds = Vec::with_capacity(3);
        for _ in 0..3 {
            let started = Instant::now();
            provider.embed_benchmark_texts(texts, batch_size)?;
            rounds.push(started.elapsed());
        }
        rounds.sort_unstable();
        let median = rounds[1];
        measurements.push(EmbeddingBatchMeasurement {
            batch_size,
            duration_ms: median.as_millis(),
            documents_per_second: texts.len() as f64 / median.as_secs_f64(),
            working_set_bytes: super::process_metrics::working_set_bytes(),
        });
    }
    Ok(measurements)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VectorPartitionMetrics {
    size_bytes: u64,
    build_duration_ms: u64,
}

impl VectorPartitionMetrics {
    #[must_use]
    pub const fn size_bytes(self) -> u64 {
        self.size_bytes
    }

    #[must_use]
    pub const fn build_duration_ms(self) -> u64 {
        self.build_duration_ms
    }
}

#[derive(Clone)]
pub(super) struct ProjectedRecord {
    pub(super) record: CanonicalRecord,
    pub(super) content_hash: String,
    pub(super) vector: Vec<f32>,
}
struct ProjectionState {
    model_root: PathBuf,
    model_id: EmbeddingModelId,
    allow_catalog_download: bool,
    model_identity: String,
    model_manifest: Option<String>,
    partition_root: Option<PathBuf>,
    records: BTreeMap<String, ProjectedRecord>,
    state_generation: u64,
    projection_generation: Option<u64>,
    freshness: IndexFreshness,
    detail: String,
}

/// Rebuildable local vector projection using only explicitly supplied model files.
pub struct LocalE5Vector {
    operation: Mutex<()>,
    state: Mutex<ProjectionState>,
}

/// Provenance of the currently usable derived projection.  Consumers must not
/// confuse it with the authoritative SQLite generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VectorProjectionProvenance {
    model_identity: String,
    state_generation: u64,
    projection_generation: Option<u64>,
}

impl VectorProjectionProvenance {
    #[must_use]
    pub fn model_identity(&self) -> &str {
        &self.model_identity
    }
    #[must_use]
    pub const fn state_generation(&self) -> u64 {
        self.state_generation
    }
    #[must_use]
    pub const fn projection_generation(&self) -> Option<u64> {
        self.projection_generation
    }
}

impl LocalE5Vector {
    #[must_use]
    pub fn open(model_root: impl Into<PathBuf>, model_identity: impl Into<String>) -> Self {
        Self::open_internal(
            model_root,
            model_identity,
            EmbeddingModelId::MultilingualE5Small,
            false,
        )
    }

    #[must_use]
    pub fn open_with_model(
        model_root: impl Into<PathBuf>,
        model_identity: impl Into<String>,
        model_id: EmbeddingModelId,
    ) -> Self {
        Self::open_internal(model_root, model_identity, model_id, true)
    }

    fn open_internal(
        model_root: impl Into<PathBuf>,
        model_identity: impl Into<String>,
        model_id: EmbeddingModelId,
        allow_catalog_download: bool,
    ) -> Self {
        Self {
            operation: Mutex::new(()),
            state: Mutex::new(ProjectionState {
                model_root: model_root.into(),
                model_id,
                allow_catalog_download,
                model_identity: model_identity.into(),
                model_manifest: None,
                partition_root: None,
                records: BTreeMap::new(),
                state_generation: 0,
                projection_generation: None,
                freshness: IndexFreshness::NotConfigured,
                detail: "vector projection is absent".to_owned(),
            }),
        }
    }

    #[must_use]
    pub fn open_persistent_with_model(
        model_root: impl Into<PathBuf>,
        model_identity: impl Into<String>,
        model_id: EmbeddingModelId,
        partition_root: impl Into<PathBuf>,
    ) -> Self {
        let vector = Self::open_internal(model_root, model_identity, model_id, true);
        if let Ok(mut state) = vector.state.lock() {
            state.partition_root = Some(partition_root.into());
        }
        vector
    }

    /// Re-admits an already materialized partition against the current
    /// canonical record set without opening the embedding model.
    pub fn restore(
        &self,
        records: &[CanonicalRecord],
        state_generation: u64,
    ) -> Result<LifecycleStatus, FastSearchError> {
        let _operation = self.lock_operation()?;
        let (partition_root, model_id, model_identity) = {
            let state = self.lock()?;
            (
                state.partition_root.clone(),
                state.model_id,
                state.model_identity.clone(),
            )
        };
        let Some(partition_root) = partition_root else {
            let mut state = self.lock()?;
            state.state_generation = state_generation;
            state.freshness = IndexFreshness::Stale;
            state.detail = "persistent vector partition is not configured".to_owned();
            return Ok(status(&state));
        };
        match partition::load(
            &partition_root,
            partition::ExpectedPartition {
                model_id,
                model_identity: &model_identity,
                state_generation,
                records,
            },
        ) {
            Ok(Some(loaded)) => {
                let mut state = self.lock()?;
                state.records = loaded.records;
                state.model_manifest = Some(loaded.manifest.artifact_manifest);
                state.state_generation = state_generation;
                state.projection_generation = Some(state_generation);
                state.freshness = IndexFreshness::Current;
                state.detail =
                    format!("persistent vector partition is current for {model_identity}");
                Ok(status(&state))
            }
            Ok(None) => {
                let mut state = self.lock()?;
                state.records.clear();
                state.model_manifest = None;
                state.state_generation = state_generation;
                state.projection_generation = None;
                state.freshness = IndexFreshness::Stale;
                state.detail = format!("vector partition is absent or stale for {model_identity}");
                Ok(status(&state))
            }
            Err(error) => {
                let mut state = self.lock()?;
                state.records.clear();
                state.model_manifest = None;
                state.state_generation = state_generation;
                state.projection_generation = None;
                state.freshness = IndexFreshness::Degraded;
                state.detail = error.message().to_owned();
                Ok(status(&state))
            }
        }
    }

    #[must_use]
    pub fn persistent_status(
        partition_root: &Path,
        model_id: EmbeddingModelId,
        model_identity: &str,
        records: &[CanonicalRecord],
        state_generation: u64,
    ) -> LifecycleStatus {
        let vector = Self::open_persistent_with_model(
            PathBuf::new(),
            model_identity,
            model_id,
            partition_root,
        );
        vector
            .restore(records, state_generation)
            .unwrap_or_else(|error| {
                LifecycleStatus::new(
                    IndexFreshness::Degraded,
                    state_generation,
                    None,
                    error.message(),
                )
            })
    }

    pub fn persistent_metrics(
        partition_root: &Path,
    ) -> Result<Option<VectorPartitionMetrics>, FastSearchError> {
        Ok(
            partition::stored_metrics(partition_root)?.map(|metrics| VectorPartitionMetrics {
                size_bytes: metrics.size_bytes,
                build_duration_ms: metrics.build_duration_ms,
            }),
        )
    }

    /// Applies the complete authoritative record set. Removed IDs disappear from
    /// the derived projection because the map is replaced atomically after E5 succeeds.
    pub fn apply(
        &self,
        records: &[CanonicalRecord],
        state_generation: u64,
    ) -> Result<LifecycleStatus, FastSearchError> {
        let _operation = self.lock_operation()?;
        self.apply_locked(records, state_generation)
    }

    fn apply_locked(
        &self,
        records: &[CanonicalRecord],
        state_generation: u64,
    ) -> Result<LifecycleStatus, FastSearchError> {
        let build_started = Instant::now();
        let (root, identity, model_id, allow_catalog_download) = {
            let state = self.lock()?;
            (
                state.model_root.clone(),
                state.model_identity.clone(),
                state.model_id,
                state.allow_catalog_download,
            )
        };
        let mut verified =
            VerifiedProvider::acquire(&root, model_id, false, allow_catalog_download)
                .map_err(|error| self.provider_failed(state_generation, error))?;
        let manifest = verified.manifest.clone();
        let unchanged = {
            let state = self.lock()?;
            state.freshness == IndexFreshness::Current
                && state.model_manifest.as_deref() == Some(&manifest)
                && state.records.len() == records.len()
                && records.iter().all(|record| {
                    state
                        .records
                        .get(record.id().as_str())
                        .is_some_and(|projected| {
                            projected.content_hash == record.content_hash().as_str()
                        })
                })
        };
        if unchanged {
            let mut state = self.lock()?;
            state.state_generation = state_generation;
            state.projection_generation = Some(state_generation);
            return Ok(status(&state));
        }
        let vectors = verified
            .embed_records(records)
            .map_err(|error| self.provider_failed(state_generation, error))?;
        let mut next = BTreeMap::new();
        for (record, vector) in records.iter().cloned().zip(vectors) {
            if next.contains_key(record.id().as_str()) {
                return Err(FastSearchError::new(
                    ErrorKind::ProjectionFailure,
                    "vector projection input contains a duplicate stable identifier",
                ));
            }
            next.insert(
                record.id().as_str().to_owned(),
                ProjectedRecord {
                    content_hash: record.content_hash().as_str().to_owned(),
                    record,
                    vector,
                },
            );
        }
        let partition_root = {
            let mut state = self.lock()?;
            // Reconfiguration during embedding is a causal stale result, never Current.
            if state.model_identity != identity
                || state.model_root != root
                || state.model_id != model_id
                || state
                    .model_manifest
                    .as_deref()
                    .is_some_and(|current| current != manifest)
            {
                state.freshness = IndexFreshness::Stale;
                state.detail = "model identity changed while projection was building".to_owned();
                return Ok(status(&state));
            }
            state.partition_root.clone()
        };
        if let Some(partition_root) = partition_root {
            partition::save(
                &partition_root,
                &partition::PartitionManifest {
                    schema_version: 2,
                    model_slug: model_id.slug().to_owned(),
                    model_identity: identity.clone(),
                    artifact_manifest: manifest.clone(),
                    runtime_contract: partition::VECTOR_RUNTIME_CONTRACT.to_owned(),
                    dimension: model_id.dimension(),
                    state_generation,
                    corpus_fingerprint: partition::corpus_fingerprint(records),
                    record_count: records.len(),
                    build_duration_ms: None,
                },
                &next,
                build_started,
            )?;
        }
        let mut state = self.lock()?;
        if state.model_identity != identity
            || state.model_root != root
            || state.model_id != model_id
        {
            state.freshness = IndexFreshness::Stale;
            state.detail = "model identity changed while projection was publishing".to_owned();
            return Ok(status(&state));
        }
        state.records = next;
        state.model_manifest = Some(manifest);
        state.state_generation = state_generation;
        state.projection_generation = Some(state_generation);
        state.freshness = IndexFreshness::Current;
        state.detail = format!("local E5 projection is current for {identity}");
        Ok(status(&state))
    }

    pub fn rebuild(
        &self,
        records: &[CanonicalRecord],
        state_generation: u64,
    ) -> Result<LifecycleStatus, FastSearchError> {
        self.apply(records, state_generation)
    }

    /// A complete model identity change invalidates all derived vectors.
    pub fn reconfigure(
        &self,
        model_root: impl Into<PathBuf>,
        model_identity: impl Into<String>,
    ) -> Result<(), FastSearchError> {
        let _operation = self.lock_operation()?;
        let mut state = self.lock()?;
        state.model_root = model_root.into();
        state.model_identity = model_identity.into();
        state.model_manifest = None;
        state.records.clear();
        state.projection_generation = None;
        state.freshness = IndexFreshness::Stale;
        state.detail = "model identity changed; vector projection must rebuild".to_owned();
        Ok(())
    }

    #[must_use]
    pub fn lifecycle_status(&self) -> LifecycleStatus {
        match self.lock() {
            Ok(state) => status(&state),
            Err(_) => LifecycleStatus::new(
                IndexFreshness::Degraded,
                0,
                None,
                "vector state lock failed",
            ),
        }
    }

    #[must_use]
    pub fn capability_status(&self) -> CapabilityStatus {
        match self.lifecycle_status().freshness() {
            IndexFreshness::Current => {
                CapabilityStatus::available(Capability::VectorRetrieval, BackendKind::Real)
            }
            IndexFreshness::Degraded => CapabilityStatus::degraded(
                Capability::VectorRetrieval,
                self.lifecycle_status().detail(),
            ),
            _ => CapabilityStatus::unavailable(
                Capability::VectorRetrieval,
                self.lifecycle_status().detail(),
            ),
        }
    }

    #[must_use]
    pub fn provenance(&self) -> VectorProjectionProvenance {
        match self.lock() {
            Ok(state) => VectorProjectionProvenance {
                model_identity: state.model_identity.clone(),
                state_generation: state.state_generation,
                projection_generation: state.projection_generation,
            },
            Err(_) => VectorProjectionProvenance {
                model_identity: "unavailable".to_owned(),
                state_generation: 0,
                projection_generation: None,
            },
        }
    }

    fn provider_failed(&self, generation: u64, error: FastSearchError) -> FastSearchError {
        if let Ok(mut state) = self.lock() {
            state.state_generation = generation;
            state.projection_generation = None;
            state.freshness = IndexFreshness::Degraded;
            state.detail = error.message().to_owned();
        }
        error
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, ProjectionState>, FastSearchError> {
        self.state.lock().map_err(|_| {
            FastSearchError::new(
                ErrorKind::ProjectionFailure,
                "vector state lock is poisoned",
            )
        })
    }

    fn lock_operation(&self) -> Result<std::sync::MutexGuard<'_, ()>, FastSearchError> {
        self.operation.lock().map_err(|_| {
            FastSearchError::new(
                ErrorKind::ProjectionFailure,
                "vector operation lock is poisoned",
            )
        })
    }
}

impl VectorRetrieval for LocalE5Vector {
    fn search(&self, query: &SearchQuery) -> Result<SearchResponse, FastSearchError> {
        let _operation = self.lock_operation()?;
        let (
            root,
            model_id,
            allow_catalog_download,
            entries,
            freshness,
            generation,
            projection_generation,
            declared_identity,
            manifest,
        ) = {
            let state = self.lock()?;
            (
                state.model_root.clone(),
                state.model_id,
                state.allow_catalog_download,
                state.records.values().cloned().collect::<Vec<_>>(),
                state.freshness,
                state.state_generation,
                state.projection_generation,
                state.model_identity.clone(),
                state.model_manifest.clone(),
            )
        };
        if freshness != IndexFreshness::Current {
            return Ok(SearchResponse::with_freshness(Vec::new(), freshness));
        }
        let mut verified =
            match VerifiedProvider::acquire(&root, model_id, false, allow_catalog_download) {
                Ok(value) => value,
                Err(error) => {
                    self.provider_failed(generation, error);
                    return Ok(SearchResponse::with_freshness(
                        Vec::new(),
                        IndexFreshness::Degraded,
                    ));
                }
            };
        if manifest.as_deref() != Some(&verified.manifest) {
            let mut state = self.lock()?;
            state.freshness = IndexFreshness::Stale;
            state.projection_generation = None;
            state.detail =
                "local E5 artifact manifest changed; vector projection must rebuild".to_owned();
            return Ok(SearchResponse::with_freshness(
                Vec::new(),
                IndexFreshness::Stale,
            ));
        }
        let query_vector = verified
            .embed_query(query.text())
            .map_err(|error| self.provider_failed(generation, error))?;
        let provenance = projection_generation
            .zip(manifest)
            .map(|(projection, fingerprint)| {
                projection_provenance(&declared_identity, fingerprint, generation, projection)
            })
            .transpose()?;
        let mut hits = entries
            .into_iter()
            .map(|entry| {
                let hit = SearchHit::new(
                    entry.record,
                    RetrievalChannel::Vector,
                    f64::from(cosine(&query_vector, &entry.vector)),
                );
                match &provenance {
                    Some(value) => hit.with_projection_provenance(value.clone()),
                    None => hit,
                }
            })
            .collect::<Vec<_>>();
        hits.sort_by(|left, right| {
            right
                .score()
                .total_cmp(&left.score())
                .then_with(|| left.record().id().cmp(right.record().id()))
        });
        Ok(SearchResponse::with_freshness(
            hits,
            IndexFreshness::Current,
        ))
    }
}

fn status(state: &ProjectionState) -> LifecycleStatus {
    LifecycleStatus::new(
        state.freshness,
        state.state_generation,
        state.projection_generation,
        &state.detail,
    )
}

fn projection_provenance(
    declared: &str,
    fingerprint: String,
    state_generation: u64,
    projection_generation: u64,
) -> Result<ProjectionProvenance, FastSearchError> {
    let (model, revision) = declared.rsplit_once('@').ok_or_else(|| {
        FastSearchError::new(
            ErrorKind::InvalidIdentifier,
            "local E5 model identity must be model@upstream-revision",
        )
    })?;
    let identity = ModelIdentity::new(model, revision, fingerprint)?;
    Ok(ProjectionProvenance::new(
        identity,
        state_generation,
        projection_generation,
    ))
}

fn cosine(left: &[f32], right: &[f32]) -> f32 {
    left.iter().zip(right).map(|(a, b)| a * b).sum()
}

#[cfg(test)]
mod boundary_tests {
    #[test]
    fn projection_lifecycle_delegates_to_verified_provider_boundary() {
        let lifecycle = include_str!("vector.rs");
        assert!(lifecycle.contains("verified_provider::VerifiedProvider"));
        assert!(!lifecycle.contains(concat!("windows", "_sys::")));
        assert!(!lifecycle.contains(concat!("fn ", "verified_model(")));
        assert!(!lifecycle.contains(concat!("fn ", "verified_snapshot(")));
        assert!(!lifecycle.contains(concat!("fn ", "open_verified_file(")));
    }
}

#[cfg(test)]
mod security_tests {
    use super::*;
    use crate::domain::{ContentHash, RecordKind, SearchMode, SourceLocator, StableId};
    use std::{
        process::Command,
        sync::{Arc, Mutex, mpsc},
        thread,
        time::{Duration, SystemTime},
    };

    struct TempTree(PathBuf);

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn disposable_model_copy() -> TempTree {
        let source = PathBuf::from(std::env::var("FASTSEARCH_E5_MODEL_ROOT").unwrap());
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "fastsearch-b2-model-{}-{unique}",
            std::process::id()
        ));
        copy_tree(&source, &root);
        TempTree(root)
    }

    fn copy_tree(source: &Path, target: &Path) {
        fs::create_dir_all(target).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let source_path = entry.path();
            let target_path = target.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&source_path, &target_path);
            } else {
                fs::copy(source_path, target_path).unwrap();
            }
        }
    }

    fn record() -> CanonicalRecord {
        CanonicalRecord::new(
            StableId::parse("b2-race").unwrap(),
            RecordKind::MarkdownSection,
            SourceLocator::markdown("race.md", ["race"]).unwrap(),
            "race",
            "semantic navigation immutable provider",
            BTreeMap::new(),
            Vec::new(),
            ContentHash::parse("race-v1").unwrap(),
        )
        .unwrap()
    }

    #[cfg(windows)]
    fn create_junction(link: &Path, target: &Path) -> std::process::Output {
        Command::new("cmd.exe")
            .args([
                "/d",
                "/c",
                "mklink",
                "/J",
                link.to_str().unwrap(),
                target.to_str().unwrap(),
            ])
            .output()
            .unwrap()
    }

    #[test]
    #[ignore = "requires FASTSEARCH_E5_MODEL_ROOT local-only cache"]
    fn verified_model_denies_mutation_and_replacement_until_provider_finishes() {
        let fixture = disposable_model_copy();
        let root = fixture.0.clone();
        let external = TempTree(root.with_extension("external-sentinel"));
        fs::create_dir_all(&external.0).unwrap();
        let sentinel = external.0.join("sentinel.txt");
        fs::write(&sentinel, b"unchanged").unwrap();
        let mutation_result = Arc::new(Mutex::new(None));
        let replacement_result = Arc::new(Mutex::new(None));
        let junction_result = Arc::new(Mutex::new(None));
        let mutation_capture = Arc::clone(&mutation_result);
        let replacement_capture = Arc::clone(&replacement_result);
        let junction_capture = Arc::clone(&junction_result);
        let config = root.join("onnx").join("config.json");
        let onnx = root.join("onnx");
        let replacement = root.join("onnx-race-replacement");
        let external_target = external.0.clone();
        verified_provider::install_verify_load_hook(move || {
            *mutation_capture.lock().unwrap() = Some(fs::write(&config, b"mutation"));
            *replacement_capture.lock().unwrap() = Some(fs::rename(&onnx, &replacement));
            #[cfg(windows)]
            {
                *junction_capture.lock().unwrap() = Some(create_junction(&onnx, &external_target));
            }
        });

        let adapter = LocalE5Vector::open(&root, "multilingual-e5-small@614241f");
        assert_eq!(
            adapter.apply(&[record()], 1).unwrap().freshness(),
            IndexFreshness::Current
        );
        assert!(mutation_result.lock().unwrap().take().unwrap().is_err());
        assert!(replacement_result.lock().unwrap().take().unwrap().is_err());
        #[cfg(windows)]
        assert!(
            !junction_result
                .lock()
                .unwrap()
                .take()
                .unwrap()
                .status
                .success()
        );
        let query = SearchQuery::new("semantic navigation", SearchMode::Balanced).unwrap();
        let response = adapter.search(&query).unwrap();
        assert_eq!(response.freshness(), IndexFreshness::Current);
        assert_eq!(response.hits().len(), 1);
        assert!(response.hits()[0].projection_provenance().is_some());
        assert_eq!(fs::read(&sentinel).unwrap(), b"unchanged");

        // A pre-existing inner junction is the fail-closed control: it is
        // rejected before bytes are admitted and cannot publish Current/hits.
        let saved = root.join("onnx-saved");
        fs::rename(root.join("onnx"), &saved).unwrap();
        #[cfg(windows)]
        assert!(
            create_junction(&root.join("onnx"), &external.0)
                .status
                .success()
        );
        let attacked = LocalE5Vector::open(&root, "multilingual-e5-small@614241f");
        assert!(attacked.apply(&[record()], 2).is_err());
        assert_ne!(
            attacked.lifecycle_status().freshness(),
            IndexFreshness::Current
        );
        let failed = attacked.search(&query).unwrap();
        assert!(failed.hits().is_empty());
        assert_ne!(failed.freshness(), IndexFreshness::Current);
        #[cfg(windows)]
        Command::new("cmd.exe")
            .args(["/d", "/c", "rmdir", root.join("onnx").to_str().unwrap()])
            .status()
            .unwrap();
        fs::rename(saved, root.join("onnx")).unwrap();
        assert_eq!(fs::read(&sentinel).unwrap(), b"unchanged");

        // Search and reconfigure are linearized: reconfiguration cannot finish
        // while inference owns the operation token, so old hits never emerge
        // after the new configuration becomes observable.
        let adapter = Arc::new(adapter);
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        verified_provider::install_verify_load_hook(move || {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        });
        let searching = Arc::clone(&adapter);
        let query_copy = query.clone();
        let search = thread::spawn(move || searching.search(&query_copy).unwrap());
        entered_rx.recv_timeout(Duration::from_secs(30)).unwrap();
        let (configured_tx, configured_rx) = mpsc::channel();
        let configuring = Arc::clone(&adapter);
        let configured_root = root.clone();
        let reconfigure = thread::spawn(move || {
            configuring
                .reconfigure(configured_root, "multilingual-e5-small@new")
                .unwrap();
            configured_tx.send(()).unwrap();
        });
        assert!(
            configured_rx
                .recv_timeout(Duration::from_millis(100))
                .is_err()
        );
        release_tx.send(()).unwrap();
        let completed = search.join().unwrap();
        assert_eq!(completed.freshness(), IndexFreshness::Current);
        configured_rx.recv_timeout(Duration::from_secs(30)).unwrap();
        reconfigure.join().unwrap();
        assert_ne!(
            adapter.lifecycle_status().freshness(),
            IndexFreshness::Current
        );
        assert!(adapter.search(&query).unwrap().hits().is_empty());
    }

    #[test]
    #[ignore = "requires FASTSEARCH_E5_MODEL_ROOT local-only cache"]
    fn opened_file_size_and_eof_are_bounded_by_b1_allowlist() {
        let fixture = disposable_model_copy();
        let tokenizer = fixture.0.join("onnx").join("tokenizer_config.json");
        let expected = fs::metadata(&tokenizer).unwrap().len();
        let mut grown = fs::read(&tokenizer).unwrap();
        grown.extend(std::iter::repeat_n(0_u8, 1_048_576));
        let exchange = TempTree(fixture.0.with_extension("bounded-read-exchange"));
        fs::create_dir_all(&exchange.0).unwrap();
        let grown_path = exchange.0.join("grown.json");
        let saved_path = exchange.0.join("saved.json");
        fs::write(&grown_path, &grown).unwrap();

        // Executable vulnerable control reproduces the exact baseline order:
        // pathname size check, replacement, then unrestricted second open/read.
        assert_eq!(fs::metadata(&tokenizer).unwrap().len(), expected);
        fs::rename(&tokenizer, &saved_path).unwrap();
        fs::rename(&grown_path, &tokenizer).unwrap();
        let mut vulnerable = Vec::new();
        File::open(&tokenizer)
            .unwrap()
            .read_to_end(&mut vulnerable)
            .unwrap();
        assert_eq!(
            vulnerable.len(),
            usize::try_from(expected).unwrap() + 1_048_576
        );
        fs::rename(&tokenizer, &grown_path).unwrap();
        fs::rename(&saved_path, &tokenizer).unwrap();

        let mut acquired = verified_provider::open_verified_file(&grown_path).unwrap();
        let error =
            verified_provider::read_exact_allowlisted_file(&mut acquired, expected).unwrap_err();
        assert!(error.message().contains("opened model file size mismatch"));
        assert_eq!(fs::metadata(&tokenizer).unwrap().len(), expected);
    }
}
