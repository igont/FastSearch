mod support;

use fastsearch::application::MockRuntime;
use fastsearch::domain::{CanonicalRecord, Capability, ErrorKind, RelatedQuery, SearchQuery};
use fastsearch::ports::{AgentSurface, LexicalRetrieval, SourcePort, StateStore};
use support::b::{assert_contract_oracle, golden::assert_golden_flow};

impl support::b::PortContractFixture for MockRuntime {
    fn source(&self) -> &dyn SourcePort {
        self.source_port()
    }

    fn state(&mut self) -> &mut dyn StateStore {
        self.state_store()
    }

    fn lexical(&self) -> &dyn LexicalRetrieval {
        self.lexical_retrieval()
    }

    fn agent(&self) -> &dyn AgentSurface {
        self.agent_surface()
    }

    fn expected_record(&self) -> CanonicalRecord {
        self.expected_record()
    }

    fn query(&self) -> SearchQuery {
        self.query()
    }
}

#[test]
fn runtime_mocks_obey_the_accepted_contract_oracle_and_goldens() {
    let mut runtime = MockRuntime::new();

    assert_contract_oracle(&mut runtime);
    assert_golden_flow(
        &runtime,
        include_str!("fixtures/reference-query.txt"),
        include_str!("fixtures/no-hit-query.txt"),
        include_str!("golden/happy-response.txt"),
        include_str!("golden/no-hit-response.txt"),
        include_str!("golden/unavailable-response.txt"),
        include_str!("golden/capability-status.txt"),
    );
}

#[test]
fn runtime_exposes_mock_and_unavailable_adapters_without_claiming_real_capability() {
    let runtime = MockRuntime::new();
    let query = runtime.query();

    assert!(runtime.symbols().find_symbols(&query).unwrap().is_empty());
    assert_eq!(
        runtime
            .vector_retrieval()
            .search(&query)
            .unwrap_err()
            .kind(),
        &ErrorKind::CapabilityUnavailable {
            capability: Capability::VectorRetrieval,
        }
    );
    assert_eq!(
        runtime
            .code_maps()
            .related_maps(&RelatedQuery::new(runtime.expected_record().id().clone()))
            .unwrap_err()
            .kind(),
        &ErrorKind::CapabilityUnavailable {
            capability: Capability::CodeMaps,
        }
    );
}
