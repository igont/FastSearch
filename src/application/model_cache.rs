//! Automatic, machine-local provisioning for selectable embedding models.
//!
//! A workspace selects one model. Provisioning only downloads and opens that
//! model; it never starts indexing. FastEmbed/Hugging Face own resumable blob
//! transport, while the vector adapter owns the inference contract.

use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use fs2::FileExt;
use hf_hub::Cache;
use reqwest::{
    StatusCode,
    blocking::Client,
    header::{CONTENT_LENGTH, CONTENT_RANGE, RANGE},
};

use crate::{
    adapters::vector::{prepare_embedding_model, probe_embedding_model_device},
    domain::{
        DeviceCapabilityStatus, EmbeddingModelId, ErrorKind, ExecutionDevice, FastSearchError,
    },
};

use super::workspace::product_home;

pub const E5_REPOSITORY: &str = "intfloat/multilingual-e5-small";
pub const E5_REVISION: &str = "614241f622f53c4eeff9890bdc4f31cfecc418b3";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EmbeddingModelDescriptor {
    pub id: EmbeddingModelId,
    pub repository: &'static str,
    pub revision: &'static str,
    pub source_url: &'static str,
    pub profile: &'static str,
    pub approximate_download_bytes: u64,
}

pub const MODEL_CATALOG: [EmbeddingModelDescriptor; 5] = [
    EmbeddingModelDescriptor {
        id: EmbeddingModelId::MultilingualE5Small,
        repository: E5_REPOSITORY,
        revision: E5_REVISION,
        source_url: "https://huggingface.co/intfloat/multilingual-e5-small",
        profile: "быстрая · 384 измерения",
        approximate_download_bytes: 487_352_503,
    },
    EmbeddingModelDescriptor {
        id: EmbeddingModelId::MultilingualE5Base,
        repository: "intfloat/multilingual-e5-base",
        revision: "d128750597153bb5987e10b1c3493a34e5a4502a",
        source_url: "https://huggingface.co/intfloat/multilingual-e5-base",
        profile: "сбалансированная · 768 измерений",
        approximate_download_bytes: 1_127_143_128,
    },
    EmbeddingModelDescriptor {
        id: EmbeddingModelId::MultilingualE5Large,
        repository: "Qdrant/multilingual-e5-large-onnx",
        revision: "66076b8dc6e367337e3e90e6fb309fb0f3addaf6",
        source_url: "https://huggingface.co/Qdrant/multilingual-e5-large-onnx",
        profile: "качество · 1024 измерения",
        approximate_download_bytes: 2_253_012_762,
    },
    EmbeddingModelDescriptor {
        id: EmbeddingModelId::Qwen3Embedding06B,
        repository: "Qwen/Qwen3-Embedding-0.6B",
        revision: "97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3",
        source_url: "https://huggingface.co/Qwen/Qwen3-Embedding-0.6B",
        profile: "современная · русский и код · 1024 измерения",
        approximate_download_bytes: 1_203_010_121,
    },
    EmbeddingModelDescriptor {
        id: EmbeddingModelId::NomicEmbedTextV2Moe,
        repository: "nomic-ai/nomic-embed-text-v2-moe",
        revision: "1066b6599d099fbb93dfcb64f9c37a7c9e503e85",
        source_url: "https://huggingface.co/nomic-ai/nomic-embed-text-v2-moe",
        profile: "экспериментальная · 768 измерений",
        approximate_download_bytes: 1_918_272_448,
    },
];

#[derive(Clone, Copy)]
struct ModelAsset {
    path: &'static str,
    size: u64,
}

