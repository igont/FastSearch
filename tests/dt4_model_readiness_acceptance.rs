use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Output},
    thread,
    time::{Instant, SystemTime},
};

use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{REPLACEFILE_WRITE_THROUGH, ReplaceFileW};

type AnyError = Box<dyn std::error::Error>;

#[test]
fn model_status_is_confined_to_product_home_and_cleanup_is_read_back() -> Result<(), AnyError> {
    let root = std::env::temp_dir().join(format!(
        "fastsearch-a5-cwd-cleanup-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_nanos()
    ));
    let safe_cwd = root.join("safe-cwd");
    let home = root.join("product-home");
    let workspace = root.join("protected/workspace");
    let projections = [
        root.join("protected/projection-arctic"),
        root.join("protected/projection-e5"),
        root.join("protected/projection-nomic"),
    ];
    fs::create_dir_all(&safe_cwd)?;
    let before = protected_snapshots(&workspace, &projections)?;
    let output = Command::new(env!("CARGO_BIN_EXE_fastsearch"))
        .args(["models", "status", "--json"])
        .current_dir(&safe_cwd)
        .env("FASTSEARCH_HOME", &home)
        .env("HF_ENDPOINT", "http://127.0.0.1:9")
        .output()?;
    assert!(!output.status.success());
    assert_eq!(before, protected_snapshots(&workspace, &projections)?);
    assert_eq!(fs::read_dir(&safe_cwd)?.count(), 0);
    if home.exists() {
        fs::remove_dir_all(&home)?;
    }
    fs::remove_dir(&safe_cwd)?;
    let receipt = json!({
        "target_home": home,
        "target_home_absent": !home.exists(),
        "safe_cwd_absent": !safe_cwd.exists(),
        "protected_paths": protected_snapshots(&workspace, &projections)?,
        "status": "PASS",
    });
    let receipt_path = root.join("cleanup.json");
    atomic_write_json(&receipt_path, &receipt)?;
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(&receipt_path)?)?,
        receipt
    );
    fs::remove_dir_all(root)?;
    Ok(())
}

#[derive(Deserialize)]
struct Catalog {
    models: Vec<Model>,
}

#[derive(Deserialize)]
struct Model {
    slug: String,
    repository: String,
    revision: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    path: String,
    bytes: u64,
    sha256: String,
}

#[test]
#[ignore = "requires the release binary and an existing verified raw-artifact store"]
fn ts_dt4_02_release_model_readiness() -> Result<(), AnyError> {
    let binary = required_path("FASTSEARCH_DT4_B_RELEASE_BINARY")?;
    let target_home = required_path("FASTSEARCH_DT4_B_HOME")?;
    let verified_home = required_path("FASTSEARCH_DT4_B_VERIFIED_HOME")?;
    let oracle = required_path("FASTSEARCH_DT4_B_ORACLE")?;
    let evidence_out = required_path("FASTSEARCH_DT4_B_EVIDENCE_OUT")?;
    let safe_cwd = required_path("FASTSEARCH_DT4_B_SAFE_CWD")?;
    let workspace = required_path("FASTSEARCH_DT4_B_WORKSPACE")?;
    let projection_roots = required_paths("FASTSEARCH_DT4_B_PROJECTION_ROOTS", 3)?;
    let started = Instant::now();
    if target_home.exists() {
        return Err("FASTSEARCH_DT4_B_HOME must name a new isolated directory".into());
    }
    if !binary.is_file() || !oracle.is_file() {
        return Err("release binary and oracle must be regular files".into());
    }

    if safe_cwd.exists() || workspace.exists() || projection_roots.iter().any(|path| path.exists())
    {
        return Err(
            "safe cwd, workspace, and three projection sentinels must be absent initially".into(),
        );
    }
    fs::create_dir_all(&safe_cwd)?;
    let protected_before = protected_snapshots(&workspace, &projection_roots)?;
    let result = execute(&binary, &target_home, &verified_home, &oracle, &safe_cwd);
    let (status, details) = match result {
        Ok(details) => ("PASS", details),
        Err(error) => (
            "FAIL",
            json!({
                "error": error.to_string(),
                "retained_home": target_home,
            }),
        ),
    };
    let evidence = json!({
        "schema": "FASTSEARCH-DT4-MODEL-READINESS-v1",
        "gate": "G-TS-DT4-02",
        "status": status,
        "candidate_binary": file_observation(&binary)?,
        "oracle": file_observation(&oracle)?,
        "environment": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "network_policy": "full-model commands use an unreachable loopback endpoint",
            "seed_policy": "exact raw allowlist only; no refs, markers, state, or locks",
        },
        "duration_ms": started.elapsed().as_millis(),
        "details": details,
        "protected_paths": {
            "safe_child_cwd": safe_cwd,
            "before": protected_before,
            "after": protected_snapshots(&workspace, &projection_roots)?,
        },
        "cleanup_receipt": evidence_out.with_extension("cleanup.json"),
    });
    atomic_write_json(&evidence_out, &evidence)?;
    let reread: Value = serde_json::from_slice(&fs::read(&evidence_out)?)?;
    if reread != evidence {
        return Err("atomic evidence readback differs".into());
    }
    if status == "PASS" {
        fs::remove_dir_all(&target_home)?;
        fs::remove_dir_all(&safe_cwd)?;
        let receipt_path = evidence_out.with_extension("cleanup.json");
        let receipt = json!({
            "schema": "FASTSEARCH-DT4-MODEL-READINESS-CLEANUP-v1",
            "evidence_sha256": sha256_file(&evidence_out)?,
            "target_home": target_home,
            "target_home_absent": !target_home.exists(),
            "safe_cwd_absent": !safe_cwd.exists(),
            "protected_paths_after": protected_snapshots(&workspace, &projection_roots)?,
            "status": "PASS",
        });
        atomic_write_json(&receipt_path, &receipt)?;
        if serde_json::from_slice::<Value>(&fs::read(&receipt_path)?)? != receipt {
            return Err("cleanup receipt readback differs".into());
        }
        return Ok(());
    }
    Err("G-TS-DT4-02 failed; isolated home retained".into())
}

