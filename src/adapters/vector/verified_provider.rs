//! Immutable, verified local E5 provider boundary.
//!
//! This module owns filesystem admission, manifest verification, pinned handles,
//! and fastembed invocation. It has no access to projection state or lifecycle locks.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Read,
    path::Path,
};

#[cfg(test)]
use std::sync::Mutex;

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

use candle_core::{DType, Device};
use fastembed::{
    EmbeddingModel, InitOptionsUserDefined, NomicV2MoeTextEmbedding, Pooling, Qwen3TextEmbedding,
    TextEmbedding, TextInitOptions, TokenizerFiles, UserDefinedEmbeddingModel,
};
use sha2::{Digest, Sha256};

use crate::application::model_descriptor;
use crate::domain::{
    CanonicalRecord, Capability, EmbeddingModelId, ErrorKind, ExecutionDevice, FastSearchError,
};

enum ProviderRuntime {
    Onnx(TextEmbedding),
    Qwen(Qwen3TextEmbedding),
    Nomic(NomicV2MoeTextEmbedding),
}

// CPU real-corpus benchmark on 16.08.2026: batch 1 reached 40.78 docs/s.
// DirectML on GTX 1050 Ti peaked at batch 32; batch 1 left preprocessing and
// dispatch overhead CPU-bound. Durable checkpoints remain independently
// bounded to eight records.
const CPU_ONNX_BATCH_SIZE: usize = 1;
const GPU_ONNX_BATCH_SIZE: usize = 32;
pub(super) const DURABLE_CHECKPOINT_CHUNK_SIZE: usize = 8;
type CheckpointCallback<'a> = dyn FnMut(usize, &[Vec<f32>]) -> Result<(), FastSearchError> + 'a;

pub(super) struct VerifiedProvider {
    model_id: EmbeddingModelId,
    device: ExecutionDevice,
    runtime: ProviderRuntime,
    pub(super) manifest: String,
    _files: Vec<File>,
    _directories: Vec<DirectoryGuard>,
}

impl VerifiedProvider {
    pub(super) fn acquire(
        root: &Path,
        model_id: EmbeddingModelId,
        show_download_progress: bool,
        allow_catalog_download: bool,
    ) -> Result<Self, FastSearchError> {
        Self::acquire_on_device(
            root,
            model_id,
            show_download_progress,
            allow_catalog_download,
            ExecutionDevice::Cpu,
        )
    }

    pub(super) fn acquire_on_device(
        root: &Path,
        model_id: EmbeddingModelId,
        show_download_progress: bool,
        allow_catalog_download: bool,
        device: ExecutionDevice,
    ) -> Result<Self, FastSearchError> {
        if allow_catalog_download
            && (model_id != EmbeddingModelId::MultilingualE5Small
                || !root.join("onnx").join("model.onnx").is_file())
        {
            return Self::from_catalog(root, model_id, show_download_progress, device);
        }
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
        let options = user_defined_options(device)?;
        let runtime =
            TextEmbedding::try_new_from_user_defined(model, options).map_err(provider_error)?;
        Ok(Self {
            model_id,
            device,
            runtime: ProviderRuntime::Onnx(runtime),
            manifest: snapshot.manifest,
            _files: snapshot.files,
            _directories: snapshot.directories,
        })
    }

