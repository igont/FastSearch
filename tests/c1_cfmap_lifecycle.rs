use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

use fastsearch::adapters::maps::CodeMapSource;
use fastsearch::adapters::state::SqliteStateStore;
use fastsearch::domain::{ErrorKind, RecordKind};
use fastsearch::ports::{SourcePort, StateChange, StateStore};

struct Fixture {
    root: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(0);
        let fixture_id = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("fastsearch-c1-{}-{fixture_id}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn write(&self, path: &str, content: &str) {
        let path = self.root.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn read(&self, path: &str) -> String {
        fs::read_to_string(self.root.join(path)).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

const AUTO: &str = "---\ncfmap: v1\nmode: AUTO\nsource: architecture.md#Navigation\ngeneration: 1\n---\n# Navigation\nDeclared AUTO facts.\n";
const CURATED: &str = "---\ncfmap: v1\nmode: CURATED\n---\n# Curated\nHuman-authored facts.\n";

#[test]
fn cfmap_v1_admits_exactly_one_map_snapshot_and_rejects_invalid_input_atomically() {
    let fixture = Fixture::new();
    fixture.write("architecture.md", "# Navigation\n");
    fixture.write("navigation.cfmap.md", AUTO);
    let source = CodeMapSource::new(&fixture.root);

    let snapshots = SourcePort::snapshot(&source).expect("valid v1 map must admit");
    assert_eq!(snapshots.len(), 1);
    let record = &snapshots[0].records()[0];
    assert_eq!(record.kind(), RecordKind::CodeMap);
    assert_eq!(record.locator().path(), "navigation.cfmap.md");
    assert_eq!(record.metadata().get("mode"), Some(&"AUTO".to_owned()));
    assert_eq!(record.metadata().get("state"), Some(&"CURRENT".to_owned()));

    fixture.write(
        "invalid.cfmap.md",
        "---\ncfmap: v1\nmode: UNKNOWN\n---\n# Invalid\n",
    );
    assert_eq!(
        source.snapshots().unwrap_err().kind(),
        &ErrorKind::InvalidContent
    );
}

#[test]
fn auto_derivation_is_read_only_idempotent_and_persists_only_through_sqlite_state() {
    let fixture = Fixture::new();
    fixture.write("architecture.md", "# Navigation\n");
    fixture.write("auto.cfmap.md", AUTO);
    fixture.write("curated.cfmap.md", CURATED);
    let before_auto = fixture.read("auto.cfmap.md");
    let before_curated = fixture.read("curated.cfmap.md");
    let source = CodeMapSource::new(&fixture.root);
    let snapshots = source.snapshots().unwrap();
    let mut state = SqliteStateStore::open(fixture.root.join("state.sqlite")).unwrap();

    assert_eq!(
        state.reconcile_snapshots(&snapshots).unwrap().changes(),
        &[StateChange::Added, StateChange::Added]
    );
    assert_eq!(
        state
            .reconcile_snapshots(&source.snapshots().unwrap())
            .unwrap()
            .changes(),
        &[StateChange::Unchanged, StateChange::Unchanged]
    );
    assert_eq!(fixture.read("auto.cfmap.md"), before_auto);
    assert_eq!(fixture.read("curated.cfmap.md"), before_curated);
}

#[test]
fn map_rename_delete_and_reopen_reconcile_through_existing_state_without_new_authority() {
    let fixture = Fixture::new();
    fixture.write("architecture.md", "# Navigation\n");
    fixture.write("navigation.cfmap.md", AUTO);
    let source = CodeMapSource::new(&fixture.root);
    let mut state = SqliteStateStore::open(fixture.root.join("state.sqlite")).unwrap();
    state
        .reconcile_snapshots(&source.snapshots().unwrap())
        .unwrap();

    fs::rename(
        fixture.root.join("navigation.cfmap.md"),
        fixture.root.join("renamed.cfmap.md"),
    )
    .unwrap();
    assert_eq!(
        state
            .reconcile_snapshots(&source.snapshots().unwrap())
            .unwrap()
            .changes(),
        &[StateChange::Added, StateChange::Deleted]
    );
    fs::remove_file(fixture.root.join("renamed.cfmap.md")).unwrap();
    assert_eq!(
        state.reconcile_snapshots(&[]).unwrap().changes(),
        &[StateChange::Deleted]
    );
    fixture.write("renamed.cfmap.md", AUTO);
    assert_eq!(
        state
            .reconcile_snapshots(&source.snapshots().unwrap())
            .unwrap()
            .changes(),
        &[StateChange::Added]
    );
}

#[test]
fn missing_or_external_auto_source_is_stale_and_oversized_maps_are_rejected() {
    let fixture = Fixture::new();
    fixture.write("stale.cfmap.md", AUTO);
    let source = CodeMapSource::new(&fixture.root);
    assert_eq!(
        source.snapshots().unwrap()[0].records()[0]
            .metadata()
            .get("state"),
        Some(&"STALE".to_owned())
    );
    fixture.write(
        "oversized.cfmap.md",
        &format!(
            "---\ncfmap: v1\nmode: CURATED\n---\n# Big\n{}",
            "x".repeat(1_048_576)
        ),
    );
    assert_eq!(
        source.snapshots().unwrap_err().kind(),
        &ErrorKind::InvalidContent
    );
}

#[test]
fn external_source_locator_is_rejected_before_any_snapshot_is_returned() {
    let fixture = Fixture::new();
    fixture.write(
        "external.cfmap.md",
        "---\ncfmap: v1\nmode: AUTO\nsource: ../outside.md#Hidden\n---\n# External\n",
    );
    assert_eq!(
        CodeMapSource::new(&fixture.root)
            .snapshots()
            .unwrap_err()
            .kind(),
        &ErrorKind::InvalidContent
    );
}

#[cfg(windows)]
#[test]
fn external_junction_source_is_rejected_before_any_snapshot_is_returned() {
    let fixture = Fixture::new();
    let outside = fixture
        .root
        .parent()
        .unwrap()
        .join(format!("fastsearch-c1-external-{}", std::process::id()));
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("source.md"), "# Outside\n").unwrap();
    let junction = fixture.root.join("linked");
    let status = std::process::Command::new("cmd")
        .args([
            "/c",
            "mklink",
            "/J",
            junction.to_str().unwrap(),
            outside.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "test requires a Windows directory junction"
    );
    fixture.write(
        "external.cfmap.md",
        "---\ncfmap: v1\nmode: AUTO\nsource: linked/source.md#Hidden\n---\n# External\n",
    );
    assert_eq!(
        CodeMapSource::new(&fixture.root)
            .snapshots()
            .unwrap_err()
            .kind(),
        &ErrorKind::InvalidContent
    );
    let _ = fs::remove_dir(junction);
    let _ = fs::remove_dir_all(outside);
}