fn execute(
    binary: &Path,
    target_home: &Path,
    verified_home: &Path,
    oracle: &Path,
    safe_cwd: &Path,
) -> Result<Value, AnyError> {
    let catalog_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("evidence/dt4/fixtures/ts-dt4-01/model-manifest.json");
    let catalog: Catalog = serde_json::from_slice(&fs::read(&catalog_path)?)?;
    let source_root = verified_home.join("models/production");
    let target_root = target_home.join("models/production");
    let source_before = seed_raw_allowlist(&catalog, &source_root, &target_root)?;
    assert_forbidden_state_absent(&target_root)?;

    let mut commands = Vec::new();
    let initial = run(
        binary,
        target_home,
        &source_root,
        safe_cwd,
        &["models", "prepare", "--json"],
    )?;
    assert_success_ready(&initial, 0)?;
    commands.push(command_observation("initial_prepare", &initial)?);
    let initial_status = run(
        binary,
        target_home,
        &source_root,
        safe_cwd,
        &["models", "status", "--json"],
    )?;
    let generation = assert_success_ready(&initial_status, 0)?;
    commands.push(command_observation("reopen_status", &initial_status)?);

    let mut recoveries = Vec::new();
    for model in &catalog.models {
        let asset = model.assets.first().ok_or("model has no assets")?;
        let target = artifact_path(&target_root, model, asset);
        let pre_snapshot = target_allowlist_metadata(&catalog, &target_root)?;
        corrupt_first_byte(&target)?;
        let damaged_sha256 = sha256_file(&target)?;
        let not_ready = run(
            binary,
            target_home,
            &source_root,
            safe_cwd,
            &["models", "status", "--json"],
        )?;
        if not_ready.status.success() {
            return Err(format!("{} corruption remained READY", model.slug).into());
        }
        let repaired = run(
            binary,
            target_home,
            &source_root,
            safe_cwd,
            &["models", "prepare", "--json"],
        )?;
        let repaired_generation = assert_success_ready(&repaired, 0)?;
        if sha256_file(&target)? != asset.sha256 {
            return Err(format!("{} was not restored exactly", model.slug).into());
        }
        let post_snapshot = target_allowlist_metadata(&catalog, &target_root)?;
        let damaged_key = format!("{}/{}", model.slug, asset.path);
        for (path, before) in &pre_snapshot {
            if path != &damaged_key && post_snapshot.get(path) != Some(before) {
                return Err(format!("unrelated target artifact changed: {path}").into());
            }
        }
        recoveries.push(json!({
            "role": model.slug,
            "artifact": asset.path,
            "not_ready": command_observation("status_after_corruption", &not_ready)?,
            "repair": command_observation("targeted_local_repair", &repaired)?,
            "generation": repaired_generation,
            "source_bytes_unchanged": true,
            "pre_target_allowlist": pre_snapshot,
            "damaged_raw": {
                "path": format!("{}/{}", model.slug, asset.path),
                "expected_sha256": asset.sha256,
                "observed_sha256": damaged_sha256,
            },
            "post_target_allowlist": post_snapshot,
            "unchanged_raw_count": 17,
        }));
    }

    let left_binary = binary.to_path_buf();
    let left_home = target_home.to_path_buf();
    let left_store = source_root.clone();
    let left_cwd = safe_cwd.to_path_buf();
    let right_binary = binary.to_path_buf();
    let right_home = target_home.to_path_buf();
    let right_store = source_root.clone();
    let right_cwd = safe_cwd.to_path_buf();
    let left = thread::spawn(move || {
        run(
            &left_binary,
            &left_home,
            &left_store,
            &left_cwd,
            &["models", "status", "--json"],
        )
        .map_err(|error| error.to_string())
    });
    let right = thread::spawn(move || {
        run(
            &right_binary,
            &right_home,
            &right_store,
            &right_cwd,
            &["models", "status", "--json"],
        )
        .map_err(|error| error.to_string())
    });
    let left = left
        .join()
        .map_err(|_| "first status process panicked")?
        .map_err(|error| -> AnyError { error.into() })?;
    let right = right
        .join()
        .map_err(|_| "second status process panicked")?
        .map_err(|error| -> AnyError { error.into() })?;
    let left_generation = assert_success_ready(&left, 0)?;
    let right_generation = assert_success_ready(&right, 0)?;
    if left_generation != right_generation || left_generation != generation {
        return Err("parallel processes observed different model-set generations".into());
    }

    let source_after = observe_allowlist(&catalog, &source_root)?;
    if source_before != source_after {
        return Err("verified source store changed during acceptance".into());
    }
    let transport = loopback_range_fixture()?;
    let aggregate = fs::read_dir(target_root.join("model-set.roles"))?.count();
    if aggregate != 4 {
        return Err(format!("expected four role marker roots, got {aggregate}").into());
    }
    Ok(json!({
        "catalog": file_observation(&catalog_path)?,
        "raw_allowlist": source_before,
        "commands": commands,
        "corruption_recovery": recoveries,
        "parallel_reopen": {
            "processes": 2,
            "generation": left_generation,
            "left": command_observation("parallel_status_1", &left)?,
            "right": command_observation("parallel_status_2", &right)?,
            "role_marker_roots": aggregate,
        },
        "transport_fixture": transport,
        "oracle_sha256": sha256_file(oracle)?,
        "source_store_unchanged": true,
        "workspace_and_projections": "not created or addressed by model commands",
        "isolated_home_cleanup": "scheduled after atomic evidence readback",
    }))
}

