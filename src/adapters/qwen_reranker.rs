//! CPU admission adapter for the pinned Qwen3 reranker artifact.

use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

#[cfg(windows)]
use std::os::windows::{
    ffi::OsStrExt,
    fs::{MetadataExt, OpenOptionsExt},
    io::{AsRawHandle, RawHandle},
};

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::qwen3::{Config, ModelForCausalLM};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokenizers::Tokenizer;

#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_LIST_DIRECTORY,
        FILE_SHARE_READ, FileAttributeTagInfo, GetFileInformationByHandleEx, OPEN_EXISTING,
    },
};

pub const QWEN_REPOSITORY: &str = "Qwen/Qwen3-Reranker-0.6B";
pub const QWEN_REVISION: &str = "e61197ed45024b0ed8a2d74b80b4d909f1255473";
pub const QWEN_MAX_TOKENS: usize = 8192;
pub const QWEN_YES_TOKEN: u32 = 9693;
pub const QWEN_NO_TOKEN: u32 = 2152;
pub const QWEN_INSTRUCTION: &str =
    "Given a web search query, retrieve relevant passages that answer the query";

const PREFIX: &str = "<|im_start|>system\nJudge whether the Document meets the requirements based on the Query and the Instruct provided. Note that the answer can only be \"yes\" or \"no\".<|im_end|>\n<|im_start|>user\n";
const SUFFIX: &str = "<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n";
const OFFICIAL_MODEL_MANIFEST_SHA256: &str =
    "f782f5f6bcb12df743a4e1be9e4a80edeff0dfaa7ed4219cbab4533db3f3f637";
const OFFICIAL_ASSETS: &[(&str, u64, &str)] = &[
    (
        "config.json",
        727,
        "d479c427a9ca5295218063d4f9aca4f297ab4ac27487cca7af42c84643d51ef0",
    ),
    (
        "tokenizer.json",
        11_422_654,
        "aeb13307a71acd8fe81861d94ad54ab689df773318809eed3cbe794b4492dae4",
    ),
    (
        "model.safetensors",
        1_191_588_280,
        "27cd75a405b9c1b46b59abfd88aaa209e6fed2a1972cde9b70e7659537c5e65b",
    ),
];

type QwenError = Box<dyn std::error::Error + Send + Sync>;

pub struct QwenReranker {
    tokenizer: Tokenizer,
    model: ModelForCausalLM,
    device: Device,
    prefix_tokens: Vec<u32>,
    suffix_tokens: Vec<u32>,
    manifest_sha256: String,
    _pinned_artifacts: PinnedQwenArtifacts,
}

impl QwenReranker {
    /// Admits the exact official manifest and pins every mapped artifact for
    /// the complete lifetime of the model.
    pub fn open(root: &Path, manifest_path: &Path) -> Result<Self, QwenError> {
        let (descriptors, manifest_sha256) = exact_manifest(manifest_path)?;
        let pinned = PinnedQwenArtifacts::open(root, &descriptors)?;
        let device = Device::Cpu;
        let config: Config = serde_json::from_slice(&pinned.config)?;
        let tokenizer = Tokenizer::from_bytes(&pinned.tokenizer)?;
        let prefix_tokens = tokenizer.encode(PREFIX, false)?.get_ids().to_vec();
        let suffix_tokens = tokenizer.encode(SUFFIX, false)?.get_ids().to_vec();
        let yes = tokenizer.encode("yes", false)?.get_ids().to_vec();
        let no = tokenizer.encode("no", false)?.get_ids().to_vec();
        if yes != [QWEN_YES_TOKEN] || no != [QWEN_NO_TOKEN] {
            return Err(format!("unexpected yes/no tokens: yes={yes:?}, no={no:?}").into());
        }
        // SAFETY: PinnedQwenArtifacts denies mutation, deletion and replacement
        // of the already verified weights until QwenReranker is dropped.
        let weights = unsafe {
            VarBuilder::from_mmaped_safetensors(
                std::slice::from_ref(&pinned.weights_path),
                DType::F32,
                &device,
            )?
        };
        let model = ModelForCausalLM::new(&config, weights)?;
        Ok(Self {
            tokenizer,
            model,
            device,
            prefix_tokens,
            suffix_tokens,
            manifest_sha256,
            _pinned_artifacts: pinned,
        })
    }