    fn from_catalog(
        root: &Path,
        model_id: EmbeddingModelId,
        show_download_progress: bool,
        device: ExecutionDevice,
    ) -> Result<Self, FastSearchError> {
        let runtime = match model_id {
            EmbeddingModelId::MultilingualE5Small
            | EmbeddingModelId::MultilingualE5Base
            | EmbeddingModelId::MultilingualE5Large => {
                let model = match model_id {
                    EmbeddingModelId::MultilingualE5Small => EmbeddingModel::MultilingualE5Small,
                    EmbeddingModelId::MultilingualE5Base => EmbeddingModel::MultilingualE5Base,
                    EmbeddingModelId::MultilingualE5Large => EmbeddingModel::MultilingualE5Large,
                    _ => unreachable!("E5 branch is exhaustive"),
                };
                let mut options = TextInitOptions::new(model)
                    .with_cache_dir(root.to_path_buf())
                    .with_show_download_progress(show_download_progress);
                if device == ExecutionDevice::GpuDirectMl {
                    options = options.with_execution_providers(directml_provider()?);
                }
                ProviderRuntime::Onnx(TextEmbedding::try_new(options).map_err(provider_error)?)
            }
            EmbeddingModelId::Qwen3Embedding06B if device == ExecutionDevice::Cpu => {
                ProviderRuntime::Qwen(
                    Qwen3TextEmbedding::from_hf(
                        "Qwen/Qwen3-Embedding-0.6B",
                        &Device::Cpu,
                        DType::F32,
                        512,
                    )
                    .map_err(provider_error)?,
                )
            }
            EmbeddingModelId::NomicEmbedTextV2Moe if device == ExecutionDevice::Cpu => {
                ProviderRuntime::Nomic(
                    NomicV2MoeTextEmbedding::from_hf(
                        "nomic-ai/nomic-embed-text-v2-moe",
                        &Device::Cpu,
                        DType::F32,
                        512,
                    )
                    .map_err(provider_error)?,
                )
            }
            EmbeddingModelId::Qwen3Embedding06B | EmbeddingModelId::NomicEmbedTextV2Moe => {
                return Err(provider_error(
                    "this Candle model has no GPU backend in the current FastSearch build",
                ));
            }
            EmbeddingModelId::SnowflakeArcticEmbedLV2
            | EmbeddingModelId::GteMultilingualBase
            | EmbeddingModelId::BgeM3 => ProviderRuntime::Onnx(user_defined_catalog_onnx(
                root,
                model_id,
                device,
                Pooling::Cls,
            )?),
            EmbeddingModelId::JinaEmbeddingsV3 => {
                return Err(provider_error(
                    "Jina Embeddings v3 requires task_id-aware ONNX inference; the dedicated provider is not bundled yet",
                ));
            }
        };
        let manifest = format!(
            "{:X}",
            Sha256::digest(format!("fastsearch-model-runtime-v1\0{}", model_id.slug()).as_bytes())
        );
        Ok(Self {
            model_id,
            device,
            runtime,
            manifest,
            _files: Vec::new(),
            _directories: Vec::new(),
        })
    }

    pub(super) fn embed_records_from_with_progress(
        &mut self,
        records: &[CanonicalRecord],
        completed_records: usize,
        progress: &mut dyn FnMut(usize, usize),
        checkpoint: &mut CheckpointCallback<'_>,
    ) -> Result<Vec<Vec<f32>>, FastSearchError> {
        if completed_records > records.len() {
            return Err(provider_error(
                "vector checkpoint exceeds the current record count",
            ));
        }
        let total = records.len();
        progress(completed_records, total);
        let mut vectors = Vec::with_capacity(total - completed_records);
        let inference_chunk_size = inference_chunk_size(self.device);
        for chunk in records[completed_records..].chunks(inference_chunk_size) {
            let embedded = self.embed_records(chunk)?;
            for durable_chunk in embedded.chunks(DURABLE_CHECKPOINT_CHUNK_SIZE) {
                let completed_before = completed_records + vectors.len();
                checkpoint(completed_before, durable_chunk)?;
                vectors.extend(durable_chunk.iter().cloned());
                progress(completed_records + vectors.len(), total);
            }
        }
        Ok(vectors)
    }

    pub(super) fn embed_records(
        &mut self,
        records: &[CanonicalRecord],
    ) -> Result<Vec<Vec<f32>>, FastSearchError> {
        let texts = records
            .iter()
            .map(|record| {
                let text = format!("{}\n{}", record.title(), record.searchable_content());
                match self.model_id {
                    EmbeddingModelId::MultilingualE5Small
                    | EmbeddingModelId::MultilingualE5Base
                    | EmbeddingModelId::MultilingualE5Large => format!("passage: {text}"),
                    EmbeddingModelId::Qwen3Embedding06B => text,
                    EmbeddingModelId::NomicEmbedTextV2Moe => {
                        format!("search_document: {text}")
                    }
                    EmbeddingModelId::SnowflakeArcticEmbedLV2
                    | EmbeddingModelId::GteMultilingualBase
                    | EmbeddingModelId::BgeM3
                    | EmbeddingModelId::JinaEmbeddingsV3 => text,
                }
            })
            .collect::<Vec<_>>();
        self.embed_formatted(&texts, None)
    }

