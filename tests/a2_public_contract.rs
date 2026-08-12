use fastsearch::domain::{
    FileHash, IndexFreshness, LifecycleStatus, LogicalRootId, RootedSourceLocator, SearchResponse,
    SourceAdmission, SourceLocator, SourceSnapshot, StableId,
};
use fastsearch::ports::{LexicalRetrieval, SourcePort, StateChange, StateChangeSet, StateStore};

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
fn named_roots_make_stable_ids_collision_free_without_absolute_paths() {
    let documents = LogicalRootId::parse("documents").expect("logical root");
    let code = LogicalRootId::parse("code-fastsearch").expect("logical root");
    let locator = SourceLocator::whole_file("guide/readme.md").expect("relative locator");

    let document_id = RootedSourceLocator::new(documents, locator.clone())
        .expect("rooted locator")
        .stable_id();
    let code_id = RootedSourceLocator::new(code, locator)
        .expect("rooted locator")
        .stable_id();

    assert_ne!(document_id, code_id);
    assert!(document_id.as_str().starts_with("named-root-v1:"));
    assert!(SourceLocator::whole_file("C:/private/readme.md").is_err());
    assert!(SourceLocator::whole_file("../readme.md").is_err());
}

#[test]
fn source_admission_reserves_cfmap_for_its_single_owner() {
    assert_eq!(
        SourceAdmission::classify("notes.md"),
        SourceAdmission::Markdown
    );
    assert_eq!(
        SourceAdmission::classify("architecture.cfmap.md"),
        SourceAdmission::CodeMap
    );
    assert_eq!(
        SourceAdmission::classify("main.rs"),
        SourceAdmission::CodeCandidate
    );
    let _ = StableId::parse("named-root-v1:example").expect("versioned stable ID");
}
