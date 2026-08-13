//! DT2 document-only compatibility runtime.

use std::{fs, path::Path};

use crate::{
    adapters::{lexical::TantivyLexical, source::FilesystemSource, state::SqliteStateStore},
    domain::{
        BackendKind, CanonicalRecord, Capability, CapabilityStatus, ErrorKind, FastSearchError,
        IndexFreshness, LifecycleStatus, RelatedQuery, SearchQuery, SearchResponse, StableId,
    },
    ports::{AgentSurface, LexicalRetrieval, SourcePort, StateChange, StateStore},
};

/// Coordinates the legacy document-only source with its authority and lexical projection.
pub(super) struct RuntimeCoordinator<S, T, L> {
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
    pub(super) fn new(source: S, state: T, lexical: L) -> Self {
        Self {
            source,
            state,
            lexical,
            projection_failure: None,
        }
    }

    pub(super) fn index(&mut self) -> Result<LifecycleStatus, FastSearchError> {
        self.project(false)
    }

    pub(super) fn index_with_test_projection_failure(
        &mut self,
    ) -> Result<LifecycleStatus, FastSearchError> {
        let snapshots = self.source.snapshot()?;
        self.state.reconcile_snapshots(&snapshots)?;
        let error = FastSearchError::new(
            ErrorKind::ProjectionFailure,
            "controlled lexical projection failure",
        );
        self.projection_failure = Some(error.message().to_owned());
        Err(error)
    }

    pub(super) fn rebuild(&mut self) -> Result<LifecycleStatus, FastSearchError> {
        self.project(true)
    }

    fn project(&mut self, rebuild: bool) -> Result<LifecycleStatus, FastSearchError> {
        let snapshots = self.source.snapshot()?;
        let records = snapshots
            .iter()
            .flat_map(|snapshot| snapshot.records().iter().cloned())
            .collect::<Vec<_>>();
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

    pub(super) fn status(&self) -> LifecycleStatus {
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

/// Production-compatible DT2 document-only runtime retained for legacy callers.
pub struct RealRuntime {
    coordinator: RuntimeCoordinator<FilesystemSource, SqliteStateStore, TantivyLexical>,
}

impl RealRuntime {
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

    pub fn index(&mut self) -> Result<LifecycleStatus, FastSearchError> {
        self.coordinator.index()
    }

    pub(super) fn index_with_test_projection_failure(
        &mut self,
    ) -> Result<LifecycleStatus, FastSearchError> {
        self.coordinator.index_with_test_projection_failure()
    }

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
