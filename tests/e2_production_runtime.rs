use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use fastsearch::{
    adapters::{source::FilesystemSource, symbols::SymbolSource},
    application::{ProductionConfig, ProductionRuntime},
    domain::{
        Capability, CapabilityState, IndexFreshness, LogicalRootId, RelatedQuery, SearchMode,
        SearchQuery,
    },
    ports::{AgentSurface, SourcePort},
};

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("fastsearch-e2-{suffix}"));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn child(&self, value: &str) -> PathBuf {
        self.0.join(value)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn one_production_composition_indexes_reopens_fuses_and_resolves_map_to_symbol() {
    let temp = Temp::new();
    let documents = temp.child("replaceable-documents");
    let code = temp.child("replaceable-code");
    let service = temp.child("service/.cfknowledge");
    fs::create_dir_all(&documents).unwrap();
    fs::create_dir_all(code.join("src")).unwrap();
    fs::write(
        documents.join("architecture.md"),
        "# Navigation contract\n\nsemantic code navigation",
    )
    .unwrap();
    fs::write(
        code.join("src/navigator.rs"),
        "pub fn stable_navigation() {}\n",
    )
    .unwrap();
    let document_id = FilesystemSource::new(&documents)
        .records()
        .unwrap()
        .into_iter()
        .next()
        .unwrap()
        .id()
        .clone();

    let symbol = SymbolSource::new(LogicalRootId::parse("code-fastsearch").unwrap(), &code)
        .records()
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    fs::write(
        documents.join("navigation.cfmap.md"),
        format!(
            "---\ncfmap: v1\nmode: CURATED\n---\n# Navigation map\n@related {}\n",
            symbol.id().as_str()
        ),
    )
    .unwrap();

    let config = ProductionConfig::new(&documents, &code, &service);
    let mut runtime = ProductionRuntime::open(config.clone()).unwrap();
    assert_eq!(
        runtime.index().unwrap().freshness(),
        IndexFreshness::Current
    );
    let fused = runtime
        .search(&SearchQuery::new("navigation", SearchMode::Design).unwrap())
        .unwrap();
    assert!(!fused.hits().is_empty());
    assert!(
        fused
            .hits()
            .iter()
            .any(|hit| hit.record().title() == "Navigation map")
    );

    let maps = fastsearch::adapters::maps::CodeMapSource::new(&documents)
        .records()
        .unwrap();
    let related = runtime
        .related(&RelatedQuery::new(maps[0].id().clone()))
        .unwrap();
    assert_eq!(related.len(), 1);
    assert_eq!(related[0].id(), symbol.id());
    assert!(related[0].metadata().contains_key("relation_provenance"));

    let vector = runtime
        .status()
        .into_iter()
        .find(|status| status.capability() == Capability::VectorRetrieval)
        .unwrap();
    assert!(matches!(
        vector.state(),
        CapabilityState::Unavailable { .. }
    ));
    assert_eq!(fused.freshness(), IndexFreshness::Stale);

    let exact_run = runtime.record_run_marker("E2-run-001").unwrap();
    fs::write(exact_run.join("owned.tmp"), "owned").unwrap();
    assert!(!runtime.cleanup_run("E2-run-002").unwrap());
    assert!(exact_run.exists());
    assert!(runtime.cleanup_run("E2-run-001").unwrap());
    assert!(!exact_run.exists());
    assert!(runtime.record_run_marker("../escape").is_err());

    fs::write(code.join("unsupported.txt"), "must not partially publish").unwrap();
    assert!(runtime.index().is_err());
    assert!(runtime.get(&document_id).unwrap().is_some());
    fs::remove_file(code.join("unsupported.txt")).unwrap();

    fs::write(
        documents.join("architecture.md"),
        "# Navigation contract\n\nsemantic code navigation changed",
    )
    .unwrap();
    runtime.index().unwrap();
    assert!(
        runtime
            .get(&document_id)
            .unwrap()
            .unwrap()
            .searchable_content()
            .contains("changed")
    );
    fs::remove_file(documents.join("architecture.md")).unwrap();
    runtime.index().unwrap();
    assert!(runtime.get(&document_id).unwrap().is_none());

    fs::write(
        code.join("src/navigator.rs"),
        "// shifted structural identity\npub fn stable_navigation() {}\n",
    )
    .unwrap();
    runtime.index().unwrap();
    assert!(
        runtime
            .related(&RelatedQuery::new(maps[0].id().clone()))
            .is_err()
    );
    let reparsed = SymbolSource::new(LogicalRootId::parse("code-fastsearch").unwrap(), &code)
        .records()
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    assert_ne!(reparsed.id(), symbol.id());
    fs::write(
        documents.join("navigation.cfmap.md"),
        format!(
            "---\ncfmap: v1\nmode: CURATED\n---\n# Navigation map\n@related {}\n",
            reparsed.id().as_str()
        ),
    )
    .unwrap();
    runtime.index().unwrap();
    assert_eq!(
        runtime
            .related(&RelatedQuery::new(maps[0].id().clone()))
            .unwrap()[0]
            .id(),
        reparsed.id()
    );
    drop(runtime);

    let mut reopened = ProductionRuntime::open(config).unwrap();
    assert_eq!(reopened.index_status().freshness(), IndexFreshness::Current);
    assert_eq!(
        reopened.rebuild().unwrap().freshness(),
        IndexFreshness::Current
    );
    assert_eq!(
        reopened
            .related(&RelatedQuery::new(maps[0].id().clone()))
            .unwrap()[0]
            .id(),
        reparsed.id()
    );
}

#[test]
fn production_configuration_rejects_overlapping_service_and_source_roots() {
    let temp = Temp::new();
    let documents = temp.child("documents");
    let code = temp.child("code");
    fs::create_dir_all(&documents).unwrap();
    fs::create_dir_all(&code).unwrap();
    let error = ProductionRuntime::open(ProductionConfig::new(
        &documents,
        &code,
        documents.join("unsafe-service"),
    ))
    .expect_err("service containment must fail closed");
    assert!(error.to_string().contains("service root"));
}

#[test]
fn parameterized_cli_reopens_in_separate_process_and_has_no_alternate_provider_route() {
    let temp = Temp::new();
    let documents = temp.child("documents-cli");
    let code = temp.child("code-cli");
    let service = temp.child("service-cli/.cfknowledge");
    fs::create_dir_all(&documents).unwrap();
    fs::create_dir_all(code.join("src")).unwrap();
    fs::write(
        documents.join("guide.md"),
        "# CLI navigation\n\nreplaceable roots",
    )
    .unwrap();
    fs::write(code.join("src/lib.rs"), "pub fn cli_symbol() {}\n").unwrap();
    let symbol = SymbolSource::new(LogicalRootId::parse("code-fastsearch").unwrap(), &code)
        .records()
        .unwrap()
        .remove(0);
    fs::write(
        documents.join("cli.cfmap.md"),
        format!(
            "---\ncfmap: v1\nmode: CURATED\n---\n# CLI map\n@related {}\n",
            symbol.id().as_str()
        ),
    )
    .unwrap();

    let binary = env!("CARGO_BIN_EXE_fastsearch");
    let run = |arguments: &[&str]| Command::new(binary).args(arguments).output().unwrap();
    let documents_text = documents.to_string_lossy().into_owned();
    let code_text = code.to_string_lossy().into_owned();
    let service_text = service.to_string_lossy().into_owned();
    let rebuild = run(&[
        "index",
        "rebuild",
        &documents_text,
        &code_text,
        &service_text,
    ]);
    assert!(
        rebuild.status.success(),
        "{}",
        String::from_utf8_lossy(&rebuild.stderr)
    );

    let status = run(&["status", &documents_text, &code_text, &service_text]);
    let status_text = String::from_utf8(status.stdout).unwrap();
    assert!(status_text.contains("freshness=Current"));
    assert!(status_text.contains("CodeMaps=Real"));
    assert!(status_text.contains("Symbols=Real"));
    assert!(status_text.contains("VectorRetrieval=Unavailable"));

    let search = run(&[
        "search",
        &documents_text,
        &code_text,
        &service_text,
        "design",
        "CLI",
    ]);
    assert!(search.status.success());
    assert!(String::from_utf8_lossy(&search.stdout).contains("hits="));

    let map = fastsearch::adapters::maps::CodeMapSource::new(&documents)
        .records()
        .unwrap()
        .remove(0);
    let related = run(&[
        "related",
        &documents_text,
        &code_text,
        &service_text,
        map.id().as_str(),
    ]);
    assert!(
        related.status.success(),
        "{}",
        String::from_utf8_lossy(&related.stderr)
    );
    assert!(String::from_utf8_lossy(&related.stdout).contains(symbol.id().as_str()));

    let help = run(&[]);
    let help_text = String::from_utf8_lossy(&help.stderr);
    assert!(!help_text.contains("mock"));
    assert!(!help_text.contains("provider"));
    assert!(!help_text.contains("test-fail"));
}
