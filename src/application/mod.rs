//! Application composition for mock compatibility and the real document runtime.

use std::{collections::BTreeMap, fs, path::Path};

use crate::adapters::mock::{
    MockLexical, MockSource, MockState, MockSymbols, UnavailableCodeMaps, UnavailableVector,
};
use crate::adapters::{lexical::TantivyLexical, source::FilesystemSource, state::SqliteStateStore};
use crate::domain::{
    BackendKind, CanonicalRecord, Capability, CapabilityStatus, ContentHash, ErrorKind,
    FastSearchError, IndexFreshness, LifecycleStatus, RecordKind, RelatedQuery, SearchQuery,
    SearchResponse, SourceLocator, StableId,
};

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
use crate::ports::{
    AgentSurface, CodeMapPort, LexicalRetrieval, SourcePort, StateStore, SymbolPort,
    VectorRetrieval,
};

pub struct MockRuntime {
    record: CanonicalRecord,
    query: SearchQuery,
    source: MockSource,
    state: MockState,
    lexical: MockLexical,
    vector: UnavailableVector,
    code_maps: UnavailableCodeMaps,
    symbols: MockSymbols,
}

impl MockRuntime {
    #[must_use]
    pub fn new() -> Self {
        let record = fixture_record();
        let query = SearchQuery::new("stable-id:guide", Default::default())
            .expect("constant mock query is valid");

        Self {
            source: MockSource::new(record.clone()),
            state: MockState::default(),
            lexical: MockLexical::new(record.clone()),
            record,
            query,
            vector: UnavailableVector,
            code_maps: UnavailableCodeMaps,
            symbols: MockSymbols,
        }
    }

    #[must_use]
    pub const fn source_port(&self) -> &dyn SourcePort {
        &self.source
    }

    #[must_use]
    pub fn state_store(&mut self) -> &mut dyn StateStore {
        &mut self.state
    }

    #[must_use]
    pub const fn lexical_retrieval(&self) -> &dyn LexicalRetrieval {
        &self.lexical
    }

    #[must_use]
    pub const fn vector_retrieval(&self) -> &dyn VectorRetrieval {
        &self.vector
    }

    #[must_use]
    pub const fn code_maps(&self) -> &dyn CodeMapPort {
        &self.code_maps
    }

    #[must_use]
    pub const fn symbols(&self) -> &dyn SymbolPort {
        &self.symbols
    }

    #[must_use]
    pub const fn agent_surface(&self) -> &dyn AgentSurface {
        self
    }

    #[must_use]
    pub fn expected_record(&self) -> CanonicalRecord {
        self.record.clone()
    }

    #[must_use]
    pub fn query(&self) -> SearchQuery {
        self.query.clone()
    }
}

impl Default for MockRuntime {
    fn default() -> Self {
        Self::new()
    }
}

/// Protocol-independent facade for every mock-facing surface.
pub struct MockFacade {
    runtime: MockRuntime,
}

impl MockFacade {
    #[must_use]
    pub fn new() -> Self {
        Self {
            runtime: MockRuntime::new(),
        }
    }

    /// Executes the accepted mock search flow and renders its observable result.
    pub fn render_mock_search(&self, text: &str) -> Result<String, FastSearchError> {
        let query = SearchQuery::new(text, Default::default())?;
        let response = self.search(&query)?;

        Ok(format!(
            "{}\n{}",
            render_response(&query, &response),
            render_status(&self.status())
        ))
    }

    #[must_use]
    pub const fn source_port(&self) -> &dyn SourcePort {
        self.runtime.source_port()
    }

    #[must_use]
    pub fn state_store(&mut self) -> &mut dyn StateStore {
        self.runtime.state_store()
    }

    #[must_use]
    pub const fn lexical_retrieval(&self) -> &dyn LexicalRetrieval {
        self.runtime.lexical_retrieval()
    }

    #[must_use]
    pub fn expected_record(&self) -> CanonicalRecord {
        self.runtime.expected_record()
    }

    #[must_use]
    pub fn query(&self) -> SearchQuery {
        self.runtime.query()
    }
}

impl Default for MockFacade {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentSurface for MockFacade {
    fn search(&self, query: &SearchQuery) -> Result<SearchResponse, FastSearchError> {
        self.runtime.search(query)
    }

    fn get(&self, id: &StableId) -> Result<Option<CanonicalRecord>, FastSearchError> {
        self.runtime.get(id)
    }

    fn related(&self, query: &RelatedQuery) -> Result<Vec<CanonicalRecord>, FastSearchError> {
        self.runtime.related(query)
    }

    fn status(&self) -> Vec<CapabilityStatus> {
        self.runtime.status()
    }
    fn index_status(&self) -> LifecycleStatus {
        LifecycleStatus::not_configured("mock facade")
    }
}

impl AgentSurface for MockRuntime {
    fn search(&self, query: &SearchQuery) -> Result<SearchResponse, FastSearchError> {
        self.lexical.search(query)
    }

    fn get(&self, id: &StableId) -> Result<Option<CanonicalRecord>, FastSearchError> {
        Ok((id == self.record.id()).then(|| self.record.clone()))
    }

    fn related(&self, query: &RelatedQuery) -> Result<Vec<CanonicalRecord>, FastSearchError> {
        self.code_maps.related_maps(query)
    }

    fn status(&self) -> Vec<CapabilityStatus> {
        vec![
            CapabilityStatus::available(Capability::Source, BackendKind::Mock),
            CapabilityStatus::available(Capability::State, BackendKind::Mock),
            CapabilityStatus::available(Capability::LexicalRetrieval, BackendKind::Mock),
            CapabilityStatus::unavailable(
                Capability::VectorRetrieval,
                "mock runtime has no vector retrieval",
            ),
            CapabilityStatus::unavailable(Capability::CodeMaps, "mock runtime has no code maps"),
        ]
    }
    fn index_status(&self) -> LifecycleStatus {
        LifecycleStatus::not_configured("mock runtime")
    }
}

fn fixture_record() -> CanonicalRecord {
    CanonicalRecord::new(
        StableId::parse("stable-id:guide").expect("constant mock id is valid"),
        RecordKind::MarkdownSection,
        SourceLocator::markdown("fixtures/guide.md", ["Guide", "Search"])
            .expect("constant mock locator is valid"),
        "Search guide",
        "stable identifier lookup guide",
        BTreeMap::new(),
        Vec::new(),
        ContentHash::parse("fixture-hash-v1").expect("constant mock hash is valid"),
    )
    .expect("constant mock record is valid")
}

fn render_response(query: &SearchQuery, response: &SearchResponse) -> String {
    let mut lines = vec![
        format!("query={}", query.text()),
        format!("hits={}", response.hits().len()),
    ];

    if let Some(hit) = response.hits().first() {
        lines.push(format!("channel={:?}", hit.channel()));
        lines.push(format!("score={}", hit.score()));
        lines.push(format!("record={}", hit.record().id().as_str()));
    }

    lines.join("\n")
}

fn render_status(statuses: &[CapabilityStatus]) -> String {
    statuses
        .iter()
        .map(|status| match status.state() {
            crate::domain::CapabilityState::Available { backend } => {
                format!("{:?}={backend:?}", status.capability())
            }
            crate::domain::CapabilityState::Unavailable { .. } => {
                format!("{:?}=Unavailable", status.capability())
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

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