fn run(
    binary: &Path,
    home: &Path,
    store: &Path,
    cwd: &Path,
    args: &[&str],
) -> Result<Output, AnyError> {
    Ok(Command::new(binary)
        .args(args)
        .current_dir(cwd)
        .env("FASTSEARCH_HOME", home)
        .env("FASTSEARCH_VERIFIED_ARTIFACT_STORE", store)
        .env("HF_ENDPOINT", "http://127.0.0.1:9")
        .output()?)
}

fn target_allowlist_metadata(
    catalog: &Catalog,
    root: &Path,
) -> Result<BTreeMap<String, Value>, AnyError> {
    let mut entries = BTreeMap::new();
    for model in &catalog.models {
        for asset in &model.assets {
            let metadata = fs::metadata(artifact_path(root, model, asset))?;
            entries.insert(
                format!("{}/{}", model.slug, asset.path),
                json!({
                    "bytes": metadata.len(),
                    "modified_unix_nanos": metadata.modified()?.duration_since(SystemTime::UNIX_EPOCH)?.as_nanos(),
                    "expected_sha256": asset.sha256,
                }),
            );
        }
    }
    Ok(entries)
}

fn protected_snapshots(workspace: &Path, projections: &[PathBuf]) -> Result<Value, AnyError> {
    Ok(json!({
        "workspace": path_snapshot(workspace)?,
        "projections": projections.iter().map(|path| path_snapshot(path)).collect::<Result<Vec<_>, _>>()?,
    }))
}

