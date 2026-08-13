use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::{
        Arc, Barrier,
        atomic::{AtomicU64, Ordering},
    },
    thread,
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

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

impl Temp {
    fn new() -> Self {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "fastsearch-e2-{}-{suffix}-{sequence}",
            std::process::id()
        ));
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
    assert!(!runtime.cleanup_run("E2-run-002").unwrap());
    assert!(exact_run.exists());
    let unknown = exact_run.join("unknown.tmp");
    fs::write(&unknown, "must survive refused cleanup").unwrap();
    assert!(runtime.cleanup_run("E2-run-001").is_err());
    assert_eq!(
        fs::read_to_string(&unknown).unwrap(),
        "must survive refused cleanup"
    );
    fs::remove_file(unknown).unwrap();
    assert!(runtime.cleanup_run("E2-run-001").unwrap());
    assert!(!exact_run.exists());
    let preexisting_empty = service.join("runs").join("preexisting-empty");
    fs::create_dir(&preexisting_empty).unwrap();
    assert!(runtime.record_run_marker("preexisting-empty").is_err());
    assert!(preexisting_empty.exists());
    let preexisting_marked = service.join("runs").join("preexisting-marked");
    fs::create_dir(&preexisting_marked).unwrap();
    fs::write(preexisting_marked.join("owner.marker"), "foreign").unwrap();
    assert!(runtime.record_run_marker("preexisting-marked").is_err());
    assert_eq!(
        fs::read_to_string(preexisting_marked.join("owner.marker")).unwrap(),
        "foreign"
    );
    assert!(runtime.cleanup_run("preexisting-marked").is_err());
    assert!(preexisting_marked.exists());
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
    let unsafe_service = documents.join("unsafe-service");
    let error = ProductionRuntime::open(ProductionConfig::new(&documents, &code, &unsafe_service))
        .expect_err("service containment must fail closed");
    assert!(error.to_string().contains("service root"));
    assert!(!unsafe_service.exists(), "reject must happen before write");

    let allowed = documents.join(".cfknowledge").join("E2-allowed");
    let runtime = ProductionRuntime::open(ProductionConfig::new(&documents, &code, &allowed));
    assert!(runtime.is_ok());

    let isolated = temp.child("isolated-service");
    assert!(ProductionRuntime::open(ProductionConfig::new(&documents, &code, &isolated)).is_ok());
}

#[cfg(windows)]
#[test]
fn service_junction_is_rejected_before_external_state_write() {
    let temp = Temp::new();
    let documents = temp.child("documents");
    let code = temp.child("code");
    let external = temp.child("external");
    fs::create_dir_all(documents.join(".cfknowledge")).unwrap();
    fs::create_dir_all(&code).unwrap();
    fs::create_dir_all(&external).unwrap();
    fs::write(external.join("sentinel.txt"), "unchanged").unwrap();
    fs::write(external.join("sentinel.txt"), "unchanged").unwrap();
    let junction = documents.join(".cfknowledge").join("E2-junction");
    let output = Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&junction)
        .arg(&external)
        .output()
        .unwrap();
    assert!(output.status.success());
    let error = ProductionRuntime::open(ProductionConfig::new(&documents, &code, &junction))
        .expect_err("junction service must fail closed");
    assert!(error.to_string().contains("reparse point"));
    assert!(!external.join("state.sqlite").exists());
    assert_eq!(
        fs::read_to_string(external.join("sentinel.txt")).unwrap(),
        "unchanged"
    );

    let missing_child = junction.join("missing-child");
    let error = ProductionRuntime::open(ProductionConfig::new(&documents, &code, &missing_child))
        .expect_err("missing child beneath junction must fail before create");
    assert!(error.to_string().contains("reparse point"));
    assert!(!external.join("missing-child").exists());
}

