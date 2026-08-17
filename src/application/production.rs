//! The single full production composition for semantic and code navigation.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use crate::{
    adapters::{
        lexical::TantivyLexical,
        maps::{CodeMapRelated, CodeMapSource},
        source::FilesystemSource,
        state::SqliteStateStore,
        symbols::SymbolSource,
        vector::{LocalE5Vector, VectorBuildProgress},
    },
    application::fusion::{ChannelCandidates, FusionCoordinator},
    domain::{
        BackendKind, CanonicalRecord, Capability, CapabilityStatus, EmbeddingModelId, ErrorKind,
        ExecutionDevice, FastSearchError, IndexFreshness, LifecycleStatus, LogicalRootId,
        RelatedQuery, RetrievalChannel, SearchHit, SearchQuery, SearchResponse, StableId,
    },
    ports::{
        AgentSurface, CodeMapPort, LexicalRetrieval, SourcePort, StateChange, StateStore,
        SymbolPort, VectorRetrieval,
    },
};

mod security {
    use std::{
        collections::BTreeMap,
        fs::{self, OpenOptions},
        io::Write,
        path::{Path, PathBuf},
        sync::{
            Mutex,
            atomic::{AtomicU64, Ordering},
        },
    };

    use crate::domain::{ErrorKind, FastSearchError};

    #[cfg(windows)]
    use std::{mem::size_of, os::windows::ffi::OsStrExt};

    #[cfg(windows)]
    use std::os::windows::fs::MetadataExt;

