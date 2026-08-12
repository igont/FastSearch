//! Local-only multilingual-E5 derived vector projection.
//!
//! The adapter receives canonical records from the SQLite authority.  It never
//! writes them back, and a model/content identity mismatch makes its projection
//! stale until the caller supplies the authoritative record set again.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
};

#[cfg(windows)]
use std::os::windows::{
    ffi::OsStrExt,
    fs::{MetadataExt, OpenOptionsExt},
    io::{AsRawHandle, RawHandle},
};

#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_LIST_DIRECTORY,
        FILE_SHARE_READ, FileAttributeTagInfo, GetFileInformationByHandleEx, OPEN_EXISTING,
    },
};

use fastembed::{
    InitOptionsUserDefined, Pooling, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel,
};
use sha2::{Digest, Sha256};

use crate::{
    domain::{
        BackendKind, CanonicalRecord, Capability, CapabilityStatus, ErrorKind, FastSearchError,
        IndexFreshness, LifecycleStatus, ModelIdentity, ProjectionProvenance, RetrievalChannel,
        SearchHit, SearchQuery, SearchResponse,
    },
    ports::VectorRetrieval,
};

#[derive(Clone)]
struct ProjectedRecord {
    record: CanonicalRecord,
    content_hash: String,
    vector: Vec<f32>,
}

