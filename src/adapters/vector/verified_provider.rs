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

use fastembed::{
    InitOptionsUserDefined, Pooling, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel,
};
use sha2::{Digest, Sha256};

use crate::domain::{CanonicalRecord, Capability, ErrorKind, FastSearchError};

pub(super) struct VerifiedProvider {
    model: UserDefinedEmbeddingModel,
    pub(super) manifest: String,
    _files: Vec<File>,
    _directories: Vec<DirectoryGuard>,
}

impl VerifiedProvider {
    pub(super) fn acquire(root: &Path) -> Result<Self, FastSearchError> {
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
        Ok(Self {
            model,
            manifest: snapshot.manifest,
            _files: snapshot.files,
            _directories: snapshot.directories,
        })
    }

    pub(super) fn embed_records(
        self,
        records: &[CanonicalRecord],
    ) -> Result<Vec<Vec<f32>>, FastSearchError> {
        let texts = records
            .iter()
            .map(|record| format!("{}\\n{}", record.title(), record.searchable_content()))
            .collect::<Vec<_>>();
        self.embed_texts(&texts)
    }

    pub(super) fn embed_texts(self, texts: &[String]) -> Result<Vec<Vec<f32>>, FastSearchError> {
        run_verify_load_hook();
        let mut runtime =
            TextEmbedding::try_new_from_user_defined(self.model, InitOptionsUserDefined::default())
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
}

/// B1 accepted complete E5 cache set/: canonical locator/bytes/full-SHA256 root.
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
