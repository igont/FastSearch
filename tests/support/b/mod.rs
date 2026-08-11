use std::collections::BTreeMap;

use fastsearch::domain::{
    BackendKind, CanonicalRecord, Capability, CapabilityState, CapabilityStatus, ContentHash,
    ErrorKind, FastSearchError, RecordKind, RetrievalChannel, SearchHit, SearchMode, SearchQuery,
    SearchResponse, SourceLocator, StableId,
};
use fastsearch::ports::{AgentSurface, LexicalRetrieval, SourcePort, StateStore};

/// Минимальная тестовая граница, которую могут реализовать будущие runtime mocks.
pub trait PortContractFixture {
    fn source(&self) -> &dyn SourcePort;
    fn state(&mut self) -> &mut dyn StateStore;
    fn lexical(&self) -> &dyn LexicalRetrieval;
    fn agent(&self) -> &dyn AgentSurface;
    fn expected_record(&self) -> CanonicalRecord;
    fn query(&self) -> SearchQuery;
}

/// Проверяет общие наблюдаемые контракты source/state/lexical/agent без knowledge о реализации.
pub fn assert_contract_oracle(fixture: &mut impl PortContractFixture) {
    let expected = fixture.expected_record();
    let id = expected.id().clone();
    let query = fixture.query();

    assert_eq!(fixture.source().records().unwrap(), vec![expected.clone()]);

    assert_eq!(fixture.state().get(&id).unwrap(), None);
    fixture.state().put(expected.clone()).unwrap();
    assert_eq!(fixture.state().get(&id).unwrap(), Some(expected.clone()));
    assert!(fixture.state().remove(&id).unwrap());
    assert!(!fixture.state().remove(&id).unwrap());

    assert_exact_hit(fixture.lexical().search(&query).unwrap(), &expected);
    assert_exact_hit(fixture.agent().search(&query).unwrap(), &expected);
    assert_eq!(fixture.agent().get(&id).unwrap(), Some(expected));

    let related_error = fixture.agent().related(&related_query(&id)).unwrap_err();
    assert_eq!(
        related_error.kind(),
        &ErrorKind::CapabilityUnavailable {
            capability: Capability::CodeMaps,
        }
    );

    let statuses = fixture.agent().status();
    assert_available(&statuses, Capability::Source);
    assert_available(&statuses, Capability::State);
    assert_available(&statuses, Capability::LexicalRetrieval);
    let vector = statuses
        .iter()
        .find(|status| status.capability() == Capability::VectorRetrieval)
        .expect("reference fixture declares vector retrieval as unavailable");
    assert!(matches!(
        vector.state(),
        CapabilityState::Unavailable { .. }
    ));
    assert_eq!(
        vector.require_available().unwrap_err().kind(),
        &ErrorKind::CapabilityUnavailable {
            capability: Capability::VectorRetrieval,
        }
    );
}

fn assert_exact_hit(response: SearchResponse, expected: &CanonicalRecord) {
    assert_eq!(response.hits().len(), 1);
    let hit = &response.hits()[0];
    assert_eq!(hit.record(), expected);
    assert_eq!(hit.channel(), RetrievalChannel::Exact);
    assert_eq!(hit.score(), 1.0);
}

fn assert_available(statuses: &[CapabilityStatus], capability: Capability) {
    assert!(statuses.iter().any(|status| {
        status.capability() == capability
            && matches!(
                status.state(),
                CapabilityState::Available {
                    backend: BackendKind::Mock
                }
            )
    }));
}

fn related_query(id: &StableId) -> fastsearch::domain::RelatedQuery {
    fastsearch::domain::RelatedQuery::new(id.clone())
}

/// Эталонные doubles для B1; они намеренно не являются runtime adapter-реализациями.
pub struct ReferenceFixture {
    record: CanonicalRecord,
    query: SearchQuery,
    source: StaticSource,
    state: MemoryState,
    lexical: ExactLexical,
    agent: ReferenceAgent,
}

