use fastsearch::{
    adapters::{state::SqliteStateStore, symbols::SymbolSource},
    domain::SearchMode,
    domain::SearchQuery,
    domain::{ErrorKind, LogicalRootId, StableId},
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
    let old_id = s.records().unwrap()[0].id().clone();
    let state_path = f.0.with_extension("state.sqlite");
    let mut state = SqliteStateStore::open(&state_path).unwrap();
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
    let reopened = SqliteStateStore::open(&state_path).unwrap();
    assert!(reopened.get(&old_id).unwrap().is_none());
    drop(reopened);
    fs::remove_file(state_path).unwrap();
}
#[test]
fn parse_failure_unsupported_or_oversized_file_returns_no_partial_snapshot() {
    let f = Fixture::new();
    f.write("ok.rs", "fn ok() {}\n");
    f.write("bad.py", "def broken(\n");
    assert_eq!(
        source(&f, "code").snapshot().unwrap_err().kind(),
        &ErrorKind::InvalidContent
    );
    fs::remove_file(f.0.join("bad.py")).unwrap();
    f.write("unsupported.txt", "fn fabricated() {}");
    assert_eq!(
        source(&f, "code").snapshot().unwrap_err().kind(),
        &ErrorKind::InvalidContent
    );
    fs::remove_file(f.0.join("unsupported.txt")).unwrap();
    f.write("big.rs", &format!("// {}", "x".repeat(64 * 1024)));
    assert_eq!(
        source(&f, "code").snapshot().unwrap_err().kind(),
        &ErrorKind::InvalidContent
    );
}

#[test]
fn duplicate_structural_names_remain_distinct_in_production_snapshot() {
    let f = Fixture::new();
    f.write(
        "src/repeated.rs",
        "fn repeated() {}\nmod nested { fn repeated() {} }\n",
    );
    let symbols = source(&f, "code").records().unwrap();
    assert_eq!(symbols.len(), 2);
    assert_ne!(symbols[0].id(), symbols[1].id());
    assert!(
        symbols
            .iter()
            .all(|symbol| symbol.id() != &StableId::parse("none").unwrap())
    );
}
