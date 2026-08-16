use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use fastsearch::{
    application::{ProductionConfig, ProductionRuntime, ensure_e5_model},
    domain::{RetrievalChannel, SearchMode, SearchQuery},
    ports::AgentSurface,
};

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new() -> Self {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("fastsearch-e5-auto-{suffix}"));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
#[ignore = "downloads the pinned multilingual E5 model when the shared cache is empty"]
fn provisioned_model_is_ready_for_explicit_index_and_search() {
    let temporary = TemporaryDirectory::new();
    let documents = temporary.0.join("documents");
    let code = temporary.0.join("code");
    let service = temporary.0.join("service");
    fs::create_dir_all(&documents).unwrap();
    fs::create_dir_all(&code).unwrap();
    fs::write(
        documents.join("guide.md"),
        "# Vector retrieval\n\nFastSearch restores a local embedding model automatically.",
    )
    .unwrap();
    fs::write(
        code.join("lib.rs"),
        "pub fn automatic_embedding_pipeline() -> bool { true }\n",
    )
    .unwrap();

    let model = ensure_e5_model(false).unwrap();
    let mut runtime = ProductionRuntime::open(
        ProductionConfig::new(&documents, &code, &service).with_e5_root(model.root().to_path_buf()),
    )
    .unwrap();
    runtime.index().unwrap();
    let query = SearchQuery::new("automatic local embedding model", SearchMode::Balanced).unwrap();
    let response = runtime.search(&query).unwrap();

    assert!(!response.hits().is_empty());
    assert!(
        response
            .hits()
            .iter()
            .any(|hit| hit.channel() == RetrievalChannel::Vector)
    );
}
