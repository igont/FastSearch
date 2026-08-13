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
        vector::LocalE5Vector,
    },
    application::fusion::{ChannelCandidates, FusionCoordinator},
    domain::{
        BackendKind, CanonicalRecord, Capability, CapabilityStatus, ErrorKind, FastSearchError,
        IndexFreshness, LifecycleStatus, LogicalRootId, RelatedQuery, RetrievalChannel, SearchHit,
        SearchQuery, SearchResponse, StableId,
    },
    ports::{
        AgentSurface, CodeMapPort, LexicalRetrieval, SourcePort, StateChange, StateStore,
        SymbolPort, VectorRetrieval,
    },
};

const E5_IDENTITY: &str = "multilingual-e5-small@614241f";

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
    document_root: PathBuf,
    code_root: PathBuf,
    service_root: PathBuf,
    e5_root: Option<PathBuf>,
}

impl ProductionConfig {
    #[must_use]
    pub fn new(
        document_root: impl Into<PathBuf>,
        code_root: impl Into<PathBuf>,
        service_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            document_root: document_root.into(),
            code_root: code_root.into(),
            service_root: service_root.into(),
            e5_root: None,
        }
    }

    #[must_use]
    pub fn with_e5_root(mut self, e5_root: impl Into<PathBuf>) -> Self {
        self.e5_root = Some(e5_root.into());
        self
    }
}

/// Filesystem + SQLite + Tantivy + local E5 + maps + symbols + E1 fusion.
pub struct ProductionRuntime {
    service: security::ServiceRunBoundary,
    documents: FilesystemSource,
    maps: CodeMapSource,
    symbols: SymbolSource,
    state: SqliteStateStore,
    lexical: TantivyLexical,
    vector: LocalE5Vector,
    vector_configured: bool,
}

struct IndexingCoordinator<'a> {
    documents: &'a FilesystemSource,
    maps: &'a CodeMapSource,
    symbols: &'a SymbolSource,
    state: &'a mut SqliteStateStore,
    lexical: &'a TantivyLexical,
    vector: &'a LocalE5Vector,
    vector_configured: bool,
}

impl IndexingCoordinator<'_> {
    fn project(&mut self, rebuild: bool) -> Result<LifecycleStatus, FastSearchError> {
        let mut snapshots = self.documents.snapshot()?;
        snapshots.extend(self.maps.snapshot()?);
        snapshots.extend(self.symbols.snapshot()?);
        let records = snapshots
            .iter()
            .flat_map(|snapshot| snapshot.records().iter().cloned())
            .collect::<Vec<_>>();
        let changes = self.state.reconcile_snapshots(&snapshots)?;
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
        if self.vector_configured {
            let _vector_result = if rebuild {
                self.vector.rebuild(&records, changes.durable_generation())
            } else {
                self.vector.apply(&records, changes.durable_generation())
            };
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
    maps: &'a CodeMapSource,
    symbols: &'a SymbolSource,
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

        if self.vector_configured
            && let Ok(vector) = self.vector.search(query)
        {
            candidates.push(ChannelCandidates::new(
                RetrievalChannel::Vector,
                vector.hits().to_vec(),
                vector.freshness(),
            )?);
        }
        let needle = query.text().to_lowercase();
        let maps = self
            .maps
            .records()?
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
        let symbols = self
            .symbols
            .find_symbols(query)?
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
        let document_root = canonical_directory(&config.document_root, "document root")?;
        let code_root = canonical_directory(&config.code_root, "code root")?;
        let service = security::ServiceRunBoundary::admit_and_pin(
            &document_root,
            &code_root,
            &config.service_root,
        )?;
        let vector_configured = config.e5_root.is_some();
        let model_root = config
            .e5_root
            .unwrap_or_else(|| service.service_root().join("unconfigured-e5"));

        Ok(Self {
            state: SqliteStateStore::open(service.service_root().join("state.sqlite"))?,
            lexical: TantivyLexical::open(service.service_root().join("lexical"))?,
            service,
            documents: FilesystemSource::new(document_root.clone()),
            maps: CodeMapSource::new(document_root),
            symbols: SymbolSource::new(LogicalRootId::parse("code-fastsearch")?, code_root),
            vector: LocalE5Vector::open(model_root, E5_IDENTITY),
            vector_configured,
        })
    }

    pub fn index(&mut self) -> Result<LifecycleStatus, FastSearchError> {
        self.indexing().project(false)
    }

    pub fn rebuild(&mut self) -> Result<LifecycleStatus, FastSearchError> {
        self.indexing().project(true)
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
        let mut records = self.maps.records()?;
        records.extend(self.symbols.records()?);
        Ok(records)
    }

    fn lifecycle_status(&self) -> LifecycleStatus {
        IndexingCoordinator::lifecycle_status(&self.state, &self.lexical)
    }
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
            CapabilityStatus::available(Capability::CodeMaps, BackendKind::Real),
            CapabilityStatus::available(Capability::Symbols, BackendKind::Real),
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