impl ReferenceFixture {
    #[must_use]
    pub fn new() -> Self {
        let record = reference_record();
        let query = SearchQuery::new("stable-id:guide", SearchMode::Balanced).unwrap();

        Self {
            source: StaticSource(record.clone()),
            state: MemoryState::default(),
            lexical: ExactLexical(record.clone()),
            agent: ReferenceAgent(record.clone()),
            record,
            query,
        }
    }
}

impl PortContractFixture for ReferenceFixture {
    fn source(&self) -> &dyn SourcePort {
        &self.source
    }

    fn state(&mut self) -> &mut dyn StateStore {
        &mut self.state
    }

    fn lexical(&self) -> &dyn LexicalRetrieval {
        &self.lexical
    }

    fn agent(&self) -> &dyn AgentSurface {
        &self.agent
    }

    fn expected_record(&self) -> CanonicalRecord {
        self.record.clone()
    }

    fn query(&self) -> SearchQuery {
        self.query.clone()
    }
}

struct StaticSource(CanonicalRecord);

impl SourcePort for StaticSource {
    fn records(&self) -> Result<Vec<CanonicalRecord>, FastSearchError> {
        Ok(vec![self.0.clone()])
    }
}

#[derive(Default)]
struct MemoryState(BTreeMap<StableId, CanonicalRecord>);

impl StateStore for MemoryState {
    fn get(&self, id: &StableId) -> Result<Option<CanonicalRecord>, FastSearchError> {
        Ok(self.0.get(id).cloned())
    }

    fn put(&mut self, record: CanonicalRecord) -> Result<(), FastSearchError> {
        self.0.insert(record.id().clone(), record);
        Ok(())
    }

    fn remove(&mut self, id: &StableId) -> Result<bool, FastSearchError> {
        Ok(self.0.remove(id).is_some())
    }
}

struct ExactLexical(CanonicalRecord);

impl LexicalRetrieval for ExactLexical {
    fn search(&self, _query: &SearchQuery) -> Result<SearchResponse, FastSearchError> {
        Ok(SearchResponse::new(vec![SearchHit::new(
            self.0.clone(),
            RetrievalChannel::Exact,
            1.0,
        )]))
    }
}

struct ReferenceAgent(CanonicalRecord);

impl AgentSurface for ReferenceAgent {
    fn search(&self, query: &SearchQuery) -> Result<SearchResponse, FastSearchError> {
        ExactLexical(self.0.clone()).search(query)
    }

    fn get(&self, id: &StableId) -> Result<Option<CanonicalRecord>, FastSearchError> {
        Ok((id == self.0.id()).then(|| self.0.clone()))
    }

    fn related(
        &self,
        _query: &fastsearch::domain::RelatedQuery,
    ) -> Result<Vec<CanonicalRecord>, FastSearchError> {
        Err(FastSearchError::new(
            ErrorKind::CapabilityUnavailable {
                capability: Capability::CodeMaps,
            },
            "reference fixture has no code-map capability",
        ))
    }

    fn status(&self) -> Vec<CapabilityStatus> {
        vec![
            CapabilityStatus::available(Capability::Source, BackendKind::Mock),
            CapabilityStatus::available(Capability::State, BackendKind::Mock),
            CapabilityStatus::available(Capability::LexicalRetrieval, BackendKind::Mock),
            CapabilityStatus::unavailable(
                Capability::VectorRetrieval,
                "reference fixture has no vector retrieval",
            ),
            CapabilityStatus::unavailable(
                Capability::CodeMaps,
                "reference fixture has no code maps",
            ),
        ]
    }
}

fn reference_record() -> CanonicalRecord {
    CanonicalRecord::new(
        StableId::parse("stable-id:guide").unwrap(),
        RecordKind::MarkdownSection,
        SourceLocator::markdown("fixtures/guide.md", ["Guide", "Search"]).unwrap(),
        "Search guide",
        "stable identifier lookup guide",
        BTreeMap::new(),
        Vec::new(),
        ContentHash::parse("fixture-hash-v1").unwrap(),
    )
    .unwrap()
}
