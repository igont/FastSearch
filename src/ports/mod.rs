//! Заменяемые границы core; реализации принадлежат adapter-веткам.

use crate::domain::{
    CanonicalRecord, CapabilityStatus, FastSearchError, RelatedQuery, SearchQuery, SearchResponse,
    StableId,
};

/// Поставляет нормализованные записи из источников.
pub trait SourcePort {
    fn records(&self) -> Result<Vec<CanonicalRecord>, FastSearchError>;
}

/// Хранит производное состояние и lifecycle канонических записей.
pub trait StateStore {
    fn get(&self, id: &StableId) -> Result<Option<CanonicalRecord>, FastSearchError>;
    fn put(&mut self, record: CanonicalRecord) -> Result<(), FastSearchError>;
    fn remove(&mut self, id: &StableId) -> Result<bool, FastSearchError>;
}

/// Выполняет exact/lexical retrieval без привязки к конкретному индексу.
pub trait LexicalRetrieval {
    fn search(&self, query: &SearchQuery) -> Result<SearchResponse, FastSearchError>;
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
}