    pub(super) fn embed_query(&mut self, query: &str) -> Result<Vec<f32>, FastSearchError> {
        let query = match self.model_id {
            EmbeddingModelId::MultilingualE5Small
            | EmbeddingModelId::MultilingualE5Base
            | EmbeddingModelId::MultilingualE5Large => format!("query: {query}"),
            EmbeddingModelId::Qwen3Embedding06B => format!(
                "Instruct: Given a search query, retrieve relevant documentation and source-code passages\nQuery: {query}"
            ),
            EmbeddingModelId::NomicEmbedTextV2Moe => format!("search_query: {query}"),
            EmbeddingModelId::SnowflakeArcticEmbedLV2
            | EmbeddingModelId::GteMultilingualBase
            | EmbeddingModelId::BgeM3
            | EmbeddingModelId::JinaEmbeddingsV3 => query.to_owned(),
        };
        self.embed_formatted(&[query], None)?
            .into_iter()
            .next()
            .ok_or_else(|| provider_error("embedding model returned no query vector"))
    }

    pub(super) fn embed_benchmark_texts(
        &mut self,
        texts: &[String],
        batch_size: usize,
    ) -> Result<Vec<Vec<f32>>, FastSearchError> {
        let formatted = texts
            .iter()
            .map(|text| match self.model_id {
                EmbeddingModelId::MultilingualE5Small
                | EmbeddingModelId::MultilingualE5Base
                | EmbeddingModelId::MultilingualE5Large => format!("passage: {text}"),
                EmbeddingModelId::Qwen3Embedding06B => text.clone(),
                EmbeddingModelId::NomicEmbedTextV2Moe => format!("search_document: {text}"),
                EmbeddingModelId::SnowflakeArcticEmbedLV2
                | EmbeddingModelId::GteMultilingualBase
                | EmbeddingModelId::BgeM3
                | EmbeddingModelId::JinaEmbeddingsV3 => text.clone(),
            })
            .collect::<Vec<_>>();
        self.embed_formatted(&formatted, Some(batch_size))
    }

    fn embed_formatted(
        &mut self,
        texts: &[String],
        onnx_batch_size: Option<usize>,
    ) -> Result<Vec<Vec<f32>>, FastSearchError> {
        run_verify_load_hook();
        let vectors = match &mut self.runtime {
            ProviderRuntime::Onnx(runtime) => runtime
                .embed(
                    texts,
                    Some(
                        onnx_batch_size
                            .unwrap_or_else(|| onnx_index_batch_size(self.model_id, self.device)),
                    ),
                )
                .map_err(provider_error)?,
            ProviderRuntime::Qwen(runtime) => runtime.embed(texts).map_err(provider_error)?,
            ProviderRuntime::Nomic(runtime) => runtime.embed(texts).map_err(provider_error)?,
        };
        if vectors.len() != texts.len()
            || vectors
                .iter()
                .any(|vector| vector.is_empty() || vector.iter().any(|value| !value.is_finite()))
        {
            return Err(FastSearchError::new(
                ErrorKind::ProjectionFailure,
                "local embedding model returned an invalid vector",
            ));
        }
        Ok(vectors.into_iter().map(normalize).collect())
    }
}

fn user_defined_catalog_onnx(
    root: &Path,
    model_id: EmbeddingModelId,
    device: ExecutionDevice,
    pooling: Pooling,
) -> Result<TextEmbedding, FastSearchError> {
    let repository = hf_hub::Cache::new(root.to_path_buf())
        .model(model_descriptor(model_id).repository.to_owned());
    let model_path = repository
        .get("onnx/model.onnx")
        .ok_or_else(|| provider_error("catalog model is missing onnx/model.onnx"))?;
    let tokenizer = |name: &str| {
        let path = repository
            .get(name)
            .ok_or_else(|| provider_error(format!("catalog model is missing {name}")))?;
        fs::read(path).map_err(provider_error)
    };
    let mut model = UserDefinedEmbeddingModel::new(
        fs::read(model_path).map_err(provider_error)?,
        TokenizerFiles {
            tokenizer_file: tokenizer("tokenizer.json")?,
            config_file: tokenizer("config.json")?,
            special_tokens_map_file: tokenizer("special_tokens_map.json")?,
            tokenizer_config_file: tokenizer("tokenizer_config.json")?,
        },
    )
    .with_pooling(pooling);
    if let Some(external_path) = repository.get("onnx/model.onnx_data") {
        model = model.with_external_initializer(
            "model.onnx_data".to_owned(),
            fs::read(external_path).map_err(provider_error)?,
        );
    }
    TextEmbedding::try_new_from_user_defined(model, user_defined_options(device)?)
        .map_err(provider_error)
}

