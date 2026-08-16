#![cfg(windows)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let nonce = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("fastsearch-e2-runner-{nanos}-{nonce}"));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn child(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn validate_runner(
    document: &Path,
    code: &Path,
    work: &Path,
    runtime_document: &Path,
    output: &Path,
) -> std::process::Output {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    Command::new("powershell")
        .args([
            "-NoProfile",
            "-File",
            repository
                .join("spikes/dt3-e2/run-release.ps1")
                .to_str()
                .unwrap(),
            "-Binary",
            env!("CARGO_BIN_EXE_fastsearch"),
            "-DocumentRoot",
            document.to_str().unwrap(),
            "-CodeRoot",
            code.to_str().unwrap(),
            "-WorkRoot",
            work.to_str().unwrap(),
            "-RuntimeDocumentRoot",
            runtime_document.to_str().unwrap(),
            "-OutputJson",
            output.to_str().unwrap(),
            "-ValidateOnly",
        ])
        .output()
        .unwrap()
}

#[test]
fn release_runner_rejects_two_way_overlap_and_junction_before_write() {
    let temp = Temp::new();
    let document = temp.child("document");
    let code = temp.child("code");
    let external = temp.child("external");
    fs::create_dir_all(&document).unwrap();
    fs::create_dir_all(&code).unwrap();
    fs::create_dir_all(&external).unwrap();
    fs::write(document.join("guide.md"), "# Guide").unwrap();
    fs::write(code.join("lib.rs"), "pub fn code() {}").unwrap();
    fs::write(external.join("sentinel.txt"), "unchanged").unwrap();

    let nested_code = document.join("nested-code");
    fs::create_dir_all(&nested_code).unwrap();
    fs::write(nested_code.join("lib.rs"), "pub fn nested() {}").unwrap();
    let overlap = validate_runner(
        &document,
        &nested_code,
        &temp.child("disjoint-work"),
        &temp.child("disjoint-runtime"),
        &temp.child("overlap.json"),
    );
    assert!(!overlap.status.success());
    assert!(
        String::from_utf8_lossy(&overlap.stderr).contains("pairwise disjoint"),
        "stderr bytes={:?}; stdout={}",
        overlap.stderr,
        String::from_utf8_lossy(&overlap.stdout)
    );

    let junction = temp.child("work-junction");
    let linked = Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&junction)
        .arg(&external)
        .output()
        .unwrap();
    assert!(linked.status.success());
    let junction_result = validate_runner(
        &document,
        &code,
        &junction,
        &temp.child("junction-runtime"),
        &temp.child("junction.json"),
    );
    assert!(!junction_result.status.success());
    assert!(String::from_utf8_lossy(&junction_result.stderr).contains("reparse point"));
    assert_eq!(
        fs::read_to_string(external.join("sentinel.txt")).unwrap(),
        "unchanged"
    );
    assert!(!external.join("owner.marker").exists());

    let old_work = temp.child("old-inside-work");
    let old_pattern = validate_runner(
        &document,
        &code,
        &old_work,
        &old_work.join("scaled-document-root"),
        &temp.child("old-pattern.json"),
    );
    assert!(!old_pattern.status.success());
    assert!(String::from_utf8_lossy(&old_pattern.stderr).contains("pairwise disjoint"));
    assert!(
        !old_work.exists(),
        "old inside-WorkRoot pattern must fail before write"
    );
}