    #[cfg(windows)]
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            CreateFileW, DELETE, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO,
            FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_LIST_DIRECTORY, FILE_SHARE_READ, FILE_SHARE_WRITE, FileAttributeTagInfo,
            FileDispositionInfo, GetFileInformationByHandleEx, OPEN_EXISTING,
            SetFileInformationByHandle,
        },
    };

    static RUN_TOKEN_COUNTER: AtomicU64 = AtomicU64::new(1);

    /// Owns the service-root admission, pinned handles and exact run cleanup policy.
    pub(super) struct ServiceRunBoundary {
        service_root: PathBuf,
        owned_runs: Mutex<BTreeMap<String, OwnedRun>>,
        _path_guards: PathGuards,
    }

    impl ServiceRunBoundary {
        pub(super) fn admit_and_pin(
            document_root: &Path,
            code_root: &Path,
            requested_service_root: &Path,
        ) -> Result<Self, FastSearchError> {
            validate_service_path_before_write(document_root, code_root, requested_service_root)?;
            let (service_root, path_guards) =
                securely_create_and_pin_service(requested_service_root)?;
            validate_service_containment(document_root, code_root, &service_root)?;
            Ok(Self {
                service_root,
                owned_runs: Mutex::new(BTreeMap::new()),
                _path_guards: path_guards,
            })
        }

        pub(super) fn admit_workspace_and_pin(
            workspace_root: &Path,
            source_roots: &[PathBuf],
            requested_service_root: &Path,
        ) -> Result<Self, FastSearchError> {
            let workspace_root = workspace_root.canonicalize().map_err(|error| {
                failure(
                    ErrorKind::SourceFailure,
                    "canonicalize workspace root",
                    error,
                )
            })?;
            if !workspace_root.is_dir() {
                return Err(FastSearchError::new(
                    ErrorKind::InvalidContent,
                    "workspace root must be a directory",
                ));
            }
            for source in source_roots {
                if !source.starts_with(&workspace_root) {
                    return Err(FastSearchError::new(
                        ErrorKind::InvalidContent,
                        "workspace source root must be contained by workspace root",
                    ));
                }
            }
            let expected = workspace_root.join(".fastsearch").join("local");
            let requested = absolute_service_path(requested_service_root)?;
            ensure_no_reparse_points(&requested)?;
            let resolved_requested = canonicalize_existing_ancestor(&requested)?;
            if resolved_requested != expected {
                return Err(FastSearchError::new(
                    ErrorKind::InvalidContent,
                    "workspace service state must be exactly .fastsearch/local",
                ));
            }
            let (service_root, path_guards) = securely_create_and_pin_service(&requested)?;
            if service_root != expected {
                return Err(FastSearchError::new(
                    ErrorKind::InvalidContent,
                    "workspace service state escaped .fastsearch/local",
                ));
            }
            Ok(Self {
                service_root,
                owned_runs: Mutex::new(BTreeMap::new()),
                _path_guards: path_guards,
            })
        }

        pub(super) fn service_root(&self) -> &Path {
            &self.service_root
        }

        pub(super) fn record_run_marker(&self, marker: &str) -> Result<PathBuf, FastSearchError> {
            validate_marker(marker)?;
            let runs = self.service_root.join("runs");
            ensure_no_reparse_points(&runs)?;
            fs::create_dir_all(&runs).map_err(|error| {
                failure(ErrorKind::StateFailure, "create runs directory", error)
            })?;
            ensure_no_reparse_points(&runs)?;
            let run = runs.join(marker);
            fs::create_dir(&run)
                .map_err(|error| failure(ErrorKind::StateFailure, "create run directory", error))?;
            let guard = match RunDirectoryGuard::acquire(&run) {
                Ok(guard) => guard,
                Err(error) => {
                    let _ = fs::remove_dir(&run);
                    return Err(error);
                }
            };
            let token = format!(
                "{}-{}",
                std::process::id(),
                RUN_TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed)
            );
            if let Err(error) = write_new_run_markers(&run, &token) {
                let _ = fs::remove_file(run.join("schema.marker"));
                let _ = fs::remove_file(run.join("owner.marker"));
                let _ = guard.mark_for_delete(&run);
                drop(guard);
                return Err(error);
            }
            self.owned_runs
                .lock()
                .map_err(|_| {
                    FastSearchError::new(ErrorKind::StateFailure, "run ownership lock poisoned")
                })?
                .insert(marker.to_owned(), OwnedRun { token, guard });
            Ok(run)
        }

        pub(super) fn cleanup_run(&self, marker: &str) -> Result<bool, FastSearchError> {
            validate_marker(marker)?;
            let run = self.service_root.join("runs").join(marker);
            if !run.exists() {
                return Ok(false);
            }
            let marker_path = run.join("owner.marker");
            let mut owned = self.owned_runs.lock().map_err(|_| {
                FastSearchError::new(ErrorKind::StateFailure, "run ownership lock poisoned")
            })?;
            let owned_run = owned.get(marker).ok_or_else(|| {
                FastSearchError::new(
                    ErrorKind::StateFailure,
                    "run directory is not owned by this runtime invocation",
                )
            })?;
            let observed = fs::read_to_string(&marker_path)
                .map_err(|error| failure(ErrorKind::StateFailure, "read run marker", error))?;
            if observed != owned_run.token {
                return Err(FastSearchError::new(
                    ErrorKind::StateFailure,
                    "run cleanup marker does not match the exact requested owner",
                ));
            }
            ensure_no_reparse_points(&run)?;
            let schema_path = run.join("schema.marker");
            if fs::read_to_string(&schema_path)
                .map_err(|error| failure(ErrorKind::StateFailure, "read run schema", error))?
                != "fastsearch-run-v1"
            {
                return Err(FastSearchError::new(
                    ErrorKind::StateFailure,
                    "run cleanup schema does not match fastsearch-run-v1",
                ));
            }
            let entries = fs::read_dir(&run)
                .map_err(|error| {
                    failure(ErrorKind::StateFailure, "read exact run directory", error)
                })?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| failure(ErrorKind::StateFailure, "read exact run entry", error))?;
            if entries.iter().any(|entry| {
                !matches!(
                    entry.file_name().to_str(),
                    Some("owner.marker" | "schema.marker")
                )
            }) {
                return Err(FastSearchError::new(
                    ErrorKind::StateFailure,
                    "run cleanup refuses unknown files or directories",
                ));
            }
            fs::remove_file(&marker_path)
                .map_err(|error| failure(ErrorKind::StateFailure, "remove owner marker", error))?;
            fs::remove_file(&schema_path)
                .map_err(|error| failure(ErrorKind::StateFailure, "remove schema marker", error))?;
            owned_run.guard.mark_for_delete(&run)?;
            owned.remove(marker);
            Ok(true)
        }
    }

    fn validate_service_containment(
        documents: &Path,
        code: &Path,
        service: &Path,
    ) -> Result<(), FastSearchError> {
        let overlaps = service.starts_with(documents)
            || service.starts_with(code)
            || documents.starts_with(service)
            || code.starts_with(service);
        let allowed = service
            .strip_prefix(documents)
            .ok()
            .is_some_and(is_exact_reserved_descendant)
            || service
                .strip_prefix(code)
                .ok()
                .is_some_and(is_exact_reserved_descendant);
        if overlaps && !allowed {
            return Err(FastSearchError::new(
                ErrorKind::InvalidContent,
                "service root may overlap a source root only inside the reserved .cfknowledge zone",
            ));
        }
        Ok(())
    }

    fn validate_service_path_before_write(
        documents: &Path,
        code: &Path,
        requested: &Path,
    ) -> Result<(), FastSearchError> {
        let requested = absolute_service_path(requested)?;
        ensure_no_reparse_points(&requested)?;
        let requested = canonicalize_existing_ancestor(&requested)?;
        ensure_no_reparse_points(&requested)?;
        validate_service_containment(documents, code, &requested)
    }

    fn absolute_service_path(requested: &Path) -> Result<PathBuf, FastSearchError> {
        if requested.is_absolute() {
            Ok(requested.to_path_buf())
        } else {
            std::env::current_dir()
                .map_err(|error| failure(ErrorKind::SourceFailure, "resolve service path", error))
                .map(|current| current.join(requested))
        }
    }

    fn canonicalize_existing_ancestor(path: &Path) -> Result<PathBuf, FastSearchError> {
        let mut missing = Vec::new();
        let mut ancestor = path;
        while !ancestor.exists() {
            let name = ancestor.file_name().ok_or_else(|| {
                FastSearchError::new(
                    ErrorKind::InvalidContent,
                    "service root has no existing ancestor",
                )
            })?;
            missing.push(name.to_os_string());
            ancestor = ancestor.parent().ok_or_else(|| {
                FastSearchError::new(
                    ErrorKind::InvalidContent,
                    "service root has no existing ancestor",
                )
            })?;
        }
        let mut resolved = ancestor.canonicalize().map_err(|error| {
            failure(
                ErrorKind::SourceFailure,
                "canonicalize service ancestor",
                error,
            )
        })?;
        for name in missing.into_iter().rev() {
            resolved.push(name);
        }
        Ok(resolved)
    }

    fn is_exact_reserved_descendant(relative: &Path) -> bool {
        let parts = relative.components().collect::<Vec<_>>();
        parts.len() == 2
            && parts[0].as_os_str() == ".cfknowledge"
            && matches!(parts[1], std::path::Component::Normal(_))
    }

    fn ensure_no_reparse_points(path: &Path) -> Result<(), FastSearchError> {
        let mut current = PathBuf::new();
        for component in path.components() {
            current.push(component.as_os_str());
            if !current.exists() {
                continue;
            }
            let metadata = fs::symlink_metadata(&current).map_err(|error| {
                failure(ErrorKind::SourceFailure, "inspect service path", error)
            })?;
            #[cfg(windows)]
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(FastSearchError::new(
                    ErrorKind::InvalidContent,
                    "service root path contains a reparse point",
                ));
            }
            #[cfg(not(windows))]
            if metadata.file_type().is_symlink() {
                return Err(FastSearchError::new(
                    ErrorKind::InvalidContent,
                    "service root path contains a symbolic link",
                ));
            }
        }
        Ok(())
    }

    fn validate_marker(marker: &str) -> Result<(), FastSearchError> {
        if marker.is_empty()
            || marker.len() > 128
            || !marker
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(FastSearchError::new(
                ErrorKind::InvalidIdentifier,
                "run marker must be 1..128 ASCII letters, digits, '-' or '_'",
            ));
        }
        Ok(())
    }

    fn write_new_run_markers(run: &Path, token: &str) -> Result<(), FastSearchError> {
        let mut owner = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(run.join("owner.marker"))
            .map_err(|error| failure(ErrorKind::StateFailure, "create run marker", error))?;
        owner
            .write_all(token.as_bytes())
            .map_err(|error| failure(ErrorKind::StateFailure, "write run marker", error))?;
        let mut schema = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(run.join("schema.marker"))
            .map_err(|error| failure(ErrorKind::StateFailure, "create run schema", error))?;
        schema
            .write_all(b"fastsearch-run-v1")
            .map_err(|error| failure(ErrorKind::StateFailure, "write run schema", error))
    }

    struct OwnedRun {
        token: String,
        guard: RunDirectoryGuard,
    }

    #[cfg(windows)]
    struct OwnedDirectoryHandle(HANDLE);
    // SAFETY: a Windows kernel HANDLE is process-wide and may be used or closed
    // from any thread. This wrapper has exclusive ownership, is never cloned,
    // and closes the handle exactly once from Drop after the move completes.
    #[cfg(windows)]
    unsafe impl Send for OwnedDirectoryHandle {}
    #[cfg(windows)]
    impl Drop for OwnedDirectoryHandle {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }
    #[cfg(windows)]
    struct RunDirectoryGuard(OwnedDirectoryHandle);
    #[cfg(windows)]
    impl RunDirectoryGuard {
        fn acquire(path: &Path) -> Result<Self, FastSearchError> {
            let handle = open_directory_handle(
                path,
                FILE_LIST_DIRECTORY | DELETE,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            )?;
            let mut attributes = FILE_ATTRIBUTE_TAG_INFO::default();
            let inspected = unsafe {
                GetFileInformationByHandleEx(
                    handle.0,
                    FileAttributeTagInfo,
                    (&raw mut attributes).cast(),
                    size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
                )
            };
            if inspected == 0 || attributes.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return Err(FastSearchError::new(
                    ErrorKind::InvalidContent,
                    "run directory is or became a reparse point",
                ));
            }
            Ok(Self(handle))
        }
        fn mark_for_delete(&self, _path: &Path) -> Result<(), FastSearchError> {
            let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
            let deleted = unsafe {
                SetFileInformationByHandle(
                    self.0.0,
                    FileDispositionInfo,
                    (&raw const disposition).cast(),
                    size_of::<FILE_DISPOSITION_INFO>() as u32,
                )
            };
            if deleted == 0 {
                return Err(failure(
                    ErrorKind::StateFailure,
                    "mark exact run directory for deletion",
                    std::io::Error::last_os_error(),
                ));
            }
            Ok(())
        }
    }
    #[cfg(not(windows))]
    struct RunDirectoryGuard;
    #[cfg(not(windows))]
    impl RunDirectoryGuard {
        fn acquire(_path: &Path) -> Result<Self, FastSearchError> {
            Ok(Self)
        }
        fn mark_for_delete(&self, path: &Path) -> Result<(), FastSearchError> {
            fs::remove_dir(path)
                .map_err(|error| failure(ErrorKind::StateFailure, "remove empty exact run", error))
        }
    }
    #[cfg(windows)]
    pub(super) struct PathGuards {
        _handles: Vec<OwnedDirectoryHandle>,
    }
    #[cfg(not(windows))]
    pub(super) struct PathGuards;

    #[cfg(windows)]
    pub(super) fn securely_create_and_pin_service(
        requested: &Path,
    ) -> Result<(PathBuf, PathGuards), FastSearchError> {
        let absolute = absolute_service_path(requested)?;
        let mut current = PathBuf::new();
        let mut handles = Vec::new();
        for component in absolute.components() {
            current.push(component.as_os_str());
            if current.exists() {
                bootstrap_before_existing_component(&current);
            } else {
                fs::create_dir(&current).map_err(|error| {
                    failure(
                        ErrorKind::StateFailure,
                        "create pinned service directory",
                        error,
                    )
                })?;
            }
            handles.push(open_directory_without_delete_share(&current)?);
        }
        let service = current.canonicalize().map_err(|error| {
            failure(
                ErrorKind::SourceFailure,
                "canonicalize pinned service root",
                error,
            )
        })?;
        let runs = service.join("runs");
        if !runs.exists() {
            fs::create_dir(&runs).map_err(|error| {
                failure(ErrorKind::StateFailure, "create pinned runs root", error)
            })?;
        }
        handles.push(open_directory_without_delete_share(&runs)?);
        Ok((service, PathGuards { _handles: handles }))
    }
    #[cfg(windows)]
    fn open_directory_without_delete_share(
        path: &Path,
    ) -> Result<OwnedDirectoryHandle, FastSearchError> {
        let handle = open_directory_handle(
            path,
            FILE_LIST_DIRECTORY,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
        )?;
        let mut attributes = FILE_ATTRIBUTE_TAG_INFO::default();
        let inspected = unsafe {
            GetFileInformationByHandleEx(
                handle.0,
                FileAttributeTagInfo,
                (&raw mut attributes).cast(),
                size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
            )
        };
        if inspected == 0 || attributes.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(FastSearchError::new(
                ErrorKind::InvalidContent,
                "service bootstrap encountered a reparse point",
            ));
        }
        Ok(handle)
    }
    #[cfg(windows)]
    fn open_directory_handle(
        path: &Path,
        desired_access: u32,
        flags: u32,
    ) -> Result<OwnedDirectoryHandle, FastSearchError> {
        let wide = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                desired_access,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                flags,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(failure(
                ErrorKind::StateFailure,
                "lock service directory against replacement",
                std::io::Error::last_os_error(),
            ));
        }
        Ok(OwnedDirectoryHandle(handle))
    }
    #[cfg(not(windows))]
    fn securely_create_and_pin_service(
        requested: &Path,
    ) -> Result<(PathBuf, PathGuards), FastSearchError> {
        fs::create_dir_all(requested)
            .map_err(|error| failure(ErrorKind::StateFailure, "create service root", error))?;
        ensure_no_reparse_points(requested)?;
        let service = requested.canonicalize().map_err(|error| {
            failure(ErrorKind::SourceFailure, "canonicalize service root", error)
        })?;
        if !service.is_dir() {
            return Err(FastSearchError::new(
                ErrorKind::InvalidContent,
                "service root must be a directory",
            ));
        }
        fs::create_dir_all(service.join("runs"))
            .map_err(|error| failure(ErrorKind::StateFailure, "create runs root", error))?;
        Ok((service, PathGuards))
    }
    fn failure(kind: ErrorKind, context: &str, error: std::io::Error) -> FastSearchError {
        FastSearchError::new(kind, format!("{context}: {error}"))
    }

    #[cfg(all(test, windows))]
    #[derive(Clone)]
    pub(super) struct BootstrapHook {
        pub(super) target: PathBuf,
        pub(super) reached: std::sync::Arc<std::sync::Barrier>,
        pub(super) resume: std::sync::Arc<std::sync::Barrier>,
    }
    #[cfg(all(test, windows))]
    pub(super) static BOOTSTRAP_HOOK: std::sync::OnceLock<Mutex<Option<BootstrapHook>>> =
        std::sync::OnceLock::new();
    #[cfg(all(test, windows))]
    fn bootstrap_before_existing_component(path: &Path) {
        let hook = BOOTSTRAP_HOOK
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap()
            .clone();
        if let Some(hook) = hook.filter(|hook| hook.target == path) {
            hook.reached.wait();
            hook.resume.wait();
        }
    }
    #[cfg(not(all(test, windows)))]
    fn bootstrap_before_existing_component(_path: &Path) {}
}