const fn onnx_index_batch_size(model: EmbeddingModelId, device: ExecutionDevice) -> usize {
    match (model, device) {
        // The large external-data ONNX graphs already reserve several GiB.
        // Conservative GPU batches keep DirectML viable on 4 GiB adapters.
        (
            EmbeddingModelId::SnowflakeArcticEmbedLV2 | EmbeddingModelId::BgeM3,
            ExecutionDevice::GpuDirectMl,
        ) => 4,
        (EmbeddingModelId::GteMultilingualBase, ExecutionDevice::GpuDirectMl) => 8,
        (
            EmbeddingModelId::SnowflakeArcticEmbedLV2
            | EmbeddingModelId::GteMultilingualBase
            | EmbeddingModelId::BgeM3,
            ExecutionDevice::Cpu,
        ) => DURABLE_CHECKPOINT_CHUNK_SIZE,
        (_, ExecutionDevice::Cpu) => CPU_ONNX_BATCH_SIZE,
        (_, ExecutionDevice::GpuDirectMl) => GPU_ONNX_BATCH_SIZE,
    }
}

const fn inference_chunk_size(device: ExecutionDevice) -> usize {
    match device {
        ExecutionDevice::Cpu => DURABLE_CHECKPOINT_CHUNK_SIZE,
        ExecutionDevice::GpuDirectMl => GPU_ONNX_BATCH_SIZE,
    }
}

fn user_defined_options(
    device: ExecutionDevice,
) -> Result<InitOptionsUserDefined, FastSearchError> {
    let options = InitOptionsUserDefined::default();
    if device == ExecutionDevice::GpuDirectMl {
        Ok(options.with_execution_providers(directml_provider()?))
    } else {
        Ok(options)
    }
}

#[cfg(windows)]
fn directml_provider() -> Result<Vec<fastembed::ExecutionProviderDispatch>, FastSearchError> {
    Ok(vec![
        ort::ep::DirectML::default().build().error_on_failure(),
    ])
}

#[cfg(not(windows))]
fn directml_provider() -> Result<Vec<fastembed::ExecutionProviderDispatch>, FastSearchError> {
    Err(provider_error("DirectML is available only on Windows"))
}

/// Qualified runtime subset of the pinned E5 revision. Every admitted byte is
/// covered by the canonical locator/size/SHA-256 manifest root.
const B1_E5_MANIFEST_ROOT: &str =
    "8FCC7E28D97B8DA292E14631A6B46E03DD0890A4DA2AE244BE62813BC8CE53A6";

const B1_E5_FILES: &[(&str, u64)] = &[
    ("onnx\\config.json", 653),
    ("onnx\\model.onnx", 470_268_510),
    ("onnx\\special_tokens_map.json", 167),
    ("onnx\\tokenizer.json", 17_082_730),
    ("onnx\\tokenizer_config.json", 443),
];

const B1_E5_DIRECTORIES: &[&str] = &["onnx"];

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

pub(super) fn read_exact_allowlisted_file(
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

pub(super) fn open_verified_file(path: &Path) -> Result<File, FastSearchError> {
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
pub(super) fn install_verify_load_hook(hook: impl FnOnce() + Send + 'static) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directml_uses_the_qualified_batch_while_cpu_keeps_its_low_memory_batch() {
        assert_eq!(
            onnx_index_batch_size(EmbeddingModelId::MultilingualE5Small, ExecutionDevice::Cpu),
            1
        );
        assert_eq!(inference_chunk_size(ExecutionDevice::Cpu), 8);
        assert_eq!(
            onnx_index_batch_size(
                EmbeddingModelId::MultilingualE5Small,
                ExecutionDevice::GpuDirectMl
            ),
            32
        );
        assert_eq!(
            onnx_index_batch_size(
                EmbeddingModelId::SnowflakeArcticEmbedLV2,
                ExecutionDevice::GpuDirectMl
            ),
            4
        );
        assert_eq!(inference_chunk_size(ExecutionDevice::GpuDirectMl), 32);
        assert_eq!(DURABLE_CHECKPOINT_CHUNK_SIZE, 8);
    }
}