fn path_snapshot(path: &Path) -> Result<Value, AnyError> {
    if !path.exists() {
        return Ok(json!({"path": path, "exists": false}));
    }
    let files = walk_files(path)?;
    Ok(json!({
        "path": path,
        "exists": true,
        "files": files.iter().map(|file| file_observation(file)).collect::<Result<Vec<_>, _>>()?,
    }))
}

fn assert_success_ready(output: &Output, downloaded_bytes: u64) -> Result<String, AnyError> {
    if !output.status.success() {
        return Err(format!(
            "command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    let value: Value = serde_json::from_slice(&output.stdout)?;
    if value["ready"] != true || value["roles"].as_array().map(Vec::len) != Some(4) {
        return Err("command did not report 4/4 READY".into());
    }
    if value["downloaded_bytes"].as_u64() != Some(downloaded_bytes) {
        return Err(format!("unexpected downloaded_bytes: {}", value["downloaded_bytes"]).into());
    }
    value["generation"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "generation missing".into())
}

fn command_observation(label: &str, output: &Output) -> Result<Value, AnyError> {
    Ok(json!({
        "label": label,
        "exit_code": output.status.code(),
        "stdout_json": serde_json::from_slice::<Value>(&output.stdout).ok(),
        "stderr": String::from_utf8_lossy(&output.stderr),
    }))
}

fn seed_raw_allowlist(
    catalog: &Catalog,
    source_root: &Path,
    target_root: &Path,
) -> Result<BTreeMap<String, Value>, AnyError> {
    let source = observe_allowlist(catalog, source_root)?;
    for model in &catalog.models {
        for asset in &model.assets {
            let from = artifact_path(source_root, model, asset);
            let to = artifact_path(target_root, model, asset);
            fs::create_dir_all(to.parent().ok_or("artifact has no parent")?)?;
            fs::copy(&from, &to)?;
            if fs::metadata(&to)?.len() != asset.bytes || sha256_file(&to)? != asset.sha256 {
                return Err(format!("seed copy differs: {}/{}", model.slug, asset.path).into());
            }
        }
    }
    Ok(source)
}

fn observe_allowlist(catalog: &Catalog, root: &Path) -> Result<BTreeMap<String, Value>, AnyError> {
    let mut observed = BTreeMap::new();
    for model in &catalog.models {
        for asset in &model.assets {
            let path = artifact_path(root, model, asset);
            let bytes = fs::metadata(&path)?.len();
            let sha256 = sha256_file(&path)?;
            if bytes != asset.bytes || sha256 != asset.sha256 {
                return Err(
                    format!("verified store mismatch: {}/{}", model.slug, asset.path).into(),
                );
            }
            observed.insert(
                format!("{}/{}", model.slug, asset.path),
                json!({
                    "bytes": bytes,
                    "sha256": sha256,
                }),
            );
        }
    }
    Ok(observed)
}

fn artifact_path(root: &Path, model: &Model, asset: &Asset) -> PathBuf {
    root.join("artifacts")
        .join(&model.slug)
        .join("hub")
        .join(format!("models--{}", model.repository.replace('/', "--")))
        .join("snapshots")
        .join(&model.revision)
        .join(&asset.path)
}

fn assert_forbidden_state_absent(root: &Path) -> Result<(), AnyError> {
    for forbidden in [
        "model-set.ready.json",
        "model-set.roles",
        "model-set.install.lock",
    ] {
        if root.join(forbidden).exists() {
            return Err(format!("seed unexpectedly contains {forbidden}").into());
        }
    }
    for entry in walk_files(root)? {
        let normalized = entry.to_string_lossy().replace('\\', "/");
        if normalized.contains("/refs/") || normalized.ends_with("/model-manifest.json") {
            return Err(format!("seed unexpectedly contains {normalized}").into());
        }
    }
    Ok(())
}

fn corrupt_first_byte(path: &Path) -> Result<(), AnyError> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let mut byte = [0_u8; 1];
    file.read_exact(&mut byte)?;
    byte[0] ^= 0xff;
    file.rewind()?;
    file.write_all(&byte)?;
    file.flush()?;
    file.sync_all()?;
    Ok(())
}

fn loopback_range_fixture() -> Result<Value, AnyError> {
    const BODY: &[u8] = b"0123456789abcdef";
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let server = thread::spawn(move || -> Result<Vec<Value>, String> {
        let mut log = Vec::new();
        for (offset, response) in [(0_usize, &BODY[..6]), (6, &BODY[6..])] {
            let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
            let request = read_headers(&mut stream).map_err(|error| error.to_string())?;
            let range = request
                .lines()
                .find(|line| line.to_ascii_lowercase().starts_with("range:"));
            if (offset == 0 && range.is_some()) || (offset == 6 && range != Some("Range: bytes=6-"))
            {
                return Err(format!("unexpected Range header: {range:?}"));
            }
            let status = if offset == 0 {
                "200 OK"
            } else {
                "206 Partial Content"
            };
            let extra = if offset == 0 {
                String::new()
            } else {
                "Content-Range: bytes 6-15/16\r\n".to_owned()
            };
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
                response.len()
            )
            .map_err(|error| error.to_string())?;
            stream
                .write_all(response)
                .map_err(|error| error.to_string())?;
            log.push(
                json!({"offset": offset, "range": range, "response_body_bytes": response.len()}),
            );
        }
        Ok(log)
    });
    let mut accumulated = Vec::new();
    for range in [None, Some("bytes=6-")] {
        let mut stream = TcpStream::connect(address)?;
        write!(stream, "GET /fixture HTTP/1.1\r\nHost: {address}\r\n")?;
        if let Some(range) = range {
            write!(stream, "Range: {range}\r\n")?;
        }
        write!(stream, "Connection: close\r\n\r\n")?;
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes)?;
        let split = bytes
            .windows(4)
            .position(|value| value == b"\r\n\r\n")
            .ok_or("fixture response headers missing")?
            + 4;
        accumulated.extend_from_slice(&bytes[split..]);
    }
    let log = server
        .join()
        .map_err(|_| "fixture proxy panicked")?
        .map_err(|error| -> AnyError { error.into() })?;
    if accumulated != BODY {
        return Err("Range fixture repeated or lost bytes".into());
    }
    Ok(json!({"fixture_bytes": BODY.len(), "requests": log, "prefix_retransmitted": false}))
}