/// Replaceable machine paths for the one full production composition.
#[derive(Clone, Debug)]
pub struct ProductionConfig {
    document_roots: Vec<ConfiguredRoot>,
    code_roots: Vec<ConfiguredRoot>,
    service_root: PathBuf,
    e5_root: Option<PathBuf>,
    embedding_model: EmbeddingModelId,
    execution_device: ExecutionDevice,
    workspace_root: Option<PathBuf>,
    workspace_layout: bool,
}

#[derive(Clone, Debug)]
struct ConfiguredRoot {
    id: Option<String>,
    path: PathBuf,
}

impl ProductionConfig {
    #[must_use]
    pub fn new(
        document_root: impl Into<PathBuf>,
        code_root: impl Into<PathBuf>,
        service_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            document_roots: vec![ConfiguredRoot {
                id: None,
                path: document_root.into(),
            }],
            code_roots: vec![ConfiguredRoot {
                id: Some("code-fastsearch".to_owned()),
                path: code_root.into(),
            }],
            service_root: service_root.into(),
            e5_root: None,
            embedding_model: EmbeddingModelId::MultilingualE5Small,
            execution_device: ExecutionDevice::Cpu,
            workspace_root: None,
            workspace_layout: false,
        }
    }

    pub fn for_workspace(
        workspace_root: impl Into<PathBuf>,
        document_roots: Vec<(String, PathBuf)>,
        code_roots: Vec<(String, PathBuf)>,
        service_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            document_roots: document_roots
                .into_iter()
                .map(|(id, path)| ConfiguredRoot { id: Some(id), path })
                .collect(),
            code_roots: code_roots
                .into_iter()
                .map(|(id, path)| ConfiguredRoot { id: Some(id), path })
                .collect(),
            service_root: service_root.into(),
            e5_root: None,
            embedding_model: EmbeddingModelId::MultilingualE5Small,
            execution_device: ExecutionDevice::Cpu,
            workspace_root: Some(workspace_root.into()),
            workspace_layout: true,
        }
    }

    #[must_use]
    pub fn with_e5_root(mut self, e5_root: impl Into<PathBuf>) -> Self {
        self.e5_root = Some(e5_root.into());
        self.embedding_model = EmbeddingModelId::MultilingualE5Small;
        self
    }

    #[must_use]
    pub fn with_embedding_model(
        mut self,
        model: EmbeddingModelId,
        model_root: impl Into<PathBuf>,
    ) -> Self {
        self.e5_root = Some(model_root.into());
        self.embedding_model = model;
        self
    }

    #[must_use]
    pub fn with_execution_device(mut self, device: ExecutionDevice) -> Self {
        self.execution_device = device;
        self
    }
}

