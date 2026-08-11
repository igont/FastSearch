//! In-memory adapters for the executable mock capability.

use std::collections::BTreeMap;

use crate::domain::{
    CanonicalRecord, Capability, ErrorKind, FastSearchError, LifecycleStatus, RelatedQuery,
    RetrievalChannel, SearchHit, SearchQuery, SearchResponse, SourceSnapshot, StableId,
};
use crate::ports::{
    CodeMapPort, LexicalRetrieval, SourcePort, StateStore, SymbolPort, VectorRetrieval,
};

pub struct MockSource {
    record: CanonicalRecord,
}

impl MockSource {
    #[must_use]
    pub const fn new(record: CanonicalRecord) -> Self {
        Self { record }
    }
}

impl SourcePort for MockSource {
    fn records(&self) -> Result<Vec<CanonicalRecord>, FastSearchError> {
        Ok(vec![self.record.clone()])
    }
    fn snapshot(&self) -> Result<Vec<SourceSnapshot>, FastSearchError> {
        Ok(Vec::new())
    }
}

#[derive(Default)]
pub struct MockState {
    records: BTreeMap<StableId, CanonicalRecord>,
}

impl StateStore for MockState {
    fn get(&self, id: &StableId) -> Result<Option<CanonicalRecord>, FastSearchError> {
        Ok(self.records.get(id).cloned())
    }
    fn lifecycle_status(&self) -> LifecycleStatus {
        LifecycleStatus::not_configured("mock state")
    }
    fn apply_snapshot(
        &mut self,
        _snapshot: SourceSnapshot,
    ) -> Result<crate::ports::StateChangeSet, FastSearchError> {
        Err(FastSearchError::new(
            ErrorKind::StateFailure,
            "mock state does not apply source snapshots",
        ))
    }

    fn put(&mut self, record: CanonicalRecord) -> Result<(), FastSearchError> {
        self.records.insert(record.id().clone(), record);
        Ok(())
    }

    fn remove(&mut self, id: &StableId) -> Result<bool, FastSearchError> {
        Ok(self.records.remove(id).is_some())
    }
}

pub struct MockLexical {
    record: CanonicalRecord,
}

impl MockLexical {
    #[must_use]
    pub const fn new(record: CanonicalRecord) -> Self {
        Self { record }
    }
}

impl LexicalRetrieval for MockLexical {
    fn search(&self, query: &SearchQuery) -> Result<SearchResponse, FastSearchError> {
        Ok(exact_response(&self.record, query))
    }
    fn lifecycle_status(&self) -> LifecycleStatus {
        LifecycleStatus::not_configured("mock lexical")
    }
    fn apply_projection(
        &self,
        _records: &[CanonicalRecord],
        _state_generation: u64,
    ) -> Result<LifecycleStatus, FastSearchError> {
        Ok(LifecycleStatus::not_configured("mock lexical"))
    }
    fn rebuild(
        &self,
        _records: &[CanonicalRecord],
        _state_generation: u64,
    ) -> Result<LifecycleStatus, FastSearchError> {
        Ok(LifecycleStatus::not_configured("mock lexical"))
    }
}

pub struct UnavailableVector;

impl VectorRetrieval for UnavailableVector {
    fn search(&self, _query: &SearchQuery) -> Result<SearchResponse, FastSearchError> {
        Err(unavailable(
            Capability::VectorRetrieval,
            "mock runtime has no vector retrieval",
        ))
    }
}

pub struct UnavailableCodeMaps;

impl CodeMapPort for UnavailableCodeMaps {
    fn related_maps(&self, _query: &RelatedQuery) -> Result<Vec<CanonicalRecord>, FastSearchError> {
        Err(unavailable(
            Capability::CodeMaps,
            "mock runtime has no code maps",
        ))
    }
}

pub struct MockSymbols;

impl SymbolPort for MockSymbols {
    fn find_symbols(&self, _query: &SearchQuery) -> Result<Vec<CanonicalRecord>, FastSearchError> {
        Ok(Vec::new())
    }
}

#[must_use]
pub fn exact_response(record: &CanonicalRecord, query: &SearchQuery) -> SearchResponse {
    if query.text() == "missing" {
        SearchResponse::default()
    } else {
        SearchResponse::new(vec![SearchHit::new(
            record.clone(),
            RetrievalChannel::Exact,
            1.0,
        )])
    }
}

#[must_use]
pub fn unavailable(capability: Capability, message: &'static str) -> FastSearchError {
    FastSearchError::new(ErrorKind::CapabilityUnavailable { capability }, message)
}
