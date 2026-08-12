use std::{collections::BTreeMap, path::PathBuf};

use fastsearch::{
    adapters::vector::LocalE5Vector,
    domain::{
        CanonicalRecord, ContentHash, ErrorKind, IndexFreshness, RecordKind, SearchMode,
        SearchQuery, SourceLocator, StableId,
    },
    ports::VectorRetrieval,
};

fn record(id: &str, hash: &str, text: &str) -> CanonicalRecord {
    CanonicalRecord::new(
        StableId::parse(id).unwrap(),
        RecordKind::MarkdownSection,
        SourceLocator::markdown("guide.md", [id]).unwrap(),
        id,
        text,
        BTreeMap::new(),
        Vec::new(),
        ContentHash::parse(hash).unwrap(),
    )
    .unwrap()
}

#[test]
fn missing_local_provider_is_typed_and_never_current() {
    let adapter = LocalE5Vector::open(PathBuf::from("missing-local-e5"), "e5-test");
    let error = adapter
        .apply(&[record("one", "v1", "semantic navigation")], 1)
        .unwrap_err();
    assert_eq!(
        error.kind(),
        &ErrorKind::CapabilityUnavailable {
            capability: fastsearch::domain::Capability::VectorRetrieval
        }
    );
    assert_ne!(
        adapter.lifecycle_status().freshness(),
        IndexFreshness::Current
    );
}

#[test]
#[ignore = "requires FASTSEARCH_E5_MODEL_ROOT local-only cache"]
fn local_e5_lifecycle_invalidates_and_recovers_deterministically() {
    let root = PathBuf::from(std::env::var("FASTSEARCH_E5_MODEL_ROOT").unwrap());
    let adapter = LocalE5Vector::open(root.clone(), "multilingual-e5-small@614241f");
    let initial = vec![
        record(
            "architecture",
            "content-v1",
            "semantic navigation optional provider fallback",
        ),
        record("current", "content-v1", "real search implementation"),
    ];
    assert_eq!(
        adapter.apply(&initial, 1).unwrap().freshness(),
        IndexFreshness::Current
    );
    assert_eq!(
        adapter.provenance().model_identity(),
        "multilingual-e5-small@614241f"
    );
    assert_eq!(adapter.provenance().projection_generation(), Some(1));
    let query = SearchQuery::new(
        "semantic navigation optional provider fallback",
        SearchMode::Balanced,
    )
    .unwrap();
    let baseline = adapter.search(&query).unwrap();
    assert_eq!(
        baseline.hits().first().unwrap().record().id().as_str(),
        "architecture"
    );
    assert_eq!(
        baseline.hits().first().unwrap().channel(),
        fastsearch::domain::RetrievalChannel::Vector
    );
    assert!(baseline.hits().iter().all(|hit| hit.score().is_finite()));
    for _ in 0..5 {
        assert_eq!(adapter.search(&query).unwrap().hits(), baseline.hits());
    }

    let changed = vec![record(
        "architecture",
        "content-v2",
        "semantic navigation updated",
    )];
    assert_eq!(
        adapter.apply(&changed, 2).unwrap().freshness(),
        IndexFreshness::Current
    );
    assert_eq!(adapter.search(&query).unwrap().hits().len(), 1);
    let reopened = LocalE5Vector::open(root.clone(), "multilingual-e5-small@614241f");
    assert_eq!(
        reopened.apply(&changed, 2).unwrap().freshness(),
        IndexFreshness::Current
    );
    assert_eq!(
        reopened.search(&query).unwrap().hits(),
        adapter.search(&query).unwrap().hits()
    );
    adapter
        .reconfigure(root.clone(), "multilingual-e5-small@different-model")
        .unwrap();
    assert_ne!(
        adapter.lifecycle_status().freshness(),
        IndexFreshness::Current
    );
    assert_eq!(
        adapter.rebuild(&changed, 3).unwrap().freshness(),
        IndexFreshness::Current
    );
    adapter
        .reconfigure("missing-local-e5", "missing-model")
        .unwrap();
    let error = adapter.rebuild(&changed, 4).unwrap_err();
    assert_eq!(
        error.kind(),
        &ErrorKind::CapabilityUnavailable {
            capability: fastsearch::domain::Capability::VectorRetrieval
        }
    );
    assert_eq!(
        adapter.lifecycle_status().freshness(),
        IndexFreshness::Degraded
    );
    adapter
        .reconfigure(root, "multilingual-e5-small@614241f")
        .unwrap();
    assert_eq!(
        adapter.rebuild(&changed, 5).unwrap().freshness(),
        IndexFreshness::Current
    );
}
