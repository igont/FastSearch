mod support;

use support::b::{ReferenceFixture, golden::assert_golden_flow};

#[test]
fn synthetic_reference_flow_matches_happy_no_hit_and_unavailable_goldens() {
    assert_golden_flow(
        &ReferenceFixture::new(),
        include_str!("fixtures/reference-query.txt"),
        include_str!("fixtures/no-hit-query.txt"),
        include_str!("golden/happy-response.txt"),
        include_str!("golden/no-hit-response.txt"),
        include_str!("golden/unavailable-response.txt"),
        include_str!("golden/capability-status.txt"),
    );
}