#[cfg(windows)]
#[test]
fn run_junction_added_after_open_is_rejected_before_external_marker_write() {
    let temp = Temp::new();
    let documents = temp.child("documents");
    let code = temp.child("code");
    let service = temp.child("service");
    let external = temp.child("external-runs");
    fs::create_dir_all(&documents).unwrap();
    fs::create_dir_all(&code).unwrap();
    fs::create_dir_all(&external).unwrap();
    fs::write(external.join("sentinel.txt"), "unchanged").unwrap();
    fs::write(documents.join("safe.md"), "# Safe").unwrap();
    fs::write(code.join("safe.rs"), "pub fn safe() {}").unwrap();
    let runtime =
        ProductionRuntime::open(ProductionConfig::new(&documents, &code, &service)).unwrap();
    let runs = service.join("runs");
    let command = format!(
        "rmdir \"{}\" && mklink /J \"{}\" \"{}\"",
        runs.display(),
        runs.display(),
        external.display()
    );
    let output = Command::new("cmd")
        .args(["/d", "/s", "/c", &command])
        .output()
        .unwrap();
    assert!(!output.status.success(), "guard must prevent junction swap");
    let run = runtime.record_run_marker("E2-run").unwrap();
    assert!(run.exists());
    let stolen = service.join("runs").join("stolen-run");
    let race_command = format!(
        "ren \"{}\" \"stolen-run\" && mklink /J \"{}\" \"{}\"",
        run.display(),
        run.display(),
        external.display()
    );
    let barrier = Arc::new(Barrier::new(2));
    let attacker_barrier = Arc::clone(&barrier);
    let attacker = thread::spawn(move || {
        attacker_barrier.wait();
        Command::new("cmd")
            .args(["/d", "/s", "/c", &race_command])
            .output()
            .unwrap()
    });
    barrier.wait();
    assert!(runtime.cleanup_run("E2-run").unwrap());
    let raced = attacker.join().unwrap();
    assert!(
        !raced.status.success(),
        "pinned run child must deny rename race"
    );
    assert!(!stolen.exists());
    assert!(!external.join("E2-run").exists());
    assert_eq!(
        fs::read_to_string(external.join("sentinel.txt")).unwrap(),
        "unchanged"
    );
}

