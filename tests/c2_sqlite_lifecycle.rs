use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use fastsearch::adapters::state::SqliteStateStore;
use fastsearch::domain::{
    CanonicalRecord, ContentHash, ErrorKind, FileHash, RecordKind, SourceLocator, SourceSnapshot,
    StableId,
};
use fastsearch::ports::{StateChange, StateStore};
use rusqlite::Connection;

fn database_path(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("fastsearch-c2-{name}-{nonce}.sqlite"))
}

fn record(id: &str, content_hash: &str) -> CanonicalRecord {
    CanonicalRecord::new(
        StableId::parse(id).unwrap(),
        RecordKind::MarkdownSection,
        SourceLocator::markdown("docs/guide.md", [id]).unwrap(),
        format!("title {id}"),
        format!("content {id}"),
        BTreeMap::new(),
        Vec::new(),
        ContentHash::parse(content_hash).unwrap(),
    )
    .unwrap()
}

fn snapshot(path: &str, hash: &str, records: Vec<CanonicalRecord>) -> SourceSnapshot {
    SourceSnapshot::new(
        SourceLocator::whole_file(path).unwrap(),
        FileHash::parse(hash).unwrap(),
        records,
    )
}

fn assert_record(store: &SqliteStateStore, id: &str, expected: Option<CanonicalRecord>) {
    assert_eq!(store.get(&StableId::parse(id).unwrap()).unwrap(), expected);
}

#[test]
fn snapshot_lifecycle_classifies_add_unchanged_change_delete_and_rebuilds_equal_corpus() {
    let incremental_path = database_path("lifecycle");
    let clean_path = database_path("rebuild");
    let first = record("guide:first", "record:first-v1");
    let second = record("guide:second", "record:second-v1");
    let replacement = record("guide:first", "record:first-v2");

    let final_snapshot = snapshot("docs/guide.md", "file-v3", vec![replacement.clone()]);
    {
        let mut store = SqliteStateStore::open(&incremental_path).unwrap();
        assert_eq!(
            store
                .apply_snapshot(snapshot(
                    "docs/guide.md",
                    "file-v1",
                    vec![first.clone(), second.clone()],
                ))
                .unwrap()
                .changes(),
            &[StateChange::Added, StateChange::Added]
        );
        assert_eq!(store.lifecycle_status().state_generation(), 1);

        assert_eq!(
            store
                .apply_snapshot(snapshot(
                    "docs/guide.md",
                    "file-v1",
                    vec![first, second.clone()],
                ))
                .unwrap()
                .changes(),
            &[StateChange::Unchanged, StateChange::Unchanged]
        );
        assert_eq!(store.lifecycle_status().state_generation(), 1);

        assert_eq!(
            store
                .apply_snapshot(snapshot(
                    "docs/guide.md",
                    "file-v2",
                    vec![replacement.clone(), second],
                ))
                .unwrap()
                .changes(),
            &[StateChange::Changed, StateChange::Unchanged]
        );
        assert_eq!(store.lifecycle_status().state_generation(), 2);
        assert_eq!(
            store
                .apply_snapshot(final_snapshot.clone())
                .unwrap()
                .changes(),
            &[StateChange::Unchanged, StateChange::Deleted]
        );
        assert_eq!(store.lifecycle_status().state_generation(), 3);
    }

    let incremental = SqliteStateStore::open(&incremental_path).unwrap();
    let mut rebuilt = SqliteStateStore::open(&clean_path).unwrap();
    assert_eq!(
        rebuilt.apply_snapshot(final_snapshot).unwrap().changes(),
        &[StateChange::Added]
    );
    assert_record(&incremental, "guide:first", Some(replacement.clone()));
    assert_record(&rebuilt, "guide:first", Some(replacement));
    assert_record(&incremental, "guide:second", None);
    assert_record(&rebuilt, "guide:second", None);
    drop(incremental);
    drop(rebuilt);
    fs::remove_file(incremental_path).unwrap();
    fs::remove_file(clean_path).unwrap();
}

#[test]
fn duplicate_or_cross_source_ids_fail_before_mutating_the_accepted_snapshot() {
    let path = database_path("duplicate");
    let original = record("guide:owned", "record:owned-v1");
    let mut store = SqliteStateStore::open(&path).unwrap();
    store
        .apply_snapshot(snapshot("docs/guide.md", "file-v1", vec![original.clone()]))
        .unwrap();

    let duplicate = store
        .apply_snapshot(snapshot(
            "docs/guide.md",
            "file-v2",
            vec![
                record("guide:owned", "record:owned-v2"),
                record("guide:owned", "record:owned-v3"),
            ],
        ))
        .unwrap_err();
    assert_eq!(duplicate.kind(), &ErrorKind::DuplicateStableId);

    let cross_source = store
        .apply_snapshot(snapshot(
            "docs/other.md",
            "file-v1",
            vec![record("guide:owned", "record:other-v1")],
        ))
        .unwrap_err();
    assert_eq!(cross_source.kind(), &ErrorKind::DuplicateStableId);
    assert_record(&store, "guide:owned", Some(original));
    assert_eq!(store.lifecycle_status().state_generation(), 1);
    drop(store);
    fs::remove_file(path).unwrap();
}

#[test]
fn failed_snapshot_transaction_rolls_back_records_ledger_and_generation() {
    let path = database_path("rollback");
    let accepted = record("guide:accepted", "record:accepted-v1");
    {
        let mut store = SqliteStateStore::open(&path).unwrap();
        store
            .apply_snapshot(snapshot("docs/guide.md", "file-v1", vec![accepted.clone()]))
            .unwrap();
    }
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER reject_snapshot_membership BEFORE INSERT ON state_source_memberships\n             WHEN NEW.record_id = 'guide:replacement'\n             BEGIN SELECT RAISE(ABORT, 'forced snapshot membership failure'); END;",
        )
        .unwrap();

    let mut store = SqliteStateStore::open(&path).unwrap();
    let error = store
        .apply_snapshot(snapshot(
            "docs/guide.md",
            "file-v2",
            vec![record("guide:replacement", "record:replacement-v1")],
        ))
        .unwrap_err();
    assert_eq!(error.kind(), &ErrorKind::StateFailure);
    assert_record(&store, "guide:accepted", Some(accepted.clone()));
    assert_record(&store, "guide:replacement", None);
    assert_eq!(store.lifecycle_status().state_generation(), 1);
    assert_eq!(
        store
            .apply_snapshot(snapshot("docs/guide.md", "file-v1", vec![accepted]))
            .unwrap()
            .changes(),
        &[StateChange::Unchanged]
    );
    drop(store);
    fs::remove_file(path).unwrap();
}