struct ProjectionState {
    model_root: PathBuf,
    model_identity: String,
    model_manifest: Option<String>,
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
        Self {
            operation: Mutex::new(()),
            state: Mutex::new(ProjectionState {
                model_root: model_root.into(),
                model_identity: model_identity.into(),
                model_manifest: None,
                records: BTreeMap::new(),
                state_generation: 0,
                projection_generation: None,
                freshness: IndexFreshness::NotConfigured,
                detail: "vector projection is absent".to_owned(),
            }),
        }
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
        let (root, identity) = {
            let state = self.lock()?;
            (state.model_root.clone(), state.model_identity.clone())
        };
        let verified =
            verified_model(&root).map_err(|error| self.provider_failed(state_generation, error))?;
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
        let vectors = embed_verified(verified, records)
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
        let mut state = self.lock()?;
        // Reconfiguration during embedding is a causal stale result, never Current.
        if state.model_identity != identity
            || state.model_root != root
            || state
                .model_manifest
                .as_deref()
                .is_some_and(|current| current != manifest)
        {
            state.freshness = IndexFreshness::Stale;
            state.detail = "model identity changed while projection was building".to_owned();
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
        let verified = match verified_model(&root) {
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
        let query_vector = embed_texts_verified(verified, &[query.text().to_owned()])
            .map_err(|error| self.provider_failed(generation, error))?
            .into_iter()
            .next()
            .ok_or_else(|| {
                FastSearchError::new(ErrorKind::ProjectionFailure, "E5 returned no query vector")
            })?;
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

fn embed_verified(
    verified: VerifiedModel,
    records: &[CanonicalRecord],
) -> Result<Vec<Vec<f32>>, FastSearchError> {
    let texts = records
        .iter()
        .map(|record| format!("{}\n{}", record.title(), record.searchable_content()))
        .collect::<Vec<_>>();
    embed_texts_verified(verified, &texts)
}

fn embed_texts_verified(
    verified: VerifiedModel,
    texts: &[String],
) -> Result<Vec<Vec<f32>>, FastSearchError> {
    run_verify_load_hook();
    let mut runtime =
        TextEmbedding::try_new_from_user_defined(verified.model, InitOptionsUserDefined::default())
            .map_err(provider_error)?;
    let vectors = runtime.embed(texts, Some(1)).map_err(provider_error)?;
    if vectors.len() != texts.len()
        || vectors
            .iter()
            .any(|vector| vector.is_empty() || vector.iter().any(|value| !value.is_finite()))
    {
        return Err(FastSearchError::new(
            ErrorKind::ProjectionFailure,
            "local E5 returned an invalid vector",
        ));
    }
    Ok(vectors.into_iter().map(normalize).collect())
}

struct VerifiedModel {
    model: UserDefinedEmbeddingModel,
    manifest: String,
    _files: Vec<File>,
    _directories: Vec<DirectoryGuard>,
}

fn verified_model(root: &Path) -> Result<VerifiedModel, FastSearchError> {
    let snapshot = verified_snapshot(root)?;
    let required = [
        "onnx\\model.onnx",
        "onnx\\tokenizer.json",
        "onnx\\config.json",
        "onnx\\special_tokens_map.json",
        "onnx\\tokenizer_config.json",
    ];
    if required
        .iter()
        .any(|name| !snapshot.bytes.contains_key(*name))
    {
        return Err(FastSearchError::new(
            ErrorKind::CapabilityUnavailable {
                capability: Capability::VectorRetrieval,
            },
            "B2_NO_LOCAL_E5_PROVIDER",
        ));
    }
    let read = |name: &str| {
        snapshot.bytes.get(name).cloned().ok_or_else(|| {
            FastSearchError::new(
                ErrorKind::CapabilityUnavailable {
                    capability: Capability::VectorRetrieval,
                },
                "B2_NO_LOCAL_E5_PROVIDER",
            )
        })
    };
    let model = UserDefinedEmbeddingModel::new(
        read("onnx\\model.onnx")?,
        TokenizerFiles {
            tokenizer_file: read("onnx\\tokenizer.json")?,
            config_file: read("onnx\\config.json")?,
            special_tokens_map_file: read("onnx\\special_tokens_map.json")?,
            tokenizer_config_file: read("onnx\\tokenizer_config.json")?,
        },
    )
    .with_pooling(Pooling::Mean);
    Ok(VerifiedModel {
        model,
        manifest: snapshot.manifest,
        _files: snapshot.files,
        _directories: snapshot.directories,
    })
}

/// B1 accepted complete E5 cache set: canonical locator/bytes/full-SHA256 root.
const B1_E5_MANIFEST_ROOT: &str =
    "63A0FA9AEC56D0A3F5080D82956111F4BBEE57BF0A3637371CF16E451B194D0E";

const B1_E5_FILES: &[(&str, u64)] = &[
    (".eval_results\\ArguAna.yaml", 595),
    (".eval_results\\BrightAopsRetrieval.yaml", 663),
    (".eval_results\\BrightBiologyLongRetrieval.yaml", 673),
    (".eval_results\\BrightBiologyRetrieval.yaml", 669),
    (".eval_results\\BrightEarthScienceLongRetrieval.yaml", 685),
    (".eval_results\\BrightEarthScienceRetrieval.yaml", 681),
    (".eval_results\\BrightEconomicsLongRetrieval.yaml", 677),
    (".eval_results\\BrightEconomicsRetrieval.yaml", 673),
    (".eval_results\\BrightLeetcodeRetrieval.yaml", 673),
    (".eval_results\\BrightPonyLongRetrieval.yaml", 667),
    (".eval_results\\BrightPonyRetrieval.yaml", 663),
    (".eval_results\\BrightPsychologyLongRetrieval.yaml", 677),
    (".eval_results\\BrightPsychologyRetrieval.yaml", 675),
    (".eval_results\\BrightRoboticsLongRetrieval.yaml", 675),
    (".eval_results\\BrightRoboticsRetrieval.yaml", 669),
    (".eval_results\\BrightStackoverflowLongRetrieval.yaml", 687),
    (".eval_results\\BrightStackoverflowRetrieval.yaml", 681),
    (
        ".eval_results\\BrightSustainableLivingLongRetrieval.yaml",
        695,
    ),
    (".eval_results\\BrightSustainableLivingRetrieval.yaml", 689),
    (".eval_results\\BrightTheoremQAQuestionsRetrieval.yaml", 691),
    (".eval_results\\BrightTheoremQATheoremsRetrieval.yaml", 689),
    (".gitattributes", 1_606),
    ("1_Pooling\\config.json", 206),
    ("config.json", 681),
    ("model.safetensors", 134),
    ("modules.json", 406),
    ("onnx\\config.json", 678),
    ("onnx\\model.onnx", 470_268_510),
    ("onnx\\model_O4.onnx", 235_052_531),
    ("onnx\\model_qint8_avx512_vnni.onnx", 118_346_824),
    ("onnx\\sentencepiece.bpe.model", 5_069_051),
    ("onnx\\special_tokens_map.json", 176),
    ("onnx\\tokenizer.json", 17_082_730),
    ("onnx\\tokenizer_config.json", 463),
    ("openvino\\openvino_model.bin", 134),
    ("openvino\\openvino_model.xml", 375_553),
    ("pytorch_model.bin", 134),
    ("README.md", 516_022),
    ("sentence_bert_config.json", 60),
    ("sentencepiece.bpe.model", 132),
    ("special_tokens_map.json", 176),
    ("tokenizer.json", 17_082_730),
    ("tokenizer_config.json", 463),
];

const B1_E5_DIRECTORIES: &[&str] = &[".eval_results", "1_Pooling", "onnx", "openvino"];

struct VerifiedSnapshot {
    bytes: BTreeMap<String, Vec<u8>>,
    manifest: String,
    files: Vec<File>,
    directories: Vec<DirectoryGuard>,
}

fn verified_snapshot(root: &Path) -> Result<VerifiedSnapshot, FastSearchError> {
    fn collect(
        root: &Path,
        directory: &Path,
        bytes: &mut BTreeMap<String, Vec<u8>>,
        files: &mut Vec<File>,
        directories: &mut Vec<DirectoryGuard>,
    ) -> Result<(), FastSearchError> {
        ensure_not_link_or_reparse(directory)?;
        if directory != root {
            let locator = directory
                .strip_prefix(root)
                .map_err(provider_error)?
                .to_string_lossy()
                .replace('/', "\\");
            if !B1_E5_DIRECTORIES.contains(&locator.as_str()) {
                return Err(provider_error("unexpected model directory"));
            }
        }
        directories.push(open_directory_guard(directory)?);
        for entry in fs::read_dir(directory).map_err(provider_error)? {
            let entry = entry.map_err(provider_error)?;
            let path = entry.path();
            if path.file_name().is_some_and(|name| name == ".git") {
                continue;
            }
            ensure_not_link_or_reparse(&path)?;
            let metadata = fs::symlink_metadata(&path).map_err(provider_error)?;
            if metadata.is_dir() {
                collect(root, &path, bytes, files, directories)?;
            } else if metadata.is_file() {
                let locator = path
                    .strip_prefix(root)
                    .map_err(provider_error)?
                    .to_string_lossy()
                    .replace('/', "\\");
                let Some((_, expected_size)) = B1_E5_FILES
                    .iter()
                    .find(|(expected, _)| *expected == locator)
                else {
                    return Err(provider_error("unexpected model file"));
                };
                if metadata.len() != *expected_size {
                    return Err(provider_error("model file size mismatch"));
                }
                let mut file = open_verified_file(&path)?;
                let content = read_exact_allowlisted_file(&mut file, *expected_size)?;
                bytes.insert(locator, content);
                files.push(file);
            }
        }
        Ok(())
    }
    let mut bytes = BTreeMap::new();
    let mut files = Vec::new();
    let mut directories = Vec::new();
    collect(root, root, &mut bytes, &mut files, &mut directories)?;
    if bytes.len() != B1_E5_FILES.len() {
        return Err(provider_error("model file set is incomplete"));
    }
    let mut lines = Vec::new();
    for (locator, content) in &bytes {
        let hash = format!("{:X}", Sha256::digest(content));
        lines.push(format!("{locator}|{}|{hash}", content.len()));
    }
    let root_hash = format!(
        "{:X}",
        Sha256::digest(format!("{}\n", lines.join("\n")).as_bytes())
    );
    if root_hash != B1_E5_MANIFEST_ROOT {
        return Err(FastSearchError::new(
            ErrorKind::CapabilityUnavailable {
                capability: Capability::VectorRetrieval,
            },
            "B2_LOCAL_E5_MANIFEST_MISMATCH",
        ));
    }
    Ok(VerifiedSnapshot {
        bytes,
        manifest: root_hash,
        files,
        directories,
    })
}

fn read_exact_allowlisted_file(
    file: &mut File,
    expected_size: u64,
) -> Result<Vec<u8>, FastSearchError> {
    let opened = file.metadata().map_err(provider_error)?;
    if !opened.is_file() || opened.len() != expected_size {
        return Err(provider_error("opened model file size mismatch"));
    }
    let expected_size = usize::try_from(expected_size).map_err(provider_error)?;
    let mut content = Vec::new();
    content
        .try_reserve_exact(expected_size)
        .map_err(provider_error)?;
    content.resize(expected_size, 0);
    file.read_exact(&mut content).map_err(provider_error)?;
    let mut trailing = [0_u8; 1];
    if file.read(&mut trailing).map_err(provider_error)? != 0 {
        return Err(provider_error("model file grew while reading"));
    }
    Ok(content)
}

fn open_verified_file(path: &Path) -> Result<File, FastSearchError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    options
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    let file = options.open(path).map_err(provider_error)?;
    #[cfg(windows)]
    ensure_handle_is_not_reparse(file.as_raw_handle())?;
    Ok(file)
}

fn ensure_not_link_or_reparse(path: &Path) -> Result<(), FastSearchError> {
    let metadata = fs::symlink_metadata(path).map_err(provider_error)?;
    #[cfg(windows)]
    if metadata.file_attributes() & 0x400 != 0 {
        return Err(provider_error("model path contains a reparse point"));
    }
    #[cfg(not(windows))]
    if metadata.file_type().is_symlink() {
        return Err(provider_error("model path contains a symbolic link"));
    }
    Ok(())
}

#[cfg(windows)]
struct DirectoryGuard(HANDLE);

#[cfg(windows)]
impl Drop for DirectoryGuard {
    fn drop(&mut self) {
        // SAFETY: the handle is owned by this guard, validated on acquisition,
        // and closed exactly once here.
        unsafe { CloseHandle(self.0) };
    }
}

#[cfg(windows)]
fn open_directory_guard(path: &Path) -> Result<DirectoryGuard, FastSearchError> {
    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: `wide` is a live NUL-terminated UTF-16 path; the returned owned
    // handle is checked before it enters `DirectoryGuard`.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_LIST_DIRECTORY,
            FILE_SHARE_READ,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(provider_error(std::io::Error::last_os_error()));
    }
    if let Err(error) = ensure_handle_is_not_reparse(handle as RawHandle) {
        // SAFETY: `handle` was just acquired above and has not been transferred.
        unsafe { CloseHandle(handle) };
        return Err(error);
    }
    Ok(DirectoryGuard(handle))
}

#[cfg(windows)]
fn ensure_handle_is_not_reparse(handle: RawHandle) -> Result<(), FastSearchError> {
    let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
    // SAFETY: `handle` is borrowed from a live owned File/DirectoryGuard and
    // `info` is a correctly sized writable output buffer for this call.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            handle as HANDLE,
            FileAttributeTagInfo,
            (&raw mut info).cast(),
            u32::try_from(std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>())
                .expect("FILE_ATTRIBUTE_TAG_INFO size fits u32"),
        )
    };
    if ok == 0 {
        return Err(provider_error(std::io::Error::last_os_error()));
    }
    if info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(provider_error("model handle is a reparse point"));
    }
    Ok(())
}