/// Filesystem + SQLite + Tantivy + local E5 + maps + symbols + E1 fusion.
pub struct ProductionRuntime {
    service: security::ServiceRunBoundary,
    documents: Vec<FilesystemSource>,
    maps: Vec<CodeMapSource>,
    symbols: Vec<SymbolSource>,
    state: SqliteStateStore,
    lexical: TantivyLexical,
    vector: LocalE5Vector,
    vector_configured: bool,
    workspace_layout: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModelPartitionMetrics {
    size_bytes: u64,
    build_duration_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum IndexingStage {
    Sources,
    State,
    Lexical,
    Vector,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum IndexingWorkStage {
    Vectorizing,
    Saving,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct IndexingProgress {
    pub(super) completed: u64,
    pub(super) total: u64,
    pub(super) stage: IndexingStage,
    pub(super) work_completed: Option<u64>,
    pub(super) work_total: Option<u64>,
    pub(super) work_stage: Option<IndexingWorkStage>,
}

impl ModelPartitionMetrics {
    #[must_use]
    pub const fn size_bytes(self) -> u64 {
        self.size_bytes
    }

    #[must_use]
    pub const fn build_duration_ms(self) -> u64 {
        self.build_duration_ms
    }
}

struct IndexingCoordinator<'a> {
    documents: &'a [FilesystemSource],
    maps: &'a [CodeMapSource],
    symbols: &'a [SymbolSource],
    state: &'a mut SqliteStateStore,
    lexical: &'a TantivyLexical,
    vector: &'a LocalE5Vector,
    vector_configured: bool,
}

impl IndexingCoordinator<'_> {
    fn project(
        &mut self,
        rebuild: bool,
        include_active_vector: bool,
    ) -> Result<LifecycleStatus, FastSearchError> {
        self.project_with_progress(rebuild, include_active_vector, &mut |_| {})
    }

    fn project_with_progress(
        &mut self,
        rebuild: bool,
        include_active_vector: bool,
        progress: &mut dyn FnMut(IndexingProgress),
    ) -> Result<LifecycleStatus, FastSearchError> {
        let vector_enabled = include_active_vector && self.vector_configured;
        let total = if vector_enabled { 4 } else { 3 };
        progress(IndexingProgress {
            completed: 0,
            total,
            stage: IndexingStage::Sources,
            work_completed: None,
            work_total: None,
            work_stage: None,
        });
        let mut snapshots = Vec::new();
        for source in self.documents {
            snapshots.extend(source.snapshot()?);
        }
        for source in self.maps {
            snapshots.extend(source.snapshot()?);
        }
        for source in self.symbols {
            snapshots.extend(source.snapshot()?);
        }
        let records = snapshots
            .iter()
            .flat_map(|snapshot| snapshot.records().iter().cloned())
            .collect::<Vec<_>>();
        progress(IndexingProgress {
            completed: 1,
            total,
            stage: IndexingStage::State,
            work_completed: None,
            work_total: None,
            work_stage: None,
        });
        let changes = self.state.reconcile_snapshots(&snapshots)?;
        progress(IndexingProgress {
            completed: 2,
            total,
            stage: IndexingStage::Lexical,
            work_completed: None,
            work_total: None,
            work_stage: None,
        });
        let lexical = if rebuild {
            self.lexical
                .rebuild(&records, changes.durable_generation())?
        } else {
            let current = self.lexical.lifecycle_status();
            let unchanged = changes
                .changes()
                .iter()
                .all(|change| *change == StateChange::Unchanged);
            if unchanged
                && current.freshness() == IndexFreshness::Current
                && current.projection_generation() == Some(changes.durable_generation())
            {
                current
            } else {
                self.lexical
                    .apply_projection(&records, changes.durable_generation())?
            }
        };
        if vector_enabled {
            progress(IndexingProgress {
                completed: 3,
                total,
                stage: IndexingStage::Vector,
                work_completed: Some(0),
                work_total: Some(records.len() as u64),
                work_stage: Some(IndexingWorkStage::Vectorizing),
            });
            self.vector.apply_with_progress(
                &records,
                changes.durable_generation(),
                &mut |event| {
                    let (work_completed, work_total, work_stage) = match event {
                        VectorBuildProgress::Embedding {
                            completed_records,
                            total_records,
                        } => (
                            Some(completed_records),
                            Some(total_records),
                            IndexingWorkStage::Vectorizing,
                        ),
                        VectorBuildProgress::Saving => (None, None, IndexingWorkStage::Saving),
                    };
                    progress(IndexingProgress {
                        completed: 3,
                        total,
                        stage: IndexingStage::Vector,
                        work_completed,
                        work_total,
                        work_stage: Some(work_stage),
                    });
                },
            )?;
        }
        Ok(lexical)
    }

    fn lifecycle_status(state: &SqliteStateStore, lexical: &TantivyLexical) -> LifecycleStatus {
        let state = state.lifecycle_status();
        let lexical = lexical.lifecycle_status();
        if state.freshness() == IndexFreshness::Degraded {
            return state;
        }
        if lexical.freshness() == IndexFreshness::Current
            && lexical.projection_generation() == Some(state.state_generation())
        {
            LifecycleStatus::new(
                IndexFreshness::Current,
                state.state_generation(),
                lexical.projection_generation(),
                lexical.detail(),
            )
        } else {
            LifecycleStatus::new(
                if lexical.freshness() == IndexFreshness::Degraded {
                    IndexFreshness::Degraded
                } else {
                    IndexFreshness::Stale
                },
                state.state_generation(),
                lexical.projection_generation(),
                lexical.detail(),
            )
        }
    }
}

struct SearchCoordinator<'a> {
    lexical: &'a TantivyLexical,
    vector: &'a LocalE5Vector,
    vector_configured: bool,
    maps: &'a [CodeMapSource],
    symbols: &'a [SymbolSource],
}

impl SearchCoordinator<'_> {
    fn search(
        &self,
        query: &SearchQuery,
        status: Vec<CapabilityStatus>,
    ) -> Result<SearchResponse, FastSearchError> {
        let lexical = self.lexical.search(query)?;
        let mut grouped = BTreeMap::<u8, (RetrievalChannel, Vec<SearchHit>)>::new();
        for hit in lexical.hits() {
            let channel = hit.channel();
            grouped
                .entry(channel_order(channel))
                .or_insert_with(|| (channel, Vec::new()))
                .1
                .push(hit.clone());
        }
        let mut candidates = grouped
            .into_values()
            .map(|(channel, hits)| ChannelCandidates::new(channel, hits, lexical.freshness()))
            .collect::<Result<Vec<_>, _>>()?;

        if self.vector_configured {
            let vector = self.vector.search(query)?;
            candidates.push(ChannelCandidates::new(
                RetrievalChannel::Vector,
                vector.hits().to_vec(),
                vector.freshness(),
            )?);
        }
        let needle = query.text().to_lowercase();
        let mut map_records = Vec::new();
        for source in self.maps {
            map_records.extend(source.records()?);
        }
        let maps = map_records
            .into_iter()
            .filter(|record| {
                record.title().to_lowercase().contains(&needle)
                    || record.searchable_content().to_lowercase().contains(&needle)
            })
            .map(|record| SearchHit::new(record, RetrievalChannel::CodeMap, 0.0))
            .collect::<Vec<_>>();
        candidates.push(ChannelCandidates::new(
            RetrievalChannel::CodeMap,
            maps,
            IndexFreshness::Current,
        )?);
        let mut symbol_records = Vec::new();
        for source in self.symbols {
            symbol_records.extend(source.find_symbols(query)?);
        }
        let symbols = symbol_records
            .into_iter()
            .map(|record| SearchHit::new(record, RetrievalChannel::Symbol, 0.0))
            .collect::<Vec<_>>();
        candidates.push(ChannelCandidates::new(
            RetrievalChannel::Symbol,
            symbols,
            IndexFreshness::Current,
        )?);
        Ok(FusionCoordinator::fuse(query, candidates, &status))
    }

