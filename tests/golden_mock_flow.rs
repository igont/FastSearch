mod support;

use support::b::{ReferenceFixture, assert_contract_oracle, golden::assert_golden_flow};

#[test]
fn synthetic_reference_flow_matches_happy_no_hit_and_unavailable_goldens() {
    let mut fixture = ReferenceFixture::new();
    assert_contract_oracle(&mut fixture);

    assert_golden_flow(
        &fixture,
        include_str!("fixtures/reference-query.txt"),
        include_str!("fixtures/no-hit-query.txt"),
        include_str!("golden/happy-response.txt"),
        include_str!("golden/no-hit-response.txt"),
        include_str!("golden/unavailable-response.txt"),
        include_str!("golden/capability-status.txt"),
    );
}

#[test]
fn golden_flow_accepts_windows_newlines_in_utf8_expectations() {
    let mut fixture = ReferenceFixture::new();
    assert_contract_oracle(&mut fixture);

    assert_golden_flow(
        &fixture,
        "stable-id:guide\r\n",
        "missing\r\n",
        "query=stable-id:guide\r\nhits=1\r\nchannel=Exact\r\nscore=1\r\nrecord=stable-id:guide\r\n",
        "query=missing\r\nhits=0\r\n",
        "capability=CodeMaps\r\nerror=CapabilityUnavailable\r\n",
        "Source=Mock\r\nState=Mock\r\nLexicalRetrieval=Mock\r\nVectorRetrieval=Unavailable\r\nCodeMaps=Unavailable\r\n",
    );
}