    #[must_use]
    pub fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }

    pub fn score(&mut self, query: &str, document: &str) -> Result<f32, QwenError> {
        let body =
            format!("<Instruct>: {QWEN_INSTRUCTION}\n<Query>: {query}\n<Document>: {document}");
        let mut body_tokens = self.tokenizer.encode(body, false)?.get_ids().to_vec();
        let body_limit = QWEN_MAX_TOKENS
            .checked_sub(self.prefix_tokens.len() + self.suffix_tokens.len())
            .ok_or("Qwen fixed prompt exceeds maximum length")?;
        body_tokens.truncate(body_limit);

        let mut tokens = Vec::with_capacity(
            self.prefix_tokens.len() + body_tokens.len() + self.suffix_tokens.len(),
        );
        tokens.extend_from_slice(&self.prefix_tokens);
        tokens.extend_from_slice(&body_tokens);
        tokens.extend_from_slice(&self.suffix_tokens);

        self.model.clear_kv_cache();
        let input = Tensor::new(tokens.as_slice(), &self.device)?.unsqueeze(0)?;
        let logits = self
            .model
            .forward(&input, 0)?
            .squeeze(0)?
            .squeeze(0)?
            .to_dtype(DType::F32)?
            .to_vec1::<f32>()?;
        let yes = logits[QWEN_YES_TOKEN as usize];
        let no = logits[QWEN_NO_TOKEN as usize];
        if !yes.is_finite() || !no.is_finite() {
            return Err("Qwen produced non-finite yes/no logits".into());
        }
        let shift = yes.max(no);
        let yes_exp = (yes - shift).exp();
        Ok(yes_exp / (yes_exp + (no - shift).exp()))
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct ArtifactDescriptor {
    path: String,
    bytes: u64,
    sha256: String,
}

#[derive(Deserialize)]
struct ModelManifest {
    models: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    slug: String,
    repository: String,
    revision: String,
    assets: Vec<ArtifactDescriptor>,
}

fn exact_manifest(path: &Path) -> Result<(Vec<ArtifactDescriptor>, String), QwenError> {
    let bytes = fs::read(path)?;
    let manifest_sha256 = format!("{:x}", Sha256::digest(&bytes));
    if manifest_sha256 != OFFICIAL_MODEL_MANIFEST_SHA256 {
        return Err("model manifest does not match the official admission".into());
    }
    let manifest: ModelManifest = serde_json::from_slice(&bytes)?;
    let model = manifest
        .models
        .into_iter()
        .find(|model| model.slug == "qwen3-reranker-0.6b")
        .ok_or("official Qwen entry is missing from model manifest")?;
    if model.repository != QWEN_REPOSITORY || model.revision != QWEN_REVISION {
        return Err("Qwen repository or revision does not match the official admission".into());
    }
    let expected = OFFICIAL_ASSETS
        .iter()
        .map(|(path, bytes, sha256)| ArtifactDescriptor {
            path: (*path).to_owned(),
            bytes: *bytes,
            sha256: (*sha256).to_owned(),
        })
        .collect::<Vec<_>>();
    if model.assets != expected {
        return Err("Qwen artifact manifest does not match the official admission".into());
    }
    Ok((expected, manifest_sha256))
}

struct PinnedQwenArtifacts {
    config: Vec<u8>,
    tokenizer: Vec<u8>,
    weights_path: PathBuf,
    _files: Vec<File>,
    _root: DirectoryGuard,
}

impl PinnedQwenArtifacts {
    fn open(root: &Path, descriptors: &[ArtifactDescriptor]) -> Result<Self, QwenError> {
        if descriptors.len() != OFFICIAL_ASSETS.len() {
            return Err("Qwen artifact set is incomplete".into());
        }
        ensure_not_link_or_reparse(root)?;
        let directory = open_directory_guard(root)?;
        let mut files = Vec::with_capacity(descriptors.len());
        let mut config = None;
        let mut tokenizer = None;
        let mut weights_path = None;
        for descriptor in descriptors {
            let path = root.join(&descriptor.path);
            ensure_not_link_or_reparse(&path)?;
            let mut file = open_pinned_file(&path)?;
            let content = verify_opened_artifact(&mut file, descriptor)?;
            match descriptor.path.as_str() {
                "config.json" => config = content,
                "tokenizer.json" => tokenizer = content,
                "model.safetensors" => weights_path = Some(path),
                _ => return Err("unexpected Qwen artifact".into()),
            }
            files.push(file);
        }
        Ok(Self {
            config: config.ok_or("Qwen config is missing")?,
            tokenizer: tokenizer.ok_or("Qwen tokenizer is missing")?,
            weights_path: weights_path.ok_or("Qwen weights are missing")?,
            _files: files,
            _root: directory,
        })
    }
}

fn verify_opened_artifact(
    file: &mut File,
    descriptor: &ArtifactDescriptor,
) -> Result<Option<Vec<u8>>, QwenError> {
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() != descriptor.bytes {
        return Err(format!("Qwen {} size mismatch", descriptor.path).into());
    }
    let capture = descriptor.path != "model.safetensors";
    let mut content = capture.then(|| Vec::with_capacity(descriptor.bytes as usize));
    let mut hasher = Sha256::new();
    let mut read_bytes = 0_u64;
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        read_bytes += u64::try_from(read)?;
        hasher.update(&buffer[..read]);
        if let Some(content) = &mut content {
            content.extend_from_slice(&buffer[..read]);
        }
    }
    if read_bytes != descriptor.bytes || format!("{:x}", hasher.finalize()) != descriptor.sha256 {
        return Err(format!("Qwen {} SHA-256 mismatch", descriptor.path).into());
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(content)
}

#[cfg(windows)]
fn open_pinned_file(path: &Path) -> Result<File, QwenError> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    let file = options.open(path)?;
    ensure_handle_is_not_reparse(file.as_raw_handle())?;
    Ok(file)
}