#[cfg(windows)]
#[test]
fn service_bootstrap_race_never_writes_sqlite_or_index_through_junction() {
    let temp = Temp::new();
    let documents = temp.child("bootstrap-documents");
    let code = temp.child("bootstrap-code");
    fs::create_dir_all(&documents).unwrap();
    fs::create_dir_all(&code).unwrap();
    fs::write(documents.join("safe.md"), "# Safe").unwrap();
    fs::write(code.join("safe.rs"), "pub fn safe() {}").unwrap();

    for attempt in 0..32 {
        let service = temp.child(&format!("service-{attempt}"));
        let external = temp.child(&format!("external-{attempt}"));
        fs::create_dir_all(&external).unwrap();
        fs::write(external.join("sentinel.txt"), "unchanged").unwrap();
        let command = format!(
            "for /L %i in (1,1,200) do @if exist \"{}\" (rmdir /S /Q \"{}\" & mklink /J \"{}\" \"{}\" & exit /B) else @ping -n 1 -w 1 127.0.0.1 >nul",
            service.display(),
            service.display(),
            service.display(),
            external.display()
        );
        let attacker = thread::spawn(move || {
            Command::new("cmd")
                .args(["/d", "/s", "/c", &command])
                .output()
                .unwrap()
        });
        let runtime = ProductionRuntime::open(ProductionConfig::new(&documents, &code, &service));
        let attacked = attacker.join().unwrap();
        if let Ok(runtime) = runtime {
            assert!(!attacked.status.success() || !service.is_symlink());
            drop(runtime);
        }
        assert_eq!(
            fs::read_to_string(external.join("sentinel.txt")).unwrap(),
            "unchanged"
        );
        assert!(!external.join("state.sqlite").exists());
        assert!(!external.join("lexical").exists());
    }
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

#[test]
fn copied_document_only_service_never_exposes_wrong_get_before_production_rebuild() {
    let temp = Temp::new();
    let documents = temp.child("legacy-documents");
    let code = temp.child("production-code");
    let service = temp.child("copied-legacy-service");
    fs::create_dir_all(&documents).unwrap();
    fs::create_dir_all(&code).unwrap();
    fs::write(
        documents.join("legacy.md"),
        "# Legacy\n\nold state identity",
    )
    .unwrap();
    fs::create_dir_all(&service).unwrap();
    let connection = rusqlite::Connection::open(service.join("state.sqlite")).unwrap();
    connection.execute_batch(
        "CREATE TABLE state_records (id TEXT PRIMARY KEY, kind INTEGER NOT NULL, locator_path TEXT NOT NULL, selector_kind INTEGER NOT NULL, title TEXT NOT NULL, searchable_content TEXT NOT NULL, content_hash TEXT NOT NULL);
         INSERT INTO state_records VALUES ('legacy-id', 1, 'legacy.md', 4, 'legacy', 'legacy', 'sha256:legacy');",
    ).unwrap();
    drop(connection);
    let legacy_id = fastsearch::domain::StableId::parse("legacy-id").unwrap();

    let mut production =
        ProductionRuntime::open(ProductionConfig::new(&documents, &code, &service)).unwrap();
    assert_ne!(
        production.index_status().freshness(),
        IndexFreshness::Current
    );
    assert!(
        production.get(&legacy_id).unwrap().is_none(),
        "legacy identity must not leak wrong get"
    );
    production.rebuild().unwrap();
    assert_eq!(
        production.index_status().freshness(),
        IndexFreshness::Current
    );
    assert!(production.get(&legacy_id).unwrap().is_none());
    let current_id = FilesystemSource::new(&documents)
        .records()
        .unwrap()
        .remove(0)
        .id()
        .clone();
    assert!(production.get(&current_id).unwrap().is_some());
}

#[test]
fn production_facade_delegates_service_and_run_security_to_internal_owner() {
    let source = include_str!("../src/application/production.rs");
    let facade = source
        .split("impl ProductionRuntime")
        .nth(1)
        .expect("ProductionRuntime implementation is present")
        .split("impl AgentSurface for ProductionRuntime")
        .next()
        .expect("production facade precedes AgentSurface implementation");

    assert!(
        source.contains("mod security") && source.contains("struct ServiceRunBoundary"),
        "service/run security needs a dedicated internal owner"
    );
    for implementation_detail in [
        "ensure_no_reparse_points(&runs)",
        "RunDirectoryGuard::acquire(&run)",
        "securely_create_and_pin_service(&config.service_root)",
    ] {
        assert!(
            !facade.contains(implementation_detail),
            "ProductionRuntime facade must delegate {implementation_detail}"
        );
    }
}

#[test]
fn production_facade_delegates_indexing_and_search_policy_to_internal_coordinators() {
    let source = include_str!("../src/application/production.rs");
    let facade = source
        .split("impl ProductionRuntime")
        .nth(1)
        .expect("ProductionRuntime implementation is present")
        .split("impl AgentSurface for ProductionRuntime")
        .next()
        .expect("production facade precedes AgentSurface implementation");

    assert!(
        source.contains("struct IndexingCoordinator")
            && source.contains("struct SearchCoordinator"),
        "indexing and search each need a dedicated internal coordinator"
    );
    for direct_policy in [
        "self.documents.snapshot()",
        "self.state.reconcile_snapshots(&snapshots)",
        "self.lexical.apply_projection(&records",
        "FusionCoordinator::fuse(query, candidates, &self.status())",
    ] {
        assert!(
            !facade.contains(direct_policy),
            "ProductionRuntime facade must delegate {direct_policy}"
        );
    }
}

#[test]
#[ignore = "requires FASTSEARCH_E5_MODEL_ROOT accepted complete local cache"]
fn configured_provider_failure_preserves_authority_then_recovers_without_false_hits() {
    let temp = Temp::new();
    let documents = temp.child("provider-documents");
    let code = temp.child("provider-code");
    let service = temp.child("provider-service");
    fs::create_dir_all(&documents).unwrap();
    fs::create_dir_all(&code).unwrap();
    fs::write(
        documents.join("provider.md"),
        "# Provider\n\nsemantic recovery",
    )
    .unwrap();
    let id = FilesystemSource::new(&documents)
        .records()
        .unwrap()
        .remove(0)
        .id()
        .clone();
    let model = PathBuf::from(std::env::var("FASTSEARCH_E5_MODEL_ROOT").unwrap());
    let mut runtime = ProductionRuntime::open(
        ProductionConfig::new(&documents, &code, &service).with_e5_root(&model),
    )
    .unwrap();
    runtime.rebuild().unwrap();
    assert!(matches!(
        runtime
            .status()
            .into_iter()
            .find(|s| s.capability() == Capability::VectorRetrieval)
            .unwrap()
            .state(),
        CapabilityState::Available { .. }
    ));
    let mutation = model.join("e2-provider-mutation.tmp");
    fs::write(&mutation, "mutation").unwrap();
    let response = runtime
        .search(&SearchQuery::new("semantic recovery", SearchMode::Balanced).unwrap())
        .unwrap();
    assert!(
        runtime.get(&id).unwrap().is_some(),
        "authority survives derived provider failure"
    );
    assert!(
        response
            .hits()
            .iter()
            .all(|hit| hit.channel() != fastsearch::domain::RetrievalChannel::Vector)
    );
    assert_ne!(response.freshness(), IndexFreshness::Current);
    fs::remove_file(&mutation).unwrap();
    drop(runtime);
    let mut recovered = ProductionRuntime::open(
        ProductionConfig::new(&documents, &code, &service).with_e5_root(&model),
    )
    .unwrap();
    recovered.rebuild().unwrap();
    assert!(matches!(
        recovered
            .status()
            .into_iter()
            .find(|s| s.capability() == Capability::VectorRetrieval)
            .unwrap()
            .state(),
        CapabilityState::Available { .. }
    ));
}