    fn related(
        &self,
        maps: Vec<CanonicalRecord>,
        query: &RelatedQuery,
    ) -> Result<Vec<CanonicalRecord>, FastSearchError> {
        CodeMapRelated::new(maps)?.related_maps(query)
    }
}

impl std::fmt::Debug for ProductionRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProductionRuntime")
            .finish_non_exhaustive()
    }
}

impl ProductionRuntime {
    pub fn open(config: ProductionConfig) -> Result<Self, FastSearchError> {
        let workspace_layout = config.workspace_layout;
        let embedding_model = config.embedding_model;
        let execution_device = config.execution_device;
        let document_roots = canonical_roots(&config.document_roots, "document root")?;
        let code_roots = canonical_roots(&config.code_roots, "code root")?;
        let source_paths = document_roots
            .iter()
            .chain(code_roots.iter())
            .map(|(_, path)| path.clone())
            .collect::<Vec<_>>();
        let service = match config.workspace_root.as_deref() {
            Some(workspace_root) => security::ServiceRunBoundary::admit_workspace_and_pin(
                workspace_root,
                &source_paths,
                &config.service_root,
            )?,
            None => {
                let documents = document_roots.first().ok_or_else(|| {
                    FastSearchError::new(
                        ErrorKind::InvalidContent,
                        "legacy production configuration requires a document root",
                    )
                })?;
                let code = code_roots.first().ok_or_else(|| {
                    FastSearchError::new(
                        ErrorKind::InvalidContent,
                        "legacy production configuration requires a code root",
                    )
                })?;
                security::ServiceRunBoundary::admit_and_pin(
                    &documents.1,
                    &code.1,
                    &config.service_root,
                )?
            }
        };
        let vector_configured = config.e5_root.is_some();
        let model_root = config
            .e5_root
            .unwrap_or_else(|| service.service_root().join("unconfigured-e5"));
        let lexical_root = if config.workspace_layout {
            service
                .service_root()
                .join("index")
                .join("cross")
                .join("lexical")
        } else {
            service.service_root().join("lexical")
        };

        let documents = document_roots
            .iter()
            .map(|(id, path)| match id {
                Some(id) => Ok(FilesystemSource::new_named(id.clone(), path.clone())),
                None => Ok(FilesystemSource::new(path.clone())),
            })
            .collect::<Result<Vec<_>, FastSearchError>>()?;
        let maps = document_roots
            .iter()
            .map(|(id, path)| match id {
                Some(id) => CodeMapSource::new_named(id.clone(), path.clone()),
                None => CodeMapSource::new(path.clone()),
            })
            .collect();
        let symbols = code_roots
            .into_iter()
            .map(|(id, path)| {
                let id = id.ok_or_else(|| {
                    FastSearchError::new(
                        ErrorKind::InvalidIdentifier,
                        "code root requires a logical root ID",
                    )
                })?;
                Ok(if config.workspace_layout {
                    SymbolSource::new_workspace(id, path)
                } else {
                    SymbolSource::new(id, path)
                })
            })
            .collect::<Result<Vec<_>, FastSearchError>>()?;

        let state = SqliteStateStore::open(service.service_root().join("state.sqlite"))?;
        let vector = if workspace_layout {
            LocalE5Vector::open_persistent_with_model_on_device(
                model_root,
                super::model_cache::model_identity(embedding_model),
                embedding_model,
                model_partition_root(service.service_root(), embedding_model),
                execution_device,
            )
        } else {
            LocalE5Vector::open_with_model_on_device(
                model_root,
                super::model_cache::model_identity(embedding_model),
                embedding_model,
                execution_device,
            )
        };
        if workspace_layout && vector_configured {
            vector.restore(&state.all_records()?, state.durable_generation()?)?;
        }

        Ok(Self {
            state,
            lexical: TantivyLexical::open(lexical_root)?,
            service,
            documents,
            maps,
            symbols,
            vector,
            vector_configured,
            workspace_layout,
        })
    }