fn read_headers(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    let mut byte = [0_u8; 1];
    while !bytes.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte)?;
        bytes.push(byte[0]);
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn walk_files(root: &Path) -> Result<Vec<PathBuf>, AnyError> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                pending.push(entry.path());
            } else {
                files.push(entry.path());
            }
        }
    }
    Ok(files)
}

fn sha256_file(path: &Path) -> Result<String, AnyError> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn file_observation(path: &Path) -> Result<Value, AnyError> {
    Ok(json!({"path": path, "bytes": fs::metadata(path)?.len(), "sha256": sha256_file(path)?}))
}

fn atomic_write_json(path: &Path, value: &Value) -> Result<(), AnyError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut file = File::create(&temporary)?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.flush()?;
    file.sync_all()?;
    replace_atomically(path, &temporary)?;
    Ok(())
}

#[cfg(windows)]
fn replace_atomically(target: &Path, replacement: &Path) -> std::io::Result<()> {
    if !target.exists() {
        return fs::rename(replacement, target);
    }
    let target = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let replacement = replacement
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: both paths are live NUL-terminated UTF-16 buffers and the call
    // returns before either buffer is dropped.
    let result = unsafe {
        ReplaceFileW(
            target.as_ptr(),
            replacement.as_ptr(),
            std::ptr::null(),
            REPLACEFILE_WRITE_THROUGH,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if result == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_atomically(target: &Path, replacement: &Path) -> std::io::Result<()> {
    fs::rename(replacement, target)
}

fn required_path(name: &str) -> Result<PathBuf, AnyError> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{name} is required").into())
}

fn required_paths(name: &str, count: usize) -> Result<Vec<PathBuf>, AnyError> {
    let value = std::env::var(name)?;
    let paths = value.split(';').map(PathBuf::from).collect::<Vec<_>>();
    if paths.len() != count {
        return Err(
            format!("{name} must contain exactly {count} semicolon-separated paths").into(),
        );
    }
    Ok(paths)
}
