//! Application composition for the observable mock runtime.

use std::collections::BTreeMap;

use crate::adapters::mock::{
    MockLexical, MockSource, MockState, MockSymbols, UnavailableCodeMaps, UnavailableVector,
};
use crate::domain::{
    BackendKind, CanonicalRecord, Capability, CapabilityStatus, ContentHash, FastSearchError,
    LifecycleStatus, RecordKind, RelatedQuery, SearchQuery, SearchResponse, SourceLocator,
    StableId,
};
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