    pub fn index(&mut self) -> Result<LifecycleStatus, FastSearchError> {
        self.indexing().project(false, true)
    }

    pub fn rebuild(&mut self) -> Result<LifecycleStatus, FastSearchError> {
        self.indexing().project(true, true)
    }

    pub(super) fn index_with_progress(
        &mut self,
        mut progress: impl FnMut(IndexingProgress),
    ) -> Result<LifecycleStatus, FastSearchError> {
        self.indexing()
            .project_with_progress(false, true, &mut progress)
    }

    pub(super) fn rebuild_with_progress(
        &mut self,
        mut progress: impl FnMut(IndexingProgress),
    ) -> Result<LifecycleStatus, FastSearchError> {
        self.indexing()
            .project_with_progress(true, true, &mut progress)
    }

    pub(super) fn index_shared_for_comparison_with_progress(
        &mut self,
        mut progress: impl FnMut(IndexingProgress),
    ) -> Result<LifecycleStatus, FastSearchError> {
        self.indexing()
            .project_with_progress(false, false, &mut progress)
    }

    /// Read-only readiness of one model partition against the current shared
    /// canonical state. This never opens or downloads model weights.
    pub fn model_partition_status(&self, model: EmbeddingModelId) -> LifecycleStatus {
        let records = match self.state.all_records() {
            Ok(records) => records,
            Err(error) => {
                return LifecycleStatus::new(IndexFreshness::Degraded, 0, None, error.message());
            }
        };
        let generation = match self.state.durable_generation() {
            Ok(generation) => generation,
            Err(error) => {
                return LifecycleStatus::new(IndexFreshness::Degraded, 0, None, error.message());
            }
        };
        LocalE5Vector::persistent_status(
            &model_partition_root(self.service.service_root(), model),
            model,
            &super::model_cache::model_identity(model),
            &records,
            generation,
        )
    }

