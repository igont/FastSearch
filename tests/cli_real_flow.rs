use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new() -> Self {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time is after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("fastsearch-e2-cli-{suffix}"));
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

fn run(arguments: &[String]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fastsearch"))
        .args(arguments)
        .output()
        .expect("real CLI starts")
}

fn args(parts: &[&str]) -> Vec<String> {
    parts.iter().map(ToString::to_string).collect()
}

#[test]
fn real_cli_lifecycle_modes_get_and_cross_process_failure_recovery_are_observable() {
    let temporary = TemporaryDirectory::new();
    let source = temporary.child("source");
    let service = temporary.child("service");
    fs::create_dir_all(&source).expect("source directory");
    fs::write(
        source.join("guide.md"),
        "---\nalignment: CURRENT\n---\n# Документальный поиск\n\nРусская фраза для real CLI.",
    )
    .expect("source fixture");

    let source = source.to_string_lossy().into_owned();
    let service = service.to_string_lossy().into_owned();
    let init = run(&args(&["init", &source, &service]));
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );
    assert!(String::from_utf8_lossy(&init.stdout).contains("Source=Real"));

    let update = run(&args(&["index", "update", &source, &service]));
    assert!(
        update.status.success(),
        "{}",
        String::from_utf8_lossy(&update.stderr)
    );
    assert!(String::from_utf8_lossy(&update.stdout).contains("freshness=Current"));

    let search = run(&args(&[
        "search",
        &source,
        &service,
        "balanced",
        "\"Русская фраза\"",
    ]));
    assert!(
        search.status.success(),
        "{}",
        String::from_utf8_lossy(&search.stderr)
    );
    assert!(String::from_utf8_lossy(&search.stdout).contains("hits=1"));
    assert!(String::from_utf8_lossy(&search.stdout).contains("freshness=Current"));

    let current = run(&args(&[
        "search",
        &source,
        &service,
        "current",
        "\"Русская фраза\"",
    ]));
    assert!(current.status.success());
    let design = run(&args(&[
        "search",
        &source,
        &service,
        "design",
        "\"Русская фраза\"",
    ]));
    assert!(design.status.success());

    let id = String::from_utf8(search.stdout)
        .expect("UTF-8 output")
        .lines()
        .find_map(|line| line.strip_prefix("record="))
        .and_then(|line| line.split('\t').next())
        .expect("search reports stable id")
        .to_owned();
    let get = run(&args(&["get", &source, &service, &id]));
    assert!(
        get.status.success(),
        "{}",
        String::from_utf8_lossy(&get.stderr)
    );
    assert!(String::from_utf8_lossy(&get.stdout).contains(&format!("record={id}")));

    fs::write(
        std::path::Path::new(&source).join("guide.md"),
        "---\nalignment: CURRENT\n---\n# Документальный поиск\n\nИзменённая русская фраза для failure recovery.",
    )
    .expect("mutate source before controlled failure");

    let failure = run(&args(&[
        "index",
        "update",
        &source,
        &service,
        "--test-fail-projection",
    ]));
    assert!(!failure.status.success());
    assert!(!String::from_utf8_lossy(&failure.stdout).contains("freshness=Current"));

    let stale = run(&args(&["status", &source, &service]));
    assert!(stale.status.success());
    let stale_stdout = String::from_utf8_lossy(&stale.stdout);
    assert!(
        stale_stdout.contains("freshness=Stale") || stale_stdout.contains("freshness=Degraded")
    );
    assert!(!stale_stdout.contains("freshness=Current"));

    let rebuild = run(&args(&["index", "rebuild", &source, &service]));
    assert!(rebuild.status.success());
    assert!(String::from_utf8_lossy(&rebuild.stdout).contains("freshness=Current"));

    let recovered = run(&args(&["status", &source, &service]));
    assert!(recovered.status.success());
    assert!(String::from_utf8_lossy(&recovered.stdout).contains("freshness=Current"));
}

#[test]
fn real_cli_accepts_an_arbitrary_source_root_with_service_state_inside_its_reserved_zone() {
    let temporary = TemporaryDirectory::new();
    let source = temporary.child("replaceable-target-root");
    let service = source.join(".cfknowledge");
    fs::create_dir_all(&service).expect("reserved service zone");
    fs::write(
        source.join("guide.md"),
        "---\ntdr_refs: [TDR-1, TDR-2]\n---\n# Target document\n\ntargetdoc",
    )
    .expect("source fixture");
    fs::write(
        service.join("must-not-be-indexed.md"),
        "# Derived sentinel\n\nderivedsentinel",
    )
    .expect("derived sentinel");

    let source = source.to_string_lossy().into_owned();
    let service = service.to_string_lossy().into_owned();
    let rebuild = run(&args(&["index", "rebuild", &source, &service]));
    assert!(
        rebuild.status.success(),
        "{}",
        String::from_utf8_lossy(&rebuild.stderr)
    );
    assert!(String::from_utf8_lossy(&rebuild.stdout).contains("freshness=Current"));
    let marker_path = std::path::Path::new(&service)
        .join("lexical")
        .join("projection.marker");
    let marker_before = fs::read(&marker_path).expect("current projection marker");

    let unchanged_update = run(&args(&["index", "update", &source, &service]));
    assert!(unchanged_update.status.success());
    let marker_after = fs::read(&marker_path).expect("unchanged projection marker");
    assert_eq!(
        marker_after, marker_before,
        "an unchanged source set must not rebuild the current lexical projection"
    );

    let source_hit = run(&args(&[
        "search",
        &source,
        &service,
        "balanced",
        "targetdoc",
    ]));
    assert!(String::from_utf8_lossy(&source_hit.stdout).contains("hits=1"));

    let derived_miss = run(&args(&[
        "search",
        &source,
        &service,
        "balanced",
        "derivedsentinel",
    ]));
    assert!(String::from_utf8_lossy(&derived_miss.stdout).contains("hits=0"));
}

#[test]
fn fault_flag_is_literal_final_update_only_and_absent_from_normal_help() {
    let help = run(&args(&[]));
    assert_eq!(help.status.code(), Some(2));
    assert!(!String::from_utf8_lossy(&help.stderr).contains("--test-fail-projection"));

    for invalid in [
        args(&["init", "--test-fail-projection", "source", "service"]),
        args(&[
            "index",
            "rebuild",
            "source",
            "service",
            "--test-fail-projection",
        ]),
        args(&[
            "index",
            "update",
            "--test-fail-projection",
            "source",
            "service",
        ]),
        args(&[
            "index",
            "update",
            "source",
            "--test-fail-projection",
            "service",
        ]),
        args(&["status", "source", "service", "--test-fail-projection"]),
        args(&[
            "search",
            "source",
            "service",
            "balanced",
            "query",
            "--test-fail-projection",
        ]),
    ] {
        let output = run(&invalid);
        assert_eq!(output.status.code(), Some(2), "invalid={invalid:?}");
    }
}
