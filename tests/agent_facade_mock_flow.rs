mod support;

use fastsearch::application::MockFacade;
use fastsearch::domain::{CanonicalRecord, SearchQuery};
use fastsearch::ports::{AgentSurface, LexicalRetrieval, SourcePort, StateStore};
use support::b::{assert_contract_oracle, golden::assert_golden_flow};

impl support::b::PortContractFixture for MockFacade {
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
        self
    }

    fn expected_record(&self) -> CanonicalRecord {
        self.expected_record()
    }

    fn query(&self) -> SearchQuery {
        self.query()
    }
}

#[test]
fn facade_delegates_post_spike_oracle_and_goldens_to_one_mock_runtime() {
    let mut facade = MockFacade::new();

    assert_contract_oracle(&mut facade);
    assert_golden_flow(
        &facade,
        include_str!("fixtures/reference-query.txt"),
        include_str!("fixtures/no-hit-query.txt"),
        include_str!("golden/happy-response.txt"),
        include_str!("golden/no-hit-response.txt"),
        include_str!("golden/unavailable-response.txt"),
        include_str!("golden/capability-status.txt"),
    );
}
