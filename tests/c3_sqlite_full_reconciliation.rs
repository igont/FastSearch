use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use fastsearch::adapters::state::SqliteStateStore;
use fastsearch::domain::{
    CanonicalRecord, ContentHash, ErrorKind, FileHash, IndexFreshness, RecordKind, SourceLocator,
    SourceSnapshot, StableId,
};
use fastsearch::ports::{StateChange, StateStore};
use rusqlite::Connection;

fn database_path(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("fastsearch-c3-{name}-{nonce}.sqlite"))
}

fn record(id: &str, hash: &str) -> CanonicalRecord {
    CanonicalRecord::new(
        StableId::parse(id).expect("valid stable id"),
        RecordKind::MarkdownSection,
        SourceLocator::markdown("docs/guide.md", [id]).expect("valid locator"),
        format!("title {id}"),
        format!("content {id}"),
        BTreeMap::new(),
        Vec::new(),
        ContentHash::parse(hash).expect("valid content hash"),
    )
    .expect("valid canonical record")
}

fn snapshot(path: &str, file_hash: &str, records: Vec<CanonicalRecord>) -> SourceSnapshot {
    SourceSnapshot::new(
        SourceLocator::whole_file(path).expect("valid locator"),
        FileHash::parse(file_hash).expect("valid file hash"),
        records,
    )
}

fn assert_record(store: &SqliteStateStore, id: &str, present: bool) {
    assert_eq!(
        store
            .get(&StableId::parse(id).expect("valid stable id"))
            .expect("state read")
            .is_some(),
        present
    );
}

