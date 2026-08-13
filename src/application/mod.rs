//! Application coordination and production compositions for FastSearch.

mod cli;
mod compatibility;
mod console;
pub mod fusion;
mod production;

pub use cli::{CliError, OutputFormat, execute_cli, execute_cli_formatted};
pub use compatibility::RealRuntime;
pub use console::{help_text, run_interactive, version_text};
pub use production::{ProductionConfig, ProductionRuntime};

#[cfg(test)]
mod real_runtime_recovery_tests {
    use std::{
        cell::Cell,
        fs,
        path::PathBuf,
        rc::Rc,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{RealRuntime, compatibility::RuntimeCoordinator};
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
