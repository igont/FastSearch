//! Application composition for the real document runtime.

use std::{fs, path::Path};

use crate::adapters::{lexical::TantivyLexical, source::FilesystemSource, state::SqliteStateStore};
use crate::domain::{
    BackendKind, CanonicalRecord, Capability, CapabilityStatus, ErrorKind, FastSearchError,
    IndexFreshness, LifecycleStatus, RelatedQuery, SearchQuery, SearchResponse, StableId,
};

mod cli;
mod console;
pub mod fusion;
mod production;

pub use cli::{CliError, OutputFormat, execute_cli, execute_cli_formatted};
pub use console::{help_text, run_interactive, version_text};
pub use production::{ProductionConfig, ProductionRuntime};

/// Coordinates one complete source scan with its authoritative state commit and projection.
///
/// This stays private because it is application control flow, not another domain boundary.
struct RuntimeCoordinator<S, T, L> {
    source: S,
    state: T,
    lexical: L,
    projection_failure: Option<String>,
}

impl<S, T, L> RuntimeCoordinator<S, T, L>
where
    S: SourcePort,
    T: StateStore,
    L: LexicalRetrieval,
{
    fn new(source: S, state: T, lexical: L) -> Self {
        Self {
            source,
            state,
            lexical,
            projection_failure: None,
        }
    }

    fn index(&mut self) -> Result<LifecycleStatus, FastSearchError> {
        self.project(false)
    }

    fn index_with_test_projection_failure(&mut self) -> Result<LifecycleStatus, FastSearchError> {
        let snapshots = self.source.snapshot()?;
        // The durable authority transition intentionally happens before this isolated fault.
        // It makes the process-boundary stale/degraded recovery protocol observable without
        // replacing the real source/state/lexical composition.
        self.state.reconcile_snapshots(&snapshots)?;
        let error = FastSearchError::new(
            ErrorKind::ProjectionFailure,
            "controlled lexical projection failure",
        );
        self.projection_failure = Some(error.message().to_owned());
        Err(error)
    }

    fn rebuild(&mut self) -> Result<LifecycleStatus, FastSearchError> {
        self.project(true)
    }

    fn project(&mut self, rebuild: bool) -> Result<LifecycleStatus, FastSearchError> {
        let snapshots = self.source.snapshot()?;
        let records = snapshots
            .iter()
            .flat_map(|snapshot| snapshot.records().iter().cloned())
            .collect::<Vec<_>>();
        // A successful full source scan has exactly one state authority transition.
        let changes = self.state.reconcile_snapshots(&snapshots)?;
        let lexical_status = self.lexical.lifecycle_status();
        let projection_is_already_current = lexical_status.freshness() == IndexFreshness::Current
            && lexical_status.projection_generation() == Some(changes.durable_generation());
        let source_set_is_unchanged = changes
            .changes()
            .iter()
            .all(|change| *change == StateChange::Unchanged);
        if !rebuild && source_set_is_unchanged && projection_is_already_current {
            self.projection_failure = None;
            return Ok(lexical_status);
        }
        let projection = if rebuild {
            self.lexical.rebuild(&records, changes.durable_generation())
        } else {
            self.lexical
                .apply_projection(&records, changes.durable_generation())
        };
        match projection {
            Ok(status) => {
                self.projection_failure = None;
                Ok(status)
            }
            Err(error) => {
                self.projection_failure = Some(error.message().to_owned());
                Err(error)
            }
        }
    }

    fn status(&self) -> LifecycleStatus {
        let state = self.state.lifecycle_status();
        if state.freshness() == IndexFreshness::Degraded {
            return state;
        }
        let lexical = self.lexical.lifecycle_status();
        if let Some(detail) = &self.projection_failure {
            return LifecycleStatus::new(
                IndexFreshness::Degraded,
                state.state_generation(),
                lexical.projection_generation(),
                detail,
            );
        }
        if lexical.freshness() == IndexFreshness::Current
            && lexical.projection_generation() == Some(state.state_generation())
        {
            return LifecycleStatus::new(
                IndexFreshness::Current,
                state.state_generation(),
                lexical.projection_generation(),
                lexical.detail(),
            );
        }
        let freshness = if lexical.freshness() == IndexFreshness::Degraded {
            IndexFreshness::Degraded
        } else {
            IndexFreshness::Stale
        };
        LifecycleStatus::new(
            freshness,
            state.state_generation(),
            lexical.projection_generation(),
            lexical.detail(),
        )
    }
}