#[test]
fn complete_scan_reconciles_every_source_and_empty_scan_removes_the_corpus() {
    let path = database_path("complete-scan");
    let first = record("guide:first", "record:first-v1");
    let second = record("reference:second", "record:second-v1");
    let changed_first = record("guide:first", "record:first-v2");
    let mut store = SqliteStateStore::open(&path).expect("open state");

    let initial = [
        snapshot("docs/guide.md", "file:guide-v1", vec![first.clone()]),
        snapshot(
            "docs/reference.md",
            "file:reference-v1",
            vec![second.clone()],
        ),
    ];
    assert_eq!(
        store
            .reconcile_snapshots(&initial)
            .expect("initial reconcile")
            .changes(),
        &[StateChange::Added, StateChange::Added]
    );
    assert_eq!(store.lifecycle_status().state_generation(), 1);

    drop(store);
    let mut store = SqliteStateStore::open(&path).expect("reopen durable state");
    assert_eq!(
        store
            .reconcile_snapshots(&initial)
            .expect("unchanged reconcile")
            .changes(),
        &[StateChange::Unchanged, StateChange::Unchanged]
    );
    assert_eq!(store.lifecycle_status().state_generation(), 1);

    assert_eq!(
        store
            .reconcile_snapshots(&[snapshot(
                "docs/guide.md",
                "file:guide-v2",
                vec![changed_first.clone()],
            )])
            .expect("changed complete reconcile")
            .changes(),
        &[StateChange::Changed, StateChange::Deleted]
    );
    assert_eq!(store.lifecycle_status().state_generation(), 2);
    assert_record(&store, "guide:first", true);
    assert_record(&store, "reference:second", false);

    assert_eq!(
        store
            .reconcile_snapshots(&[])
            .expect("empty complete reconcile")
            .changes(),
        &[StateChange::Deleted]
    );
    assert_eq!(store.lifecycle_status().state_generation(), 3);
    assert_eq!(store.lifecycle_status().freshness(), IndexFreshness::Stale);
    assert_record(&store, "guide:first", false);
    drop(store);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn incremental_reconciliation_preserves_unchanged_sources_and_removes_missing_ones() {
    let path = database_path("incremental-scan");
    let guide = snapshot(
        "docs/guide.md",
        "file:guide-v1",
        vec![record("guide", "guide-v1")],
    );
    let reference = snapshot(
        "docs/reference.md",
        "file:reference-v1",
        vec![record("reference", "reference-v1")],
    );
    let changed_reference = snapshot(
        "docs/reference.md",
        "file:reference-v2",
        vec![record("reference", "reference-v2")],
    );
    let mut store = SqliteStateStore::open(&path).expect("open state");
    store
        .reconcile_snapshots(&[guide.clone(), reference])
        .expect("initial full reconcile");

    let seen = [guide.storage_key(), changed_reference.storage_key()]
        .into_iter()
        .collect::<BTreeSet<_>>();
    let changes = store
        .reconcile_incremental(&[changed_reference], &seen, &BTreeSet::new())
        .expect("one changed source applies without replacing the whole corpus");
    assert_eq!(changes.changes(), &[StateChange::Changed]);
    assert_record(&store, "guide", true);
    assert_record(&store, "reference", true);

    let reference_key = store
        .source_hashes()
        .expect("read source hashes")
        .keys()
        .find(|key| key.contains("reference.md"))
        .cloned()
        .expect("reference snapshot is durable");
    let changes = store
        .reconcile_incremental(
            &[],
            &[reference_key].into_iter().collect(),
            &BTreeSet::new(),
        )
        .expect("missing guide source is removed from durable state");
    assert_eq!(changes.changes(), &[StateChange::Deleted]);
    assert_record(&store, "guide", false);
    assert_record(&store, "reference", true);
    drop(store);
    fs::remove_file(path).expect("remove database");
}

#[test]
fn complete_scan_validates_all_snapshots_before_mutating_and_rolls_back_as_one_transaction() {
    let path = database_path("atomic");
    let accepted = record("guide:accepted", "record:accepted-v1");
    let retained = snapshot("docs/guide.md", "file:guide-v1", vec![accepted.clone()]);
    let mut store = SqliteStateStore::open(&path).expect("open state");
    store
        .reconcile_snapshots(std::slice::from_ref(&retained))
        .expect("accepted reconcile");

    let duplicate_locator = store
        .reconcile_snapshots(&[
            retained.clone(),
            snapshot("docs/guide.md", "file:guide-v2", Vec::new()),
        ])
        .expect_err("duplicate source locator must fail before mutation");
    assert_eq!(duplicate_locator.kind(), &ErrorKind::StateFailure);

    let duplicate_id = store
        .reconcile_snapshots(&[
            retained.clone(),
            snapshot(
                "docs/reference.md",
                "file:reference-v1",
                vec![record("guide:accepted", "record:other-v1")],
            ),
        ])
        .expect_err("cross-source stable id must fail before mutation");
    assert_eq!(duplicate_id.kind(), &ErrorKind::DuplicateStableId);
    assert_record(&store, "guide:accepted", true);
    assert_eq!(store.lifecycle_status().state_generation(), 1);

    drop(store);
    Connection::open(&path)
        .expect("open trigger connection")
        .execute_batch(
            "CREATE TRIGGER reject_second_source BEFORE INSERT ON state_source_memberships\n             WHEN NEW.record_id = 'reference:replacement'\n             BEGIN SELECT RAISE(ABORT, 'forced aggregate reconciliation failure'); END;",
        )
        .expect("create trigger");
    let mut store = SqliteStateStore::open(&path).expect("reopen state");
    let error = store
        .reconcile_snapshots(&[
            snapshot(
                "docs/guide.md",
                "file:guide-v2",
                vec![record("guide:replacement", "record:replacement-v1")],
            ),
            snapshot(
                "docs/reference.md",
                "file:reference-v1",
                vec![record("reference:replacement", "record:replacement-v1")],
            ),
        ])
        .expect_err("failure in one source must rollback complete scan");
    assert_eq!(error.kind(), &ErrorKind::StateFailure);
    assert_record(&store, "guide:accepted", true);
    assert_record(&store, "guide:replacement", false);
    assert_record(&store, "reference:replacement", false);
    assert_eq!(store.lifecycle_status().state_generation(), 1);
    assert_eq!(store.lifecycle_status().freshness(), IndexFreshness::Stale);
    let ledger = Connection::open(&path)
        .expect("open ledger connection")
        .query_row(
            "SELECT source_key, file_hash FROM state_source_snapshots",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .expect("accepted ledger remains durable");
    assert_eq!(
        ledger,
        ("13:docs/guide.mdF".to_owned(), "file:guide-v1".to_owned())
    );
    let membership = Connection::open(&path)
        .expect("open membership connection")
        .query_row(
            "SELECT source_key, record_id FROM state_source_memberships",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .expect("accepted membership remains durable");
    assert_eq!(
        membership,
        ("13:docs/guide.mdF".to_owned(), "guide:accepted".to_owned())
    );
    drop(store);
    fs::remove_file(path).expect("remove database");
}
