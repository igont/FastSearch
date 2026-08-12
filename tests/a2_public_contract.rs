use fastsearch::domain::{
    BackendKind, CanonicalRecord, Capability, CapabilityStatus, ContentHash, FileHash,
    IndexFreshness, LifecycleStatus, LogicalRootId, RecordKind, RetrievalChannel,
    RootedSourceLocator, SearchHit, SearchResponse, SourceAdmission, SourceLocator, SourceSnapshot,
    StableId,
};
use fastsearch::ports::{LexicalRetrieval, SourcePort, StateChange, StateChangeSet, StateStore};
use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};

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

    let first = RootedSourceLocator::new(
        LogicalRootId::parse("documents").unwrap(),
        SourceLocator::markdown("guide.md", ["a/b"]).unwrap(),
    )
    .unwrap()
    .stable_id();
    let second = RootedSourceLocator::new(
        LogicalRootId::parse("documents").unwrap(),
        SourceLocator::markdown("guide.md", ["a", "b"]).unwrap(),
    )
    .unwrap()
    .stable_id();
    assert_ne!(
        first, second,
        "selector component boundaries must remain injective"
    );
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

#[test]
fn fused_result_preserves_provenance_and_is_deterministic_with_partial_capabilities() {
    let record = CanonicalRecord::new(
        StableId::parse("named-root-v1:documents:guide.md:file").unwrap(),
        RecordKind::MarkdownSection,
        SourceLocator::whole_file("guide.md").unwrap(),
        "Guide",
        "content",
        BTreeMap::new(),
        Vec::new(),
        ContentHash::parse("sha256:content").unwrap(),
    )
    .unwrap();
    let response = SearchResponse::fuse(
        vec![
            SearchHit::new(record.clone(), RetrievalChannel::Vector, 1.0),
            SearchHit::new(record, RetrievalChannel::Lexical, 1.0),
        ],
        vec![
            CapabilityStatus::available(Capability::LexicalRetrieval, BackendKind::Real),
            CapabilityStatus::unavailable(Capability::VectorRetrieval, "offline"),
        ],
    );
    assert_eq!(response.freshness(), IndexFreshness::Stale);
    assert_eq!(response.hits()[0].channel(), RetrievalChannel::Lexical);
    assert_eq!(response.hits()[1].channel(), RetrievalChannel::Vector);
}

#[test]
fn copied_dt2_state_is_stale_and_hides_legacy_records_until_rebuild() {
    let database = std::env::temp_dir().join(format!(
        "fastsearch-a2-{}.sqlite",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection.execute_batch(
        "CREATE TABLE state_records (id TEXT PRIMARY KEY, kind INTEGER NOT NULL, locator_path TEXT NOT NULL, selector_kind INTEGER NOT NULL, title TEXT NOT NULL, searchable_content TEXT NOT NULL, content_hash TEXT NOT NULL);
         INSERT INTO state_records VALUES ('legacy-id', 1, 'guide.md', 4, 'legacy', 'legacy', 'sha256:legacy');",
    ).unwrap();
    drop(connection);

    let store = fastsearch::adapters::state::SqliteStateStore::open(&database).unwrap();
    assert_eq!(store.lifecycle_status().freshness(), IndexFreshness::Stale);
    assert!(
        store
            .get(&StableId::parse("legacy-id").unwrap())
            .unwrap()
            .is_none()
    );
    drop(store);
    std::fs::remove_file(database).unwrap();
}
