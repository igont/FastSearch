mod support;

use support::b::{ReferenceFixture, assert_contract_oracle};

#[test]
fn reference_doubles_obey_post_spike_port_contracts() {
    let mut fixture = ReferenceFixture::new();

    assert_contract_oracle(&mut fixture);
}
