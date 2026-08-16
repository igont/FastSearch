use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use fastsearch::{
    application::{ProductionRuntime, WorkspaceProfile, WorkspaceStore},
    domain::{IndexFreshness, SearchMode, SearchQuery},
    ports::AgentSurface,
};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "fastsearch-workspace-runtime-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn workspace_runtime_combines_multiple_roots_without_identity_collisions() {
    let temp = Temp::new();
    let docs_a = temp.0.join("docs-a");
    let docs_b = temp.0.join("docs-b");
    let code_a = temp.0.join("code-a");
    let code_b = temp.0.join("code-b");
    for root in [&docs_a, &docs_b, &code_a, &code_b] {
        fs::create_dir_all(root).unwrap();
    }
    fs::write(
        docs_a.join("guide.md"),
        "# Alpha retry\n\nalpha workspace token",
    )
    .unwrap();
    fs::write(
        docs_b.join("guide.md"),
        "# Beta retry\n\nbeta workspace token",
    )
    .unwrap();
    fs::write(code_a.join("lib.rs"), "pub fn alpha_workspace() {}\n").unwrap();
    fs::write(code_b.join("lib.rs"), "pub fn beta_workspace() {}\n").unwrap();

    let profile =
        WorkspaceProfile::from_roots(&temp.0, "Multi root", [docs_a, docs_b], [code_a, code_b])
            .unwrap();
    let store = WorkspaceStore::create(&temp.0, profile).unwrap();
    let mut runtime = ProductionRuntime::open(store.production_config()).unwrap();

    assert_eq!(
        runtime.index().unwrap().freshness(),
        IndexFreshness::Current
    );
    let alpha = runtime
        .search(&SearchQuery::new("alpha", SearchMode::Balanced).unwrap())
        .unwrap();
    let beta = runtime
        .search(&SearchQuery::new("beta", SearchMode::Balanced).unwrap())
        .unwrap();
    let alpha_id = alpha
        .hits()
        .iter()
        .map(|hit| hit.record().id().as_str())
        .find(|id| id.contains("documentation-"))
        .unwrap();
    let beta_id = beta
        .hits()
        .iter()
        .map(|hit| hit.record().id().as_str())
        .find(|id| id.contains("documentation-"))
        .unwrap();
    assert!(alpha_id.contains("documentation-"), "{alpha_id}");
    assert!(beta_id.contains("documentation-"), "{beta_id}");
    assert_ne!(alpha_id, beta_id);
    assert!(temp.0.join(".fastsearch/local/state.sqlite").is_file());
    assert!(
        temp.0
            .join(".fastsearch/local/index/cross/lexical")
            .is_dir()
    );
}

#[test]
fn documentation_only_code_only_and_empty_workspaces_open_independently() {
    for (name, with_documents, with_code) in [
        ("documents", true, false),
        ("code", false, true),
        ("empty", false, false),
    ] {
        let temp = Temp::new();
        let documents = temp.0.join("documents");
        let code = temp.0.join("code");
        let document_roots = if with_documents {
            fs::create_dir_all(&documents).unwrap();
            fs::write(documents.join("guide.md"), "# Guide\n\nsearchable").unwrap();
            vec![documents]
        } else {
            Vec::new()
        };
        let code_roots = if with_code {
            fs::create_dir_all(&code).unwrap();
            fs::write(code.join("lib.rs"), "pub fn searchable_code() {}\n").unwrap();
            vec![code]
        } else {
            Vec::new()
        };
        let profile =
            WorkspaceProfile::from_roots(&temp.0, name, document_roots, code_roots).unwrap();
        let store = WorkspaceStore::create(&temp.0, profile).unwrap();
        let mut runtime = ProductionRuntime::open(store.production_config()).unwrap();

        assert_eq!(
            runtime.index().unwrap().freshness(),
            IndexFreshness::Current
        );
        assert_eq!(
            store.profile().contour_count(),
            usize::from(with_documents) + usize::from(with_code)
        );
    }
}

#[test]
fn generated_traceability_coverage_is_absent_but_ordinary_registry_is_searchable() {
    let temp = Temp::new();
    let documents = temp.0.join("documents");
    let traceability = documents.join("Traceability");
    fs::create_dir_all(&traceability).unwrap();
    fs::write(
        traceability.join("Paradigm Coverage Registry.tsv"),
        concat!(
            "id\tpath\tsummary\ttdr_coverage\ttdr_refs\twarnings\terrors\n",
            "derived\tdocs/source.md\tderivedonlytoken\tdirect\tTDR-1\t\t\n"
        ),
    )
    .unwrap();
    fs::write(
        documents.join("TDR Registry.tsv"),
        "id\ttitle\nTDR-1\tordinarytoken\n",
    )
    .unwrap();
    let transitioning = traceability.join("Transitioning Registry.tsv");
    fs::write(&transitioning, "id\ttitle\nlegacy\tlegacyregistrytoken\n").unwrap();

    let profile = WorkspaceProfile::from_roots(
        &temp.0,
        "Registry policy",
        [documents],
        Vec::<PathBuf>::new(),
    )
    .unwrap();
    let store = WorkspaceStore::create(&temp.0, profile).unwrap();
    let mut runtime = ProductionRuntime::open(store.production_config()).unwrap();
    assert_eq!(
        runtime.index().unwrap().freshness(),
        IndexFreshness::Current
    );

    let generated = runtime
        .search(&SearchQuery::new("derivedonlytoken", SearchMode::Balanced).unwrap())
        .unwrap();
    let ordinary = runtime
        .search(&SearchQuery::new("ordinarytoken", SearchMode::Balanced).unwrap())
        .unwrap();
    let legacy = runtime
        .search(&SearchQuery::new("legacyregistrytoken", SearchMode::Balanced).unwrap())
        .unwrap();

    assert!(generated.hits().is_empty());
    assert_eq!(ordinary.hits().len(), 1);
    assert_eq!(ordinary.hits()[0].record().title(), "TDR-1");
    assert_eq!(legacy.hits().len(), 1);

    fs::write(
        &transitioning,
        concat!(
            "id\tpath\tsummary\ttdr_coverage\ttdr_refs\twarnings\terrors\n",
            "derived\tdocs/source.md\tnewderivedtoken\tdirect\tTDR-1\t\t\n"
        ),
    )
    .unwrap();
    assert_eq!(
        runtime.index().unwrap().freshness(),
        IndexFreshness::Current
    );
    let removed_legacy = runtime
        .search(&SearchQuery::new("legacyregistrytoken", SearchMode::Balanced).unwrap())
        .unwrap();
    let skipped_new = runtime
        .search(&SearchQuery::new("newderivedtoken", SearchMode::Balanced).unwrap())
        .unwrap();
    assert!(removed_legacy.hits().is_empty());
    assert!(skipped_new.hits().is_empty());
}
