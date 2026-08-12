use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

use fastsearch::adapters::maps::{CodeMapSource, RegenerationOutcome};
use fastsearch::domain::{ErrorKind, RecordKind};
use fastsearch::ports::SourcePort;

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

const AUTO: &str = "---\ncfmap: v1\nmode: AUTO\nsource: architecture.md#Navigation\ngeneration: 1\n---\n# Navigation\nHuman note stays.\n<!-- cfmap:auto:start -->\nold derived body\n<!-- cfmap:auto:end -->\n";

const CURATED: &str = "---\ncfmap: v1\nmode: CURATED\n---\n# Curated\nHuman-authored body.\n";

#[test]
fn cfmap_v1_admits_exactly_one_map_snapshot_and_rejects_invalid_input_atomically() {
    let fixture = Fixture::new();
    fixture.write("architecture.md", "# Navigation\n");
    fixture.write("navigation.cfmap.md", AUTO);

    let source = CodeMapSource::new(&fixture.root);
    let snapshots = SourcePort::snapshot(&source).expect("valid v1 map must admit");
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].records().len(), 1);
    let record = &snapshots[0].records()[0];
    assert_eq!(record.kind(), RecordKind::CodeMap);
    assert_eq!(record.locator().path(), "navigation.cfmap.md");
    assert_eq!(record.metadata().get("mode"), Some(&"AUTO".to_owned()));
    assert_eq!(record.metadata().get("state"), Some(&"CURRENT".to_owned()));

    fixture.write(
        "invalid.cfmap.md",
        "---\ncfmap: v1\nmode: UNKNOWN\n---\n# Invalid\n",
    );
    let error = source
        .snapshots()
        .expect_err("one invalid map rejects all maps");
    assert_eq!(error.kind(), &ErrorKind::InvalidContent);
}

#[test]
fn map_delete_and_reopen_keep_the_same_locator_identity_and_restore_current_state() {
    let fixture = Fixture::new();
    fixture.write("architecture.md", "# Navigation\n");
    fixture.write("navigation.cfmap.md", AUTO);
    let source = CodeMapSource::new(&fixture.root);
    let first = source.snapshots().unwrap();
    let first_id = first[0].records()[0].id().clone();

    fs::remove_file(fixture.root.join("navigation.cfmap.md")).unwrap();
    assert!(source.snapshots().unwrap().is_empty());
    fixture.write(
        "navigation.cfmap.md",
        AUTO.replace("old derived body", "reopened body").as_str(),
    );

    let reopened = source.snapshots().unwrap();
    assert_eq!(reopened[0].records()[0].id(), &first_id);
    assert_eq!(
        reopened[0].records()[0].metadata().get("state"),
        Some(&"CURRENT".to_owned())
    );
}

#[test]
fn regeneration_replaces_only_auto_region_and_never_overwrites_curated_bytes() {
    let fixture = Fixture::new();
    fixture.write("architecture.md", "# Navigation\n");
    fixture.write("auto.cfmap.md", AUTO);
    fixture.write("curated.cfmap.md", CURATED);
    let source = CodeMapSource::new(&fixture.root);

    assert_eq!(
        source
            .regenerate("auto.cfmap.md", "new derived body")
            .unwrap(),
        RegenerationOutcome::UpdatedAuto
    );
    assert_eq!(
        fixture.read("auto.cfmap.md"),
        AUTO.replace("old derived body", "new derived body")
    );
    assert_eq!(
        source
            .regenerate("curated.cfmap.md", "must not appear")
            .unwrap(),
        RegenerationOutcome::PreservedCurated
    );
    assert_eq!(fixture.read("curated.cfmap.md"), CURATED);
}

#[test]
fn missing_auto_source_is_stale_and_failed_regeneration_leaves_prior_file_recoverable() {
    let fixture = Fixture::new();
    fixture.write("stale.cfmap.md", AUTO);
    let source = CodeMapSource::new(&fixture.root);

    let snapshots = source.snapshots().expect("stale map remains observable");
    assert_eq!(
        snapshots[0].records()[0].metadata().get("state"),
        Some(&"STALE".to_owned())
    );
    let before = fixture.read("stale.cfmap.md");
    let error = source
        .regenerate("stale.cfmap.md", "bad\n<!-- cfmap:auto:end -->")
        .unwrap_err();
    assert_eq!(error.kind(), &ErrorKind::InvalidContent);
    assert_eq!(fixture.read("stale.cfmap.md"), before);
}