#[cfg(not(windows))]
fn open_pinned_file(_path: &Path) -> Result<File, QwenError> {
    Err("Qwen immutable artifact admission is available only on Windows".into())
}

fn ensure_not_link_or_reparse(path: &Path) -> Result<(), QwenError> {
    let metadata = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    if metadata.file_attributes() & 0x400 != 0 {
        return Err("Qwen artifact path contains a reparse point".into());
    }
    #[cfg(not(windows))]
    if metadata.file_type().is_symlink() {
        return Err("Qwen artifact path contains a symbolic link".into());
    }
    Ok(())
}

#[cfg(windows)]
struct DirectoryGuard(HANDLE);

#[cfg(windows)]
unsafe impl Send for DirectoryGuard {}

#[cfg(windows)]
impl Drop for DirectoryGuard {
    fn drop(&mut self) {
        // SAFETY: this guard owns the validated handle and closes it once.
        unsafe { CloseHandle(self.0) };
    }
}

#[cfg(windows)]
fn open_directory_guard(path: &Path) -> Result<DirectoryGuard, QwenError> {
    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: wide is a live NUL-terminated UTF-16 path.
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
        return Err(std::io::Error::last_os_error().into());
    }
    if let Err(error) = ensure_handle_is_not_reparse(handle as RawHandle) {
        // SAFETY: handle is owned here and has not been transferred.
        unsafe { CloseHandle(handle) };
        return Err(error);
    }
    Ok(DirectoryGuard(handle))
}

#[cfg(not(windows))]
struct DirectoryGuard;

