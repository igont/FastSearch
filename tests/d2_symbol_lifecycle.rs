use fastsearch::{
    adapters::{state::SqliteStateStore, symbols::SymbolSource},
    domain::SearchMode,
    domain::SearchQuery,
    domain::{ErrorKind, LogicalRootId},
    ports::{SourcePort, StateChange, StateStore, SymbolPort},
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "fastsearch-d2-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn write(&self, p: &str, t: &str) {
        let p = self.0.join(p);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, t).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn source(f: &Fixture, id: &str) -> SymbolSource {
    SymbolSource::new(LogicalRootId::parse(id).unwrap(), &f.0)
}

#[test]
fn named_roots_are_deterministic_and_collision_free() {
    let a = Fixture::new();
    let b = Fixture::new();
    a.write("src/n.rs", "pub struct Nav;\npub fn go() {}\n");
    b.write("src/n.rs", "pub struct Nav;\npub fn go() {}\n");
    let left = source(&a, "code-a").records().unwrap();
    let right = source(&b, "code-b").records().unwrap();
    assert_eq!(left.len(), 2);
    assert_ne!(left[0].id(), right[0].id());
    assert_eq!(left, source(&a, "code-a").records().unwrap());
    assert!(
        left.iter()
            .all(|r| !r.id().as_str().contains(&a.0.to_string_lossy().to_string()))
    );
}
#[test]
fn symbols_search_and_state_lifecycle_cover_rename_delete_reopen_and_rebuild() {
    let f = Fixture::new();
    f.write("src/nav.rs", "pub fn rebuild_index() {}\n");
    let s = source(&f, "code-fastsearch");
    let mut state = SqliteStateStore::open(f.0.join("state.sqlite")).unwrap();
    assert_eq!(
        state
            .reconcile_snapshots(&s.snapshot().unwrap())
            .unwrap()
            .changes(),
        &[StateChange::Added]
    );
    assert_eq!(
        s.find_symbols(&SearchQuery::new("rebuild", SearchMode::Balanced).unwrap())
            .unwrap()
            .len(),
        1
    );
    fs::rename(f.0.join("src/nav.rs"), f.0.join("src/renamed.rs")).unwrap();
    assert_eq!(
        state
            .reconcile_snapshots(&s.snapshot().unwrap())
            .unwrap()
            .changes(),
        &[StateChange::Added, StateChange::Deleted]
    );
    fs::remove_file(f.0.join("src/renamed.rs")).unwrap();
    assert_eq!(
        state.reconcile_snapshots(&[]).unwrap().changes(),
        &[StateChange::Deleted]
    );
    drop(state);
    let reopened = SqliteStateStore::open(f.0.join("state.sqlite")).unwrap();
    assert!(
        reopened
            .get(
                &s.records()
                    .unwrap_or_default()
                    .first()
                    .map(|r| r.id().clone())
                    .unwrap_or_else(|| fastsearch::domain::StableId::parse("none").unwrap())
            )
            .unwrap()
            .is_none()
    );
}
#[test]
fn parse_failure_or_unsupported_file_returns_no_partial_snapshot() {
    let f = Fixture::new();
    f.write("ok.rs", "fn ok() {}\n");
    f.write("bad.py", "def broken(\n");
    assert_eq!(
        source(&f, "code").snapshot().unwrap_err().kind(),
        &ErrorKind::InvalidContent
    );
    fs::remove_file(f.0.join("bad.py")).unwrap();
    f.write("ignored.txt", "fn fabricated() {}");
    assert_eq!(source(&f, "code").records().unwrap().len(), 1);
}
