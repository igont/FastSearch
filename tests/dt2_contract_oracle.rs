mod support;

use fastsearch::domain::BackendKind;
use support::b::{ReferenceFixture, assert_contract_oracle_for_backend};

#[test]
fn oracle_requires_the_explicit_expected_backend() {
    let mut mock = ReferenceFixture::new();
    assert_contract_oracle_for_backend(&mut mock, BackendKind::Mock);

    let real_mismatch = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assert_contract_oracle_for_backend(&mut mock, BackendKind::Real);
    }));
    assert!(
        real_mismatch.is_err(),
        "Mock fixture must not satisfy a Real expectation"
    );
}

#[test]
fn dt2_corpus_is_a_complete_adapter_neutral_regression_oracle() {
    let outcomes = include_str!("golden/dt2/expected-outcomes.md");
    let passport = include_str!("golden/dt2/boundary-passport.md");
    let current_markdown = include_str!("fixtures/dt2/guide-current.md");
    let design_markdown = include_str!("fixtures/dt2/guide-design.md");
    let registry = include_str!("fixtures/dt2/registry.tsv");
    let excluded = include_str!("fixtures/dt2/excluded.tmp");

    assert!(current_markdown.contains("alignment: CURRENT"));
    assert!(current_markdown.contains("документальный поиск"));
    assert!(design_markdown.contains("alignment: DESIGN"));
    assert!(registry.contains("2433\tТехническая запись"));
    assert!(excluded.contains("not an indexable source"));

    for required in [
        "guide-current",
        "guide-design",
        "registry-2433",
        "русская фраза",
        "add",
        "unchanged",
        "change",
        "delete",
        "Exact",
        "Current",
        "Design",
        "B",
        "C",
        "D",
        "E",
    ] {
        assert!(outcomes.contains(required), "missing outcome: {required}");
    }

    for required in [
        "StableId = source kind + normalized repo-relative locator + record-local component",
        "file hash",
        "record hash",
        "current",
        "stale",
        "degraded",
        "SQLite",
        "Tantivy",
        "search",
        "status",
        "update",
        "rebuild",
        "tie-break",
        "lexicographic StableId",
        "adapter-only",
        "public contract",
    ] {
        assert!(
            passport.contains(required),
            "missing boundary decision: {required}"
        );
    }
}