#[cfg(not(windows))]
fn open_directory_guard(_path: &Path) -> Result<DirectoryGuard, QwenError> {
    Err("Qwen immutable artifact admission is available only on Windows".into())
}

#[cfg(windows)]
fn ensure_handle_is_not_reparse(handle: RawHandle) -> Result<(), QwenError> {
    let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
    // SAFETY: handle is borrowed from a live owned handle and info is writable.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            handle as HANDLE,
            FileAttributeTagInfo,
            (&raw mut info).cast(),
            u32::try_from(std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>())?,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    if info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err("Qwen artifact handle is a reparse point".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        sync::atomic::{AtomicU64, Ordering},
        time::SystemTime,
    };

    use super::*;

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TempTree(PathBuf);

    impl TempTree {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "fastsearch-qwen-{label}-{}-{unique}-{}",
                std::process::id(),
                TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn descriptor(path: &str, bytes: &[u8]) -> ArtifactDescriptor {
        ArtifactDescriptor {
            path: path.to_owned(),
            bytes: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(bytes)),
        }
    }

    fn tiny_fixture() -> (TempTree, Vec<ArtifactDescriptor>) {
        let root = TempTree::new("pinning");
        let artifacts = [
            ("config.json", b"config".as_slice()),
            ("tokenizer.json", b"tokenizer".as_slice()),
            ("model.safetensors", b"weights".as_slice()),
        ];
        for (name, bytes) in artifacts {
            fs::write(root.0.join(name), bytes).unwrap();
        }
        let descriptors = artifacts
            .into_iter()
            .map(|(name, bytes)| descriptor(name, bytes))
            .collect();
        (root, descriptors)
    }

    #[test]
    fn official_manifest_entry_matches_compiled_admission() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("evidence/dt4/fixtures/ts-dt4-01/model-manifest.json");
        let (descriptors, _) = exact_manifest(&path).unwrap();
        assert_eq!(descriptors.len(), 3);
    }

    #[test]
    fn altered_manifest_and_substituted_root_fail_closed() {
        let fixture = TempTree::new("manifest-mismatch");
        let official = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("evidence/dt4/fixtures/ts-dt4-01/model-manifest.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&official).unwrap()).unwrap();
        value["models"][3]["assets"][2]["sha256"] = serde_json::Value::String("00".repeat(32));
        let changed_manifest = fixture.0.join("model-manifest.json");
        fs::write(&changed_manifest, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(exact_manifest(&changed_manifest).is_err());

        fs::write(fixture.0.join("config.json"), vec![b'x'; 727]).unwrap();
        let error = QwenReranker::open(&fixture.0, &official)
            .err()
            .expect("substituted root must fail");
        assert!(error.to_string().contains("SHA-256 mismatch"));
    }

    #[cfg(windows)]
    #[test]
    fn active_provider_denies_mutation_and_replacement_then_releases_handles() {
        let (fixture, descriptors) = tiny_fixture();
        let provider = PinnedQwenArtifacts::open(&fixture.0, &descriptors).unwrap();
        let config = fixture.0.join("config.json");
        let weights = fixture.0.join("model.safetensors");
        let replacement = fixture.0.join("replacement.safetensors");
        assert!(fs::write(&config, b"mutation").is_err());
        assert!(fs::rename(&weights, &replacement).is_err());
        drop(provider);
        fs::write(&config, b"mutation").unwrap();
        fs::rename(&weights, &replacement).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn failed_admission_releases_every_partially_acquired_handle() {
        let (fixture, mut descriptors) = tiny_fixture();
        descriptors[1].sha256 = "00".repeat(32);
        assert!(PinnedQwenArtifacts::open(&fixture.0, &descriptors).is_err());
        fs::write(fixture.0.join("config.json"), b"released").unwrap();
        fs::rename(
            fixture.0.join("model.safetensors"),
            fixture.0.join("released.safetensors"),
        )
        .unwrap();
    }
}