    /// Stored measurements for the committed files of one model partition.
    /// This is read-only and never opens model weights.
    pub fn model_partition_metrics(
        &self,
        model: EmbeddingModelId,
    ) -> Result<Option<ModelPartitionMetrics>, FastSearchError> {
        Ok(LocalE5Vector::persistent_metrics(&model_partition_root(
            self.service.service_root(),
            model,
        ))?
        .map(|metrics| ModelPartitionMetrics {
            size_bytes: metrics.size_bytes(),
            build_duration_ms: metrics.build_duration_ms(),
        }))
    }

    /// Builds exactly one model-specific vector partition from the current
    /// shared canonical state without rescanning sources or rebuilding lexical.
    pub fn build_model_partition(
        &self,
        model: EmbeddingModelId,
        model_root: &Path,
    ) -> Result<LifecycleStatus, FastSearchError> {
        self.build_model_partition_with_progress(model, model_root, |_| {})
    }

    pub(super) fn build_model_partition_with_progress(
        &self,
        model: EmbeddingModelId,
        model_root: &Path,
        mut progress: impl FnMut(VectorBuildProgress),
    ) -> Result<LifecycleStatus, FastSearchError> {
        if !self.workspace_layout {
            return Err(FastSearchError::new(
                ErrorKind::InvalidContent,
                "model partitions require a workspace layout",
            ));
        }
        let records = self.state.all_records()?;
        let generation = self.state.durable_generation()?;
        let vector = LocalE5Vector::open_persistent_with_model_on_device(
            model_root,
            super::model_cache::model_identity(model),
            model,
            model_partition_root(self.service.service_root(), model),
            super::model_cache::configured_model_device(model)?,
        );
        vector.restore(&records, generation)?;
        vector.apply_with_progress(&records, generation, &mut progress)
    }

    /// Executes vector-only retrieval for one already admitted model
    /// partition. Shared lexical results are requested separately once.
    pub fn search_model_partition(
        &self,
        model: EmbeddingModelId,
        model_root: &Path,
        query: &SearchQuery,
    ) -> Result<SearchResponse, FastSearchError> {
        let records = self.state.all_records()?;
        let generation = self.state.durable_generation()?;
        let vector = LocalE5Vector::open_persistent_with_model_on_device(
            model_root,
            super::model_cache::model_identity(model),
            model,
            model_partition_root(self.service.service_root(), model),
            super::model_cache::configured_model_device(model)?,
        );
        vector.restore(&records, generation)?;
        vector.search(query)
    }

    pub fn lexical_baseline(&self, query: &SearchQuery) -> Result<SearchResponse, FastSearchError> {
        self.lexical.search(query)
    }

    /// Creates an exact run-owned directory used by acceptance jobs and batch callers.
    pub fn record_run_marker(&self, marker: &str) -> Result<PathBuf, FastSearchError> {
        self.service.record_run_marker(marker)
    }

    /// Removes only a directory whose marker content exactly matches the requested run.
    pub fn cleanup_run(&self, marker: &str) -> Result<bool, FastSearchError> {
        self.service.cleanup_run(marker)
    }

    fn indexing(&mut self) -> IndexingCoordinator<'_> {
        IndexingCoordinator {
            documents: &self.documents,
            maps: &self.maps,
            symbols: &self.symbols,
            state: &mut self.state,
            lexical: &self.lexical,
            vector: &self.vector,
            vector_configured: self.vector_configured,
        }
    }

    fn search_coordinator(&self) -> SearchCoordinator<'_> {
        SearchCoordinator {
            lexical: &self.lexical,
            vector: &self.vector,
            vector_configured: self.vector_configured,
            maps: &self.maps,
            symbols: &self.symbols,
        }
    }

    fn combined_records(&self) -> Result<Vec<CanonicalRecord>, FastSearchError> {
        let mut records = Vec::new();
        for source in &self.maps {
            records.extend(source.records()?);
        }
        for source in &self.symbols {
            records.extend(source.records()?);
        }
        Ok(records)
    }

    fn lifecycle_status(&self) -> LifecycleStatus {
        IndexingCoordinator::lifecycle_status(&self.state, &self.lexical)
    }
}

fn model_partition_root(service_root: &Path, model: EmbeddingModelId) -> PathBuf {
    service_root
        .join("index")
        .join("vector")
        .join(model.slug())
        .join(super::model_cache::model_descriptor(model).revision)
}

impl AgentSurface for ProductionRuntime {
    fn search(&self, query: &SearchQuery) -> Result<SearchResponse, FastSearchError> {
        self.search_coordinator().search(query, self.status())
    }

    fn get(&self, id: &StableId) -> Result<Option<CanonicalRecord>, FastSearchError> {
        self.state.get(id)
    }

    fn related(&self, query: &RelatedQuery) -> Result<Vec<CanonicalRecord>, FastSearchError> {
        self.search_coordinator()
            .related(self.combined_records()?, query)
    }

