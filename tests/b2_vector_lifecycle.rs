use std::{collections::BTreeMap, fs, path::PathBuf};

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

struct ExactTempFile(PathBuf);

impl Drop for ExactTempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
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
    let port: &dyn VectorRetrieval = &adapter;
    let port_response = port.search(&query).unwrap();
    let provenance = port_response.hits()[0].projection_provenance().unwrap();
    assert_eq!(provenance.model_identity().model(), "multilingual-e5-small");
    assert_eq!(provenance.model_identity().upstream_revision(), "614241f");
    assert_eq!(
        provenance.model_identity().artifact_manifest_sha256().len(),
        64
    );
    assert_eq!(provenance.authoritative_state_generation(), 1);
    assert_eq!(provenance.derived_projection_generation(), 1);
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
    let mutation = ExactTempFile(root.join("b2-model-bytes-mutation.tmp"));
    fs::write(&mutation.0, b"mutation").unwrap();
    let stale = port.search(&query).unwrap();
    assert!(stale.hits().is_empty());
    assert!(matches!(
        stale.freshness(),
        IndexFreshness::Stale | IndexFreshness::Degraded
    ));
    assert_ne!(
        adapter.lifecycle_status().freshness(),
        IndexFreshness::Current
    );
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
    assert_eq!(adapter.lifecycle_status().state_generation(), 4);
    fs::remove_file(&mutation.0).unwrap();
    adapter
        .reconfigure(root, "multilingual-e5-small@614241f")
        .unwrap();
    assert_eq!(
        adapter.rebuild(&changed, 5).unwrap().freshness(),
        IndexFreshness::Current
    );
}
