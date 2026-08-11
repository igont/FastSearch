use fastsearch::domain::{
    FileHash, IndexFreshness, LifecycleStatus, SearchResponse, SourceLocator, SourceSnapshot,
};
use fastsearch::ports::{LexicalRetrieval, SourcePort, StateChange, StateChangeSet, StateStore};
use fastsearch::{adapters::mock::MockLexical, domain::CanonicalRecord};

#[test]
fn public_ports_express_source_hashes_lifecycle_and_changes_without_adapter_handles() {
    let file_hash = FileHash::parse("sha256:file-v1").expect("file hash is valid");
    let status = LifecycleStatus::not_configured("mock adapter");
    assert_eq!(status.freshness(), IndexFreshness::NotConfigured);

    let snapshot = SourceSnapshot::new(
        SourceLocator::whole_file("fixtures/guide.md").expect("locator is valid"),
        file_hash,
        Vec::new(),
    );
    assert!(snapshot.records().is_empty());
    assert_eq!(StateChange::Added.as_str(), "add");
    assert_eq!(
        StateChangeSet::new(vec![StateChange::Changed], 7).durable_generation(),
        7
    );
    assert_eq!(
        SearchResponse::with_freshness(Vec::new(), IndexFreshness::Stale).freshness(),
        IndexFreshness::Stale
    );

    fn assert_object_safe(_: &dyn SourcePort, _: &mut dyn StateStore, _: &dyn LexicalRetrieval) {}
    let _ = assert_object_safe;
}

#[test]
fn mock_lexical_lifecycle_is_explicitly_not_configured() {
    let record = CanonicalRecord::new(
        fastsearch::domain::StableId::parse("mock:record").unwrap(),
        fastsearch::domain::RecordKind::MarkdownSection,
        SourceLocator::whole_file("mock.md").unwrap(),
        "mock",
        "mock",
        Default::default(),
        Vec::new(),
        fastsearch::domain::ContentHash::parse("mock-hash").unwrap(),
    )
    .unwrap();
    let lexical = MockLexical::new(record);
    assert_eq!(
        lexical.apply_projection(&[], 1).unwrap().freshness(),
        IndexFreshness::NotConfigured
    );
    assert_eq!(
        lexical.rebuild(&[], 1).unwrap().freshness(),
        IndexFreshness::NotConfigured
    );
}
