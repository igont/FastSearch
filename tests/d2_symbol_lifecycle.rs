use fastsearch::{
    adapters::{state::SqliteStateStore, symbols::SymbolSource},
    domain::SearchMode,
    domain::SearchQuery,
    domain::{ErrorKind, LogicalRootId, StableId},
    ports::{SourcePort, StateChange, StateStore, SymbolPort},
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use tree_sitter::Parser;

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

fn rust_node_count(source: &str) -> usize {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    assert!(!tree.root_node().has_error());
    let mut nodes = 0;
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        nodes += 1;
        let mut cursor = node.walk();
        stack.extend(node.children(&mut cursor));
    }
    nodes
}

fn admitted_node_passport(root: &Path) -> Vec<(String, usize)> {
    fn visit(root: &Path, directory: &Path, rows: &mut Vec<(String, usize)>) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let file_type = entry.file_type().unwrap();
            let path = entry.path();
            if file_type.is_dir() {
                visit(root, &path, rows);
            } else if file_type.is_file()
                && matches!(
                    path.extension().and_then(|value| value.to_str()),
                    Some("rs")
                )
            {
                let text = fs::read_to_string(&path).unwrap();
                rows.push((
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                    rust_node_count(&text),
                ));
            }
        }
    }
    let mut rows = Vec::new();
    visit(root, root, &mut rows);
    rows.sort();
    rows
}

fn dense_rust(functions: usize) -> String {
    (0..functions)
        .map(|index| format!("fn dense_{index}() {{}}\n"))
        .collect()
}

#[test]
fn named_roots_are_deterministic_and_collision_free() {
    let a = Fixture::new();
    let b = Fixture::new();
    a.write("src/n.rs", "pub struct Nav;\npub fn go() {}\n");
    a.write("tools/n.py", "class Nav:\n    pass\ndef go():\n    pass\n");
    b.write("src/n.rs", "pub struct Nav;\npub fn go() {}\n");
    b.write("tools/n.py", "class Nav:\n    pass\ndef go():\n    pass\n");
    let left = source(&a, "code-a").records().unwrap();
    let right = source(&b, "code-b").records().unwrap();
    assert_eq!(left.len(), 4);
    assert_ne!(left[0].id(), right[0].id());
    assert_eq!(left, source(&a, "code-a").records().unwrap());
    assert!(
        left.iter()
            .all(|r| !r.id().as_str().contains(&a.0.to_string_lossy().to_string()))
    );
    assert!(
        left.iter()
            .any(|record| record.metadata().get("language") == Some(&"rust".to_owned()))
    );
    assert!(
        left.iter()
            .any(|record| record.metadata().get("language") == Some(&"python".to_owned()))
    );
}

#[test]
fn exact_current_src_repeats_complete_bounded_runtime_inventory() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let logical_root = LogicalRootId::parse("code-fastsearch").unwrap();
    let first = SymbolSource::new(logical_root.clone(), &root)
        .snapshot()
        .unwrap();
    let second = SymbolSource::new(logical_root, &root).snapshot().unwrap();
    let first_passport = admitted_node_passport(&root);
    let second_passport = admitted_node_passport(&root);
    let inventory = first_passport
        .iter()
        .map(|(path, _)| path.clone())
        .collect::<Vec<_>>();
    let mut snapshot_paths = first
        .iter()
        .map(|snapshot| snapshot.locator().path().to_owned())
        .collect::<Vec<_>>();
    snapshot_paths.sort();

    assert!(!first_passport.is_empty());
    assert_eq!(first_passport, second_passport);
    assert!(
        first_passport
            .iter()
            .all(|(_, nodes)| *nodes > 0 && *nodes <= 16_384)
    );
    assert_eq!(first, second);
    assert_eq!(snapshot_paths, inventory);
}

#[test]
fn amended_node_budget_accepts_dense_realistic_source_and_rejects_excess_atomically() {
    let accepted = dense_rust(1_000);
    let rejected = dense_rust(3_000);
    let accepted_nodes = rust_node_count(&accepted);
    let rejected_nodes = rust_node_count(&rejected);
    assert!(accepted_nodes > 512 && accepted_nodes <= 16_384);
    assert!(rejected_nodes > 16_384);
    assert!(rejected.len() <= 64 * 1024);

    let fixture = Fixture::new();
    fixture.write("accepted.rs", &accepted);
    assert_eq!(source(&fixture, "code").snapshot().unwrap().len(), 1);
    fixture.write("rejected.rs", &rejected);
    assert_eq!(
        source(&fixture, "code").snapshot().unwrap_err().kind(),
        &ErrorKind::InvalidContent
    );
}

#[test]
fn directory_depth_and_file_count_limits_remain_fail_closed() {
    let deep = Fixture::new();
    deep.write(
        &format!(
            "{}/deep.rs",
            (0..17).map(|_| "nested").collect::<Vec<_>>().join("/")
        ),
        "fn deep() {}\n",
    );
    assert_eq!(
        source(&deep, "code").snapshot().unwrap_err().kind(),
        &ErrorKind::InvalidContent
    );

    let many = Fixture::new();
    for index in 0..=1_024 {
        many.write(&format!("files/{index}.rs"), "fn item() {}\n");
    }
    assert_eq!(
        source(&many, "code").snapshot().unwrap_err().kind(),
        &ErrorKind::InvalidContent
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
