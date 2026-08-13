use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use fastsearch::{
    adapters::source::FilesystemSource,
    application::RealRuntime,
    domain::{IndexFreshness, SearchMode, SearchQuery},
    ports::{AgentSurface, SourcePort},
};

#[test]
fn public_real_runtime_is_exported_from_a_dedicated_compatibility_owner() {
    let application = include_str!("../src/application/mod.rs");

    assert!(
        application.contains("mod compatibility;")
            && application.contains("pub use compatibility::RealRuntime;"),
        "RealRuntime must be exported from the dedicated compatibility module"
    );
}

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new(name: &str) -> Self {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time is after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("fastsearch-e1-{name}-{suffix}"));
        fs::create_dir_all(&path).expect("temporary directory");
        Self(path)
    }

    fn child(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn real_runtime_reconciles_one_full_scan_and_rebuilds_after_deleted_source_reopen() {
    let temporary = TemporaryDirectory::new("deleted-source");
    let source_root = temporary.child("source");
    let service_root = temporary.child("service");
    fs::create_dir_all(&source_root).expect("source root");
    let document = source_root.join("guide.md");
    fs::write(
        &document,
        "# Real lifecycle\n\nУдаляемый документ для реального восстановления.",
    )
    .expect("source fixture");

    let source = FilesystemSource::new(&source_root);
    let deleted_id = source
        .records()
        .expect("fixture source records")
        .into_iter()
        .next()
        .expect("fixture has one record")
        .id()
        .clone();
    let query = SearchQuery::new("lifecycle", SearchMode::Balanced).expect("valid query");

    let mut runtime = RealRuntime::open(&source_root, &service_root).expect("open real runtime");
    assert_eq!(
        runtime.index().expect("initial index").freshness(),
        IndexFreshness::Current
    );
    assert!(runtime.search(&query).expect("current search").hits().len() == 1);

    fs::remove_file(document).expect("delete source document");
    runtime
        .index()
        .expect("delete reconciliation and projection");
    drop(runtime);

    let mut reopened = RealRuntime::open(&source_root, &service_root).expect("reopen real runtime");
    assert!(
        reopened
            .get(&deleted_id)
            .expect("durable state read")
            .is_none()
    );
    assert_eq!(reopened.index_status().freshness(), IndexFreshness::Current);
    assert!(
        reopened
            .search(&query)
            .expect("reopened search")
            .hits()
            .is_empty()
    );

    assert_eq!(
        reopened.rebuild().expect("rebuild").freshness(),
        IndexFreshness::Current
    );
}