/// Production composition of filesystem source, durable SQLite authority and Tantivy projection.
pub struct RealRuntime {
    coordinator: RuntimeCoordinator<FilesystemSource, SqliteStateStore, TantivyLexical>,
}

impl RealRuntime {
    /// Opens the only production runtime factory for one source root and service directory.
    pub fn open(
        source_root: impl AsRef<Path>,
        service_root: impl AsRef<Path>,
    ) -> Result<Self, FastSearchError> {
        let service_root = service_root.as_ref();
        fs::create_dir_all(service_root).map_err(|error| {
            FastSearchError::new(
                ErrorKind::StateFailure,
                format!("create service directory: {error}"),
            )
        })?;
        Ok(Self {
            coordinator: RuntimeCoordinator::new(
                FilesystemSource::new(source_root.as_ref()),
                SqliteStateStore::open(service_root.join("state.sqlite"))?,
                TantivyLexical::open(service_root.join("lexical"))?,
            ),
        })
    }

    /// Reconciles one full source scan and updates the disposable lexical projection.
    pub fn index(&mut self) -> Result<LifecycleStatus, FastSearchError> {
        self.coordinator.index()
    }

    /// Isolated CLI-regression fault: fails only after durable source reconciliation.
    ///
    /// Normal callers must use [`Self::index`]. There is no environment or configuration
    /// switch for this behaviour.
    pub(super) fn index_with_test_projection_failure(
        &mut self,
    ) -> Result<LifecycleStatus, FastSearchError> {
        self.coordinator.index_with_test_projection_failure()
    }

    /// Reconciles one full source scan and reconstructs the lexical projection.
    pub fn rebuild(&mut self) -> Result<LifecycleStatus, FastSearchError> {
        self.coordinator.rebuild()
    }
}

impl AgentSurface for RealRuntime {
    fn search(&self, query: &SearchQuery) -> Result<SearchResponse, FastSearchError> {
        let response = self.coordinator.lexical.search(query)?;
        Ok(SearchResponse::with_freshness(
            response.hits().to_vec(),
            self.index_status().freshness(),
        ))
    }

    fn get(&self, id: &StableId) -> Result<Option<CanonicalRecord>, FastSearchError> {
        self.coordinator.state.get(id)
    }

    fn related(&self, _query: &RelatedQuery) -> Result<Vec<CanonicalRecord>, FastSearchError> {
        Err(FastSearchError::new(
            ErrorKind::CapabilityUnavailable {
                capability: Capability::CodeMaps,
            },
            "real runtime has no code maps",
        ))
    }

    fn status(&self) -> Vec<CapabilityStatus> {
        vec![
            CapabilityStatus::available(Capability::Source, BackendKind::Real),
            CapabilityStatus::available(Capability::State, BackendKind::Real),
            CapabilityStatus::available(Capability::LexicalRetrieval, BackendKind::Real),
            CapabilityStatus::unavailable(
                Capability::VectorRetrieval,
                "real runtime has no vector retrieval",
            ),
            CapabilityStatus::unavailable(Capability::CodeMaps, "real runtime has no code maps"),
            CapabilityStatus::unavailable(Capability::Symbols, "real runtime has no symbols"),
        ]
    }

    fn index_status(&self) -> LifecycleStatus {
        self.coordinator.status()
    }
}
use crate::ports::{AgentSurface, LexicalRetrieval, SourcePort, StateChange, StateStore};

#[cfg(test)]
mod real_runtime_recovery_tests {
    use std::{
        cell::Cell,
        fs,
        path::PathBuf,
        rc::Rc,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{RealRuntime, RuntimeCoordinator};
    use crate::{
        adapters::{source::FilesystemSource, state::SqliteStateStore},
        domain::{
            CanonicalRecord, ErrorKind, FastSearchError, IndexFreshness, LifecycleStatus,
            SearchQuery, SearchResponse, SourceSnapshot, StableId,
        },
        ports::{AgentSurface, LexicalRetrieval, SourcePort, StateChangeSet, StateStore},
    };

    struct TemporaryDirectory(PathBuf);

    impl TemporaryDirectory {
        fn new() -> Self {
            let suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time is after epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("fastsearch-e1-recovery-{suffix}"));
            fs::create_dir_all(&path).expect("temporary directory");
            Self(path)
        }
    }

