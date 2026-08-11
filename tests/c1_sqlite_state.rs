use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use fastsearch::adapters::state::SqliteStateStore;
use fastsearch::domain::{
    CanonicalRecord, ContentHash, ErrorKind, IndexFreshness, RecordKind, SourceLocator, StableId,
};
use fastsearch::ports::StateStore;
use rusqlite::Connection;

fn database_path(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("fastsearch-c1-{name}-{nonce}.sqlite"))
}

fn record(id: &str, heading: &[&str], metadata_value: &str, relations: &[&str]) -> CanonicalRecord {
    let metadata = BTreeMap::from([("owner".to_owned(), metadata_value.to_owned())]);
    CanonicalRecord::new(
        StableId::parse(id).unwrap(),
        RecordKind::MarkdownSection,
        SourceLocator::markdown("docs/guide.md", heading.iter().copied()).unwrap(),
        "Guide title",
        "Searchable guide content",
        metadata,
        relations
            .iter()
            .map(|relation| StableId::parse(*relation).unwrap())
            .collect(),
        ContentHash::parse(format!("hash-{metadata_value}")).unwrap(),
    )
    .unwrap()
}

#[test]
fn reopens_complete_record_and_replaces_ordered_children_for_the_same_id() {
    let path = database_path("reopen");
    let id = StableId::parse("record-1").unwrap();

    {
        let mut store = SqliteStateStore::open(&path).unwrap();
        store
            .put(record(
                "record-1",
                &["Guide", "Initial"],
                "first",
                &["rel-a", "rel-b"],
            ))
            .unwrap();
        store
            .put(record(
                "record-1",
                &["Guide", "Replacement"],
                "second",
                &["rel-c"],
            ))
            .unwrap();
        assert_eq!(store.lifecycle_status().freshness(), IndexFreshness::Stale);
        assert_eq!(store.lifecycle_status().state_generation(), 2);
        assert_eq!(store.lifecycle_status().projection_generation(), None);
    }

    let store = SqliteStateStore::open(&path).unwrap();
    let expected = record("record-1", &["Guide", "Replacement"], "second", &["rel-c"]);
    assert_eq!(store.get(&id).unwrap(), Some(expected));
    assert_eq!(store.lifecycle_status().state_generation(), 2);
    drop(store);
    fs::remove_file(path).unwrap();
}

#[test]
fn failed_transaction_preserves_the_previous_record_and_generation() {
    let path = database_path("rollback");
    let id = StableId::parse("record-2").unwrap();
    let sentinel = record("record-2", &["Guide", "Sentinel"], "sentinel", &["rel-a"]);
    let replacement = record(
        "record-2",
        &["Guide", "Replacement"],
        "replacement",
        &["rel-b"],
    );

    {
        let mut store = SqliteStateStore::open(&path).unwrap();
        store.put(sentinel.clone()).unwrap();
    }
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER reject_replacement BEFORE INSERT ON state_metadata\n             WHEN NEW.value = 'replacement'\n             BEGIN SELECT RAISE(ABORT, 'forced metadata failure'); END;",
        )
        .unwrap();

    let mut store = SqliteStateStore::open(&path).unwrap();
    let error = store.put(replacement).unwrap_err();
    assert_eq!(error.kind(), &ErrorKind::StateFailure);
    assert_eq!(store.get(&id).unwrap(), Some(sentinel));
    assert_eq!(store.lifecycle_status().state_generation(), 1);
    drop(store);
    fs::remove_file(path).unwrap();
}

#[test]
fn duplicate_input_ids_are_rejected_before_mutating_the_existing_record() {
    let path = database_path("duplicate");
    let id = StableId::parse("record-3").unwrap();
    let original = record("record-3", &["Guide", "Original"], "original", &[]);
    let mut store = SqliteStateStore::open(&path).unwrap();
    store.put(original.clone()).unwrap();

    let error = store
        .put_all(vec![
            record("record-3", &["Guide", "Duplicate"], "duplicate", &[]),
            record("record-3", &["Guide", "Duplicate 2"], "duplicate-2", &[]),
        ])
        .unwrap_err();
    assert_eq!(error.kind(), &ErrorKind::DuplicateStableId);
    assert_eq!(store.get(&id).unwrap(), Some(original));
    drop(store);
    fs::remove_file(path).unwrap();
}