#[cfg(not(windows))]
struct DirectoryGuard;

#[cfg(not(windows))]
fn open_directory_guard(_path: &Path) -> Result<DirectoryGuard, FastSearchError> {
    Ok(DirectoryGuard)
}

#[cfg(test)]
type VerifyLoadHook = Box<dyn FnOnce() + Send>;

#[cfg(test)]
static VERIFY_LOAD_HOOK: Mutex<Option<VerifyLoadHook>> = Mutex::new(None);

#[cfg(test)]
fn install_verify_load_hook(hook: impl FnOnce() + Send + 'static) {
    *VERIFY_LOAD_HOOK.lock().unwrap() = Some(Box::new(hook));
}

#[cfg(test)]
fn run_verify_load_hook() {
    if let Some(hook) = VERIFY_LOAD_HOOK.lock().unwrap().take() {
        hook();
    }
}

#[cfg(not(test))]
fn run_verify_load_hook() {}

fn provider_error(error: impl std::fmt::Display) -> FastSearchError {
    FastSearchError::new(
        ErrorKind::CapabilityUnavailable {
            capability: Capability::VectorRetrieval,
        },
        format!("B2_LOCAL_E5_PROVIDER_FAILED: {error}"),
    )
}

fn normalize(mut vector: Vec<f32>) -> Vec<f32> {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm.is_finite() && norm > 0.0 {
        for value in &mut vector {
            *value /= norm;
        }
    }
    vector
}

fn cosine(left: &[f32], right: &[f32]) -> f32 {
    left.iter().zip(right).map(|(a, b)| a * b).sum()
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
        install_verify_load_hook(move || {
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
        install_verify_load_hook(move || {
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

        let mut acquired = open_verified_file(&grown_path).unwrap();
        let error = read_exact_allowlisted_file(&mut acquired, expected).unwrap_err();
        assert!(error.message().contains("opened model file size mismatch"));
        assert_eq!(fs::metadata(&tokenizer).unwrap().len(), expected);
    }
}