    fn status(&self) -> Vec<CapabilityStatus> {
        vec![
            CapabilityStatus::available(Capability::Source, BackendKind::Real),
            CapabilityStatus::available(Capability::State, BackendKind::Real),
            CapabilityStatus::available(Capability::LexicalRetrieval, BackendKind::Real),
            if self.vector_configured {
                self.vector.capability_status()
            } else {
                CapabilityStatus::unavailable(
                    Capability::VectorRetrieval,
                    "local E5 root is not configured",
                )
            },
            if self.maps.is_empty() {
                CapabilityStatus::unavailable(
                    Capability::CodeMaps,
                    "documentation contour is not configured",
                )
            } else {
                CapabilityStatus::available(Capability::CodeMaps, BackendKind::Real)
            },
            if self.symbols.is_empty() {
                CapabilityStatus::unavailable(Capability::Symbols, "code contour is not configured")
            } else {
                CapabilityStatus::available(Capability::Symbols, BackendKind::Real)
            },
        ]
    }

    fn index_status(&self) -> LifecycleStatus {
        self.lifecycle_status()
    }
}

fn channel_order(channel: RetrievalChannel) -> u8 {
    match channel {
        RetrievalChannel::Exact => 0,
        RetrievalChannel::Lexical => 1,
        RetrievalChannel::Vector => 2,
        RetrievalChannel::CodeMap => 3,
        RetrievalChannel::Symbol => 4,
    }
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf, FastSearchError> {
    let canonical = path.canonicalize().map_err(|error| {
        FastSearchError::new(
            ErrorKind::SourceFailure,
            format!("canonicalize {label}: {error}"),
        )
    })?;
    if !canonical.is_dir() {
        return Err(FastSearchError::new(
            ErrorKind::InvalidContent,
            format!("{label} must be a directory"),
        ));
    }
    Ok(canonical)
}

fn canonical_roots(
    configured: &[ConfiguredRoot],
    label: &str,
) -> Result<Vec<(Option<LogicalRootId>, PathBuf)>, FastSearchError> {
    configured
        .iter()
        .map(|root| {
            Ok((
                root.id
                    .as_ref()
                    .map(|id| LogicalRootId::parse(id.clone()))
                    .transpose()?,
                canonical_directory(&root.path, label)?,
            ))
        })
        .collect()
}

#[cfg(all(test, windows))]
mod bootstrap_race_tests {
    use super::security::{BOOTSTRAP_HOOK, BootstrapHook};
    use super::*;
    use std::{
        fs,
        process::Command,
        sync::{Arc, Barrier, Mutex},
        thread,
        time::SystemTime,
    };

    #[test]
    fn existing_parent_swap_is_denied_or_detected_before_external_state_write() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("fastsearch-bootstrap-hook-{nonce}"));
        let documents = root.join("documents");
        let code = root.join("code");
        let parent = root.join("existing-parent");
        let displaced = root.join("displaced-parent");
        let external = root.join("external");
        fs::create_dir_all(&documents).unwrap();
        fs::create_dir_all(&code).unwrap();
        fs::create_dir_all(&parent).unwrap();
        fs::create_dir_all(&external).unwrap();
        fs::write(documents.join("safe.md"), "# Safe").unwrap();
        fs::write(code.join("safe.rs"), "pub fn safe() {}").unwrap();
        fs::write(external.join("sentinel.txt"), "unchanged").unwrap();

        let reached = Arc::new(Barrier::new(2));
        let resume = Arc::new(Barrier::new(2));
        *BOOTSTRAP_HOOK
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap() = Some(BootstrapHook {
            target: parent.clone(),
            reached: Arc::clone(&reached),
            resume: Arc::clone(&resume),
        });
        let service = parent.join("service");
        let worker = thread::spawn(move || {
            ProductionRuntime::open(ProductionConfig::new(documents, code, service))
                .map(drop)
                .map_err(|error| error.to_string())
        });
        reached.wait();
        let command = format!(
            "ren \"{}\" \"{}\" && mklink /J \"{}\" \"{}\"",
            parent.display(),
            displaced.file_name().unwrap().to_string_lossy(),
            parent.display(),
            external.display()
        );
        let attack = Command::new("cmd")
            .args(["/d", "/s", "/c", &command])
            .output()
            .unwrap();
        resume.wait();
        let result = worker.join().unwrap();
        *BOOTSTRAP_HOOK.get().unwrap().lock().unwrap() = None;

        assert!(
            !attack.status.success() || result.is_err(),
            "successful parent swap must be detected by no-follow component open"
        );
        assert_eq!(
            fs::read_to_string(external.join("sentinel.txt")).unwrap(),
            "unchanged"
        );
        assert!(!external.join("state.sqlite").exists());
        assert!(!external.join("lexical").exists());
        drop(result);
        let _ = fs::remove_dir(&parent);
        if displaced.exists() {
            let _ = fs::rename(&displaced, &parent);
        }
        let unlocked = root.join("unlocked-parent");
        fs::rename(&parent, &unlocked).expect("all bootstrap handles must close on return");
        fs::rename(&unlocked, &parent).unwrap();

        for attempt in 0..16 {
            let rejected_parent = root.join(format!("rejected-parent-{attempt}"));
            let rejected_external = root.join(format!("rejected-external-{attempt}"));
            fs::create_dir_all(&rejected_external).unwrap();
            fs::write(rejected_external.join("sentinel.txt"), "unchanged").unwrap();
            let linked = Command::new("cmd")
                .args(["/c", "mklink", "/J"])
                .arg(&rejected_parent)
                .arg(&rejected_external)
                .output()
                .unwrap();
            assert!(linked.status.success());
            let rejected =
                security::securely_create_and_pin_service(&rejected_parent.join("service"));
            assert!(rejected.is_err());
            assert!(!rejected_external.join("state.sqlite").exists());
            let removed = Command::new("cmd")
                .args(["/c", "rmdir"])
                .arg(&rejected_parent)
                .output()
                .unwrap();
            assert!(
                removed.status.success(),
                "rejected traversal leaked a HANDLE"
            );
            fs::remove_dir_all(rejected_external).unwrap();
        }
        let _ = fs::remove_dir_all(&root);
    }
}