    impl Drop for TemporaryDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct CountingState {
        inner: SqliteStateStore,
        reconciliations: Rc<Cell<usize>>,
    }

    impl StateStore for CountingState {
        fn get(&self, id: &StableId) -> Result<Option<CanonicalRecord>, FastSearchError> {
            self.inner.get(id)
        }

        fn put(&mut self, record: CanonicalRecord) -> Result<(), FastSearchError> {
            self.inner.put(record)
        }

        fn remove(&mut self, id: &StableId) -> Result<bool, FastSearchError> {
            self.inner.remove(id)
        }

        fn lifecycle_status(&self) -> LifecycleStatus {
            self.inner.lifecycle_status()
        }

        fn reconcile_snapshots(
            &mut self,
            snapshots: &[SourceSnapshot],
        ) -> Result<StateChangeSet, FastSearchError> {
            self.reconciliations.set(self.reconciliations.get() + 1);
            self.inner.reconcile_snapshots(snapshots)
        }
    }

    struct FailingLexical {
        prior_generation: u64,
    }

    impl LexicalRetrieval for FailingLexical {
        fn search(&self, _query: &SearchQuery) -> Result<SearchResponse, FastSearchError> {
            Err(FastSearchError::new(
                ErrorKind::ProjectionFailure,
                "controlled lexical failure has no search surface",
            ))
        }

        fn lifecycle_status(&self) -> LifecycleStatus {
            LifecycleStatus::new(
                IndexFreshness::Current,
                self.prior_generation,
                Some(self.prior_generation),
                "prior disposable projection",
            )
        }

        fn apply_projection(
            &self,
            _records: &[CanonicalRecord],
            _state_generation: u64,
        ) -> Result<LifecycleStatus, FastSearchError> {
            Err(FastSearchError::new(
                ErrorKind::ProjectionFailure,
                "controlled lexical failure",
            ))
        }
    }

    #[test]
    fn failed_projection_is_degraded_then_reopen_is_stale_until_rebuild() {
        let temporary = TemporaryDirectory::new();
        let source_root = temporary.0.join("source");
        let service_root = temporary.0.join("service");
        fs::create_dir_all(&source_root).expect("source root");
        let document = source_root.join("guide.md");
        fs::write(&document, "# Lifecycle\n\nДокумент для recovery-проверки.")
            .expect("source fixture");

        let source = FilesystemSource::new(&source_root);
        let deleted_id = source
            .records()
            .expect("source record")
            .into_iter()
            .next()
            .expect("one record")
            .id()
            .clone();
        let mut initial = RealRuntime::open(&source_root, &service_root).expect("initial runtime");
        assert_eq!(
            initial.index().expect("initial index").freshness(),
            IndexFreshness::Current
        );
        drop(initial);

        fs::remove_file(&document).expect("delete document");
        let reconciliations = Rc::new(Cell::new(0));
        let mut failing = RuntimeCoordinator::new(
            FilesystemSource::new(&source_root),
            CountingState {
                inner: SqliteStateStore::open(service_root.join("state.sqlite"))
                    .expect("state reopen"),
                reconciliations: Rc::clone(&reconciliations),
            },
            FailingLexical {
                prior_generation: 1,
            },
        );
        let failure = failing.index().expect_err("controlled projection failure");
        assert_eq!(failure.kind(), &ErrorKind::ProjectionFailure);
        assert_eq!(reconciliations.get(), 1, "one reconcile for one full scan");
        assert_eq!(failing.status().freshness(), IndexFreshness::Degraded);
        assert_eq!(failing.status().state_generation(), 2);
        drop(failing);

        let mut reopened = RealRuntime::open(&source_root, &service_root).expect("real reopen");
        assert!(
            reopened
                .get(&deleted_id)
                .expect("authoritative read")
                .is_none()
        );
        assert_eq!(reopened.index_status().freshness(), IndexFreshness::Stale);
        assert_eq!(
            reopened
                .search(&SearchQuery::new("lifecycle", Default::default()).expect("query"))
                .expect("stale search")
                .freshness(),
            IndexFreshness::Stale
        );
        assert_eq!(
            reopened.rebuild().expect("rebuild").freshness(),
            IndexFreshness::Current
        );
        assert!(
            reopened
                .search(&SearchQuery::new("lifecycle", Default::default()).expect("query"))
                .expect("recovered search")
                .hits()
                .is_empty()
        );
    }
}