const E5_BASE_ASSETS: &[ModelAsset] = &[
    ModelAsset {
        path: "config.json",
        size: 694,
    },
    ModelAsset {
        path: "tokenizer.json",
        size: 17_082_660,
    },
    ModelAsset {
        path: "tokenizer_config.json",
        size: 418,
    },
    ModelAsset {
        path: "special_tokens_map.json",
        size: 280,
    },
    ModelAsset {
        path: "onnx/model.onnx",
        size: 1_110_059_084,
    },
];
const E5_SMALL_ASSETS: &[ModelAsset] = &[
    ModelAsset {
        path: "onnx/config.json",
        size: 653,
    },
    ModelAsset {
        path: "onnx/model.onnx",
        size: 470_268_510,
    },
    ModelAsset {
        path: "onnx/special_tokens_map.json",
        size: 167,
    },
    ModelAsset {
        path: "onnx/tokenizer.json",
        size: 17_082_730,
    },
    ModelAsset {
        path: "onnx/tokenizer_config.json",
        size: 443,
    },
];
const E5_LARGE_ASSETS: &[ModelAsset] = &[
    ModelAsset {
        path: "config.json",
        size: 716,
    },
    ModelAsset {
        path: "tokenizer.json",
        size: 17_082_756,
    },
    ModelAsset {
        path: "tokenizer_config.json",
        size: 1_147,
    },
    ModelAsset {
        path: "special_tokens_map.json",
        size: 964,
    },
    ModelAsset {
        path: "model.onnx",
        size: 545_851,
    },
    ModelAsset {
        path: "model.onnx_data",
        size: 2_235_363_328,
    },
];
const QWEN_ASSETS: &[ModelAsset] = &[
    ModelAsset {
        path: "config.json",
        size: 727,
    },
    ModelAsset {
        path: "tokenizer.json",
        size: 11_423_705,
    },
    ModelAsset {
        path: "model.safetensors",
        size: 1_191_586_416,
    },
];
const NOMIC_ASSETS: &[ModelAsset] = &[
    ModelAsset {
        path: "config.json",
        size: 2_482,
    },
    ModelAsset {
        path: "tokenizer.json",
        size: 17_082_734,
    },
    ModelAsset {
        path: "model.safetensors",
        size: 1_901_187_232,
    },
];

#[must_use]
pub fn model_descriptor(id: EmbeddingModelId) -> &'static EmbeddingModelDescriptor {
    MODEL_CATALOG
        .iter()
        .find(|descriptor| descriptor.id == id)
        .expect("the model catalog covers every stable model ID")
}

