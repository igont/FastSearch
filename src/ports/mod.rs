//! Заменяемые границы core; реализации принадлежат adapter-веткам.

use crate::domain::{
    CanonicalRecord, CapabilityStatus, FastSearchError, LifecycleStatus, RelatedQuery, SearchQuery,
    SearchResponse, SourceSnapshot, StableId,
};

/// Поставляет нормализованные записи из источников.
pub trait SourcePort {
    fn records(&self) -> Result<Vec<CanonicalRecord>, FastSearchError>;
    fn snapshot(&self) -> Result<Vec<SourceSnapshot>, FastSearchError> {
        Ok(Vec::new())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StateChange {
    Added,
    Unchanged,
    Changed,
    Deleted,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateChangeSet {
    changes: Vec<StateChange>,
    durable_generation: u64,
}
impl StateChangeSet {
    #[must_use]
    pub const fn new(changes: Vec<StateChange>, durable_generation: u64) -> Self {
        Self {
            changes,
            durable_generation,
        }
    }
    #[must_use]
    pub fn changes(&self) -> &[StateChange] {
        &self.changes
    }
    #[must_use]
    pub const fn durable_generation(&self) -> u64 {
        self.durable_generation
    }
}
impl StateChange {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Added => "add",
            Self::Unchanged => "unchanged",
            Self::Changed => "change",
            Self::Deleted => "delete",
        }
    }
}

/// Хранит производное состояние и lifecycle канонических записей.
pub trait StateStore {
    fn get(&self, id: &StableId) -> Result<Option<CanonicalRecord>, FastSearchError>;
    fn put(&mut self, record: CanonicalRecord) -> Result<(), FastSearchError>;
    fn remove(&mut self, id: &StableId) -> Result<bool, FastSearchError>;
    fn lifecycle_status(&self) -> LifecycleStatus {
        LifecycleStatus::not_configured("adapter does not expose lifecycle")
    }
    fn apply_snapshot(
        &mut self,
        _snapshot: SourceSnapshot,
    ) -> Result<StateChangeSet, FastSearchError> {
        Err(FastSearchError::new(
            crate::domain::ErrorKind::StateFailure,
            "state adapter does not apply snapshots",
        ))
    }
    /// Atomically reconciles the complete set of successfully scanned sources.
    fn reconcile_snapshots(
        &mut self,
        _snapshots: &[SourceSnapshot],
    ) -> Result<StateChangeSet, FastSearchError> {
        Err(FastSearchError::new(
            crate::domain::ErrorKind::StateFailure,
            "state adapter does not reconcile source snapshots",
        ))
    }
}

/// Выполняет exact/lexical retrieval без привязки к конкретному индексу.
pub trait LexicalRetrieval {
    fn search(&self, query: &SearchQuery) -> Result<SearchResponse, FastSearchError>;
    fn lifecycle_status(&self) -> LifecycleStatus {
        LifecycleStatus::not_configured("adapter does not expose lifecycle")
    }
    fn apply_projection(
        &self,
        _records: &[CanonicalRecord],
        _state_generation: u64,
    ) -> Result<LifecycleStatus, FastSearchError> {
        Err(FastSearchError::new(
            crate::domain::ErrorKind::ProjectionFailure,
            "lexical adapter does not apply projections",
        ))
    }
    fn rebuild(
        &self,
        _records: &[CanonicalRecord],
        _state_generation: u64,
    ) -> Result<LifecycleStatus, FastSearchError> {
        Err(FastSearchError::new(
            crate::domain::ErrorKind::ProjectionFailure,
            "lexical adapter does not rebuild projections",
        ))
    }
}

/// Выполняет optional vector retrieval без привязки к provider.
pub trait VectorRetrieval {
    fn search(&self, query: &SearchQuery) -> Result<SearchResponse, FastSearchError>;
}

/// Выдаёт ближайшие явные связи code maps.
pub trait CodeMapPort {
    fn related_maps(&self, query: &RelatedQuery) -> Result<Vec<CanonicalRecord>, FastSearchError>;
}

/// Выполняет точный поиск symbol cards.
pub trait SymbolPort {
    fn find_symbols(&self, query: &SearchQuery) -> Result<Vec<CanonicalRecord>, FastSearchError>;
}

/// Единая граница для будущих CLI и agent transports.
pub trait AgentSurface {
    fn search(&self, query: &SearchQuery) -> Result<SearchResponse, FastSearchError>;
    fn get(&self, id: &StableId) -> Result<Option<CanonicalRecord>, FastSearchError>;
    fn related(&self, query: &RelatedQuery) -> Result<Vec<CanonicalRecord>, FastSearchError>;
    fn status(&self) -> Vec<CapabilityStatus>;
    fn index_status(&self) -> LifecycleStatus {
        LifecycleStatus::not_configured("agent does not expose an index")
    }
}