#[must_use]
pub fn model_identity(id: EmbeddingModelId) -> String {
    let descriptor = model_descriptor(id);
    format!("{}@{}", descriptor.repository, descriptor.revision)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmbeddingModelAvailability {
    model: EmbeddingModelId,
    root: PathBuf,
    downloaded: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelProvisionProgress {
    completed_bytes: u64,
    total_bytes: u64,
    asset: String,
}

impl ModelProvisionProgress {
    #[must_use]
    pub const fn completed_bytes(&self) -> u64 {
        self.completed_bytes
    }

    #[must_use]
    pub const fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    #[must_use]
    pub fn asset(&self) -> &str {
        &self.asset
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmbeddingModelCacheStatus {
    model: EmbeddingModelId,
    root: PathBuf,
    ready: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelRuntimeCapabilities {
    cpu: DeviceCapabilityStatus,
    gpu: DeviceCapabilityStatus,
    gpu_backend: Option<String>,
    gpu_detail: Option<String>,
}

impl ModelRuntimeCapabilities {
    #[must_use]
    pub const fn cpu(&self) -> DeviceCapabilityStatus {
        self.cpu
    }

    #[must_use]
    pub const fn gpu(&self) -> DeviceCapabilityStatus {
        self.gpu
    }

    #[must_use]
    pub fn gpu_backend(&self) -> Option<&str> {
        self.gpu_backend.as_deref()
    }

    #[must_use]
    pub fn gpu_detail(&self) -> Option<&str> {
        self.gpu_detail.as_deref()
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StoredRuntimeCapabilities {
    schema: u8,
    model_revision: String,
    cpu: String,
    gpu: String,
    gpu_backend: Option<String>,
    gpu_detail: Option<String>,
}

impl EmbeddingModelCacheStatus {
    #[must_use]
    pub const fn model(&self) -> EmbeddingModelId {
        self.model
    }
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
    #[must_use]
    pub const fn ready(&self) -> bool {
        self.ready
    }
}

impl EmbeddingModelAvailability {
    #[must_use]
    pub const fn model(&self) -> EmbeddingModelId {
        self.model
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub const fn downloaded(&self) -> bool {
        self.downloaded
    }
}

/// Downloads the selected model when absent and proves that its runtime opens.
/// No index or corpus is touched.
pub fn ensure_embedding_model(
    model: EmbeddingModelId,
    show_download_progress: bool,
) -> Result<EmbeddingModelAvailability, FastSearchError> {
    ensure_embedding_model_with_progress(model, show_download_progress, |_| {})
}

pub fn ensure_embedding_model_with_progress(
    model: EmbeddingModelId,
    show_download_progress: bool,
    mut progress: impl FnMut(ModelProvisionProgress),
) -> Result<EmbeddingModelAvailability, FastSearchError> {
    let (model_directory, root) = model_paths(model)?;
    fs::create_dir_all(&model_directory).map_err(model_error)?;
    fs::create_dir_all(&root).map_err(model_error)?;
    let marker = model_directory.join(".ready");
    let install_lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(model_directory.join(".install.lock"))
        .map_err(model_error)?;
    install_lock.lock_exclusive().map_err(model_error)?;
    let was_ready = marker.is_file();

    if was_ready && cached_model_artifacts_are_present(model, &root) {
        ensure_runtime_capability_probe(model, &model_directory, &root);
        return Ok(EmbeddingModelAvailability {
            model,
            root,
            downloaded: false,
        });
    }

    if was_ready {
        fs::remove_file(&marker).map_err(model_error)?;
    }

    provision_catalog_assets(model, &root, &mut progress)?;
    prepare_embedding_model(model, &root, show_download_progress)?;
    let descriptor = model_descriptor(model);
    let marker_text = format!(
        "model={}\nrepository={}\nrevision={}\ndimension={}\n",
        model.slug(),
        descriptor.repository,
        descriptor.revision,
        model.dimension(),
    );
    fs::write(&marker, marker_text).map_err(model_error)?;
    ensure_runtime_capability_probe(model, &model_directory, &root);

    Ok(EmbeddingModelAvailability {
        model,
        root,
        downloaded: !was_ready,
    })
}

/// Returns persisted machine-local execution capability evidence. A missing
/// probe is intentionally represented as unknown rather than guessed.
pub fn model_runtime_capabilities(
    model: EmbeddingModelId,
) -> Result<ModelRuntimeCapabilities, FastSearchError> {
    let (model_directory, _) = model_paths(model)?;
    let path = model_directory.join("runtime-capabilities.toml");
    if !path.is_file() {
        return Ok(ModelRuntimeCapabilities {
            cpu: DeviceCapabilityStatus::Ready,
            gpu: DeviceCapabilityStatus::Unknown,
            gpu_backend: None,
            gpu_detail: None,
        });
    }
    let source = fs::read_to_string(path).map_err(model_error)?;
    let stored: StoredRuntimeCapabilities = toml::from_str(&source).map_err(model_error)?;
    if stored.schema != 1 || stored.model_revision != model_descriptor(model).revision {
        return Ok(ModelRuntimeCapabilities {
            cpu: DeviceCapabilityStatus::Ready,
            gpu: DeviceCapabilityStatus::Unknown,
            gpu_backend: None,
            gpu_detail: Some("сохранённая проба относится к другой ревизии модели".to_owned()),
        });
    }
    Ok(ModelRuntimeCapabilities {
        cpu: parse_capability_status(&stored.cpu),
        gpu: parse_capability_status(&stored.gpu),
        gpu_backend: stored.gpu_backend,
        gpu_detail: stored.gpu_detail,
    })
}

fn ensure_runtime_capability_probe(model: EmbeddingModelId, model_directory: &Path, root: &Path) {
    if model_directory.join("runtime-capabilities.toml").is_file() {
        return;
    }
    let (gpu, detail) =
        match probe_embedding_model_device(model, root, ExecutionDevice::GpuDirectMl) {
            Ok(()) => (
                "ready",
                "DirectML probe produced a valid embedding".to_owned(),
            ),
            Err(error) => ("unavailable", error.message().replace(['\r', '\n'], " ")),
        };
    let stored = StoredRuntimeCapabilities {
        schema: 1,
        model_revision: model_descriptor(model).revision.to_owned(),
        cpu: "ready".to_owned(),
        gpu: gpu.to_owned(),
        gpu_backend: Some("DirectML".to_owned()),
        gpu_detail: Some(detail),
    };
    if let Ok(source) = toml::to_string_pretty(&stored) {
        let _ = fs::write(model_directory.join("runtime-capabilities.toml"), source);
    }
}

fn parse_capability_status(value: &str) -> DeviceCapabilityStatus {
    match value {
        "ready" => DeviceCapabilityStatus::Ready,
        "unavailable" => DeviceCapabilityStatus::Unavailable,
        _ => DeviceCapabilityStatus::Unknown,
    }
}

/// Read-only cache admission used by comparison readiness. It never downloads
/// weights and never creates a ready marker.
pub fn embedding_model_cache_status(
    model: EmbeddingModelId,
) -> Result<EmbeddingModelCacheStatus, FastSearchError> {
    let (model_directory, root) = model_paths(model)?;
    Ok(EmbeddingModelCacheStatus {
        model,
        ready: model_directory.join(".ready").is_file()
            && cached_model_artifacts_are_present(model, &root),
        root,
    })
}

fn provision_catalog_assets(
    model: EmbeddingModelId,
    root: &Path,
    progress: &mut dyn FnMut(ModelProvisionProgress),
) -> Result<(), FastSearchError> {
    let assets = match model {
        EmbeddingModelId::MultilingualE5Small => E5_SMALL_ASSETS,
        EmbeddingModelId::MultilingualE5Base => E5_BASE_ASSETS,
        EmbeddingModelId::MultilingualE5Large => E5_LARGE_ASSETS,
        EmbeddingModelId::Qwen3Embedding06B => QWEN_ASSETS,
        EmbeddingModelId::NomicEmbedTextV2Moe => NOMIC_ASSETS,
    };
    let descriptor = model_descriptor(model);
    let cache_root = match model {
        EmbeddingModelId::MultilingualE5Small => root.to_path_buf(),
        EmbeddingModelId::MultilingualE5Base | EmbeddingModelId::MultilingualE5Large => {
            root.to_path_buf()
        }
        EmbeddingModelId::Qwen3Embedding06B | EmbeddingModelId::NomicEmbedTextV2Moe => {
            Cache::from_env().path().clone()
        }
    };
    let repository_root = cache_root.join(format!(
        "models--{}",
        descriptor.repository.replace('/', "--")
    ));
    let snapshot = if model == EmbeddingModelId::MultilingualE5Small {
        root.to_path_buf()
    } else {
        repository_root.join("snapshots").join(descriptor.revision)
    };
    fs::create_dir_all(&snapshot).map_err(model_error)?;

    let client = Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::limited(10))
        .user_agent("FastSearch/0.1 model-provisioner")
        .build()
        .map_err(model_error)?;
    let total_bytes = assets.iter().map(|asset| asset.size).sum::<u64>();
    let mut completed_before = 0;
    for asset in assets {
        download_asset_with_resume(
            &client,
            descriptor,
            *asset,
            &snapshot.join(asset.path),
            completed_before,
            total_bytes,
            progress,
        )?;
        completed_before += asset.size;
    }

    if model != EmbeddingModelId::MultilingualE5Small {
        let refs = repository_root.join("refs");
        fs::create_dir_all(&refs).map_err(model_error)?;
        fs::write(refs.join("main"), descriptor.revision).map_err(model_error)?;
    }
    Ok(())
}

fn download_asset_with_resume(
    client: &Client,
    descriptor: &EmbeddingModelDescriptor,
    asset: ModelAsset,
    target: &Path,
    completed_before: u64,
    total_bytes: u64,
    progress: &mut dyn FnMut(ModelProvisionProgress),
) -> Result<(), FastSearchError> {
    if fs::metadata(target).is_ok_and(|metadata| metadata.len() == asset.size) {
        progress(ModelProvisionProgress {
            completed_bytes: completed_before + asset.size,
            total_bytes,
            asset: asset.path.to_owned(),
        });
        return Ok(());
    }
    if target.exists() {
        fs::remove_file(target).map_err(model_error)?;
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(model_error)?;
    }
    let partial = PathBuf::from(format!("{}.download", target.display()));
    if fs::metadata(&partial).is_ok_and(|metadata| metadata.len() > asset.size) {
        fs::remove_file(&partial).map_err(model_error)?;
    }
    let endpoint =
        std::env::var("HF_ENDPOINT").unwrap_or_else(|_| "https://huggingface.co".to_owned());
    let url = format!(
        "{}/{}/resolve/{}/{}",
        endpoint.trim_end_matches('/'),
        descriptor.repository,
        descriptor.revision,
        asset.path
    );
    let mut last_error = String::new();

    for attempt in 1..=24 {
        let offset = fs::metadata(&partial)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        let mut request = client.get(&url);
        if offset > 0 {
            request = request.header(RANGE, format!("bytes={offset}-"));
        }

        match request.send() {
            Ok(mut response) if response.status().is_success() => {
                let resumed = response.status() == StatusCode::PARTIAL_CONTENT && offset > 0;
                let starting_length = if resumed { offset } else { 0 };
                let expected_length = response
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.rsplit('/').next())
                    .and_then(|value| value.parse::<u64>().ok())
                    .or_else(|| {
                        response
                            .headers()
                            .get(CONTENT_LENGTH)
                            .and_then(|value| value.to_str().ok())
                            .and_then(|value| value.parse::<u64>().ok())
                            .map(|length| starting_length + length)
                    });
                let mut output = OpenOptions::new()
                    .create(true)
                    .write(true)
                    .append(resumed)
                    .truncate(!resumed)
                    .open(&partial)
                    .map_err(model_error)?;
                let copy_result = copy_with_progress(
                    &mut response,
                    &mut output,
                    starting_length,
                    asset,
                    completed_before,
                    total_bytes,
                    progress,
                )
                .and_then(|_| output.flush());
                match copy_result {
                    Ok(()) => {
                        let actual_length = fs::metadata(&partial).map_err(model_error)?.len();
                        if expected_length.is_none_or(|expected| expected == actual_length)
                            && actual_length == asset.size
                        {
                            fs::rename(&partial, target).map_err(model_error)?;
                            return Ok(());
                        }
                        last_error = format!(
                            "incomplete response for {}: {actual_length} of {expected_length:?} bytes",
                            asset.path
                        );
                    }
                    Err(error) => {
                        last_error = format!("stream for {} stalled: {error}", asset.path)
                    }
                }
            }
            Ok(response) => {
                last_error = format!(
                    "HTTP {} while downloading {}",
                    response.status(),
                    asset.path
                );
            }
            Err(error) => last_error = format!("request for {} failed: {error}", asset.path),
        }
        if attempt < 24 {
            thread::sleep(Duration::from_secs(attempt.min(5)));
        }
    }

    Err(model_error(format!(
        "could not recover {} after 24 resumable attempts: {last_error}",
        descriptor.id.display_name()
    )))
}

fn copy_with_progress(
    input: &mut impl Read,
    output: &mut impl Write,
    starting_length: u64,
    asset: ModelAsset,
    completed_before: u64,
    total_bytes: u64,
    progress: &mut dyn FnMut(ModelProvisionProgress),
) -> io::Result<()> {
    let mut buffer = [0_u8; 1024 * 1024];
    let mut asset_completed = starting_length;
    let mut last_reported = starting_length;
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read])?;
        asset_completed += read as u64;
        if asset_completed.saturating_sub(last_reported) >= 4 * 1024 * 1024
            || asset_completed == asset.size
        {
            progress(ModelProvisionProgress {
                completed_bytes: completed_before + asset_completed.min(asset.size),
                total_bytes,
                asset: asset.path.to_owned(),
            });
            last_reported = asset_completed;
        }
    }
    Ok(())
}

fn cached_model_artifacts_are_present(model: EmbeddingModelId, root: &Path) -> bool {
    let substantial_file_exists = |path: Option<PathBuf>| {
        path.and_then(|path| fs::metadata(path).ok())
            .is_some_and(|metadata| metadata.len() > 100_000_000)
    };

    match model {
        EmbeddingModelId::MultilingualE5Small => {
            substantial_file_exists(Some(root.join("onnx").join("model.onnx")))
        }
        EmbeddingModelId::MultilingualE5Base => substantial_file_exists(
            Cache::new(root.to_path_buf())
                .model(model_descriptor(model).repository.to_owned())
                .get("onnx/model.onnx"),
        ),
        EmbeddingModelId::MultilingualE5Large => substantial_file_exists(
            Cache::new(root.to_path_buf())
                .model(model_descriptor(model).repository.to_owned())
                .get("model.onnx_data"),
        ),
        EmbeddingModelId::Qwen3Embedding06B | EmbeddingModelId::NomicEmbedTextV2Moe => {
            substantial_file_exists(
                Cache::from_env()
                    .model(model_descriptor(model).repository.to_owned())
                    .get("model.safetensors"),
            )
        }
    }
}

fn model_paths(model: EmbeddingModelId) -> Result<(PathBuf, PathBuf), FastSearchError> {
    let model_directory = product_home()?.join("models").join(model.slug());
    let root = if model == EmbeddingModelId::MultilingualE5Small {
        model_directory.join(E5_REVISION)
    } else {
        model_directory.join("runtime")
    };
    Ok((model_directory, root))
}

/// Backward-compatible name for callers that still request the default model.
pub fn ensure_e5_model(
    show_download_progress: bool,
) -> Result<EmbeddingModelAvailability, FastSearchError> {
    ensure_embedding_model(
        EmbeddingModelId::MultilingualE5Small,
        show_download_progress,
    )
}

pub type E5ModelAvailability = EmbeddingModelAvailability;

fn model_error(error: impl std::fmt::Display) -> FastSearchError {
    FastSearchError::new(
        ErrorKind::CapabilityUnavailable {
            capability: crate::domain::Capability::VectorRetrieval,
        },
        format!("automatic embedding model provisioning failed: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_complete_unique_and_dimensioned() {
        assert_eq!(MODEL_CATALOG.len(), EmbeddingModelId::ALL.len());
        for model in EmbeddingModelId::ALL {
            let descriptor = model_descriptor(model);
            assert_eq!(descriptor.id, model);
            assert!(!descriptor.repository.is_empty());
            assert_eq!(descriptor.revision.len(), 40);
            assert!(descriptor.source_url.starts_with("https://huggingface.co/"));
            assert!(descriptor.approximate_download_bytes > 100_000_000);
            assert!(model.dimension() >= 384);
        }
    }
}
