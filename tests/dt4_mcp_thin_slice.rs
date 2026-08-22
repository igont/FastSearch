use std::{
    collections::BTreeMap,
    env, fs,
    io::Read,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use fastsearch::{
    application::{ProductionRuntime, WorkspaceStore},
    domain::{EmbeddingModelId, IndexFreshness},
};
use rmcp::{
    ClientHandler, ServiceExt,
    model::{CallToolRequestParams, ClientInfo, JsonObject, ProtocolVersion},
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    process::{Child, Command},
};

const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(10);
const EOF_TIMEOUT: Duration = Duration::from_secs(10);
const BUSY_TIMEOUT: Duration = Duration::from_secs(2);
const SEARCH_WATCHDOG: Duration = Duration::from_secs(35);
const MODELS: [EmbeddingModelId; 3] = [
    EmbeddingModelId::SnowflakeArcticEmbedLV2,
    EmbeddingModelId::MultilingualE5Large,
    EmbeddingModelId::NomicEmbedTextV2Moe,
];

#[derive(Clone)]
struct VersionedClient;

impl ClientHandler for VersionedClient {
    fn get_info(&self) -> ClientInfo {
        let mut info = ClientInfo::default();
        info.protocol_version = ProtocolVersion::V_2025_11_25;
        info
    }
}
const PATH_ENVIRONMENT: &[&str] = &[
    "FASTSEARCH_DT4_FIXTURE_ROOT",
    "FASTSEARCH_DT4_MODEL_CACHE",
    "FASTSEARCH_DT4_ORACLE",
    "FASTSEARCH_DT4_EVIDENCE_OUT",
    "FASTSEARCH_A5_RELEASE_BINARY",
    "FASTSEARCH_A5_WORKSPACE",
    "FASTSEARCH_A5_UNREADY_WORKSPACE",
    "FASTSEARCH_A5_QUERY",
    "FASTSEARCH_A5_ORACLE",
    "HF_HOME",
    "HF_HUB_CACHE",
    "HUGGINGFACE_HUB_CACHE",
    "TRANSFORMERS_CACHE",
    "LOCALAPPDATA",
    "USERPROFILE",
];

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires pinned TS-DT4-01 fixture, model cache and release binary"]
async fn mcp_search_crosses_three_projections_and_qwen() -> Result<(), Box<dyn std::error::Error>> {
    let run = IsolatedRun::prepare()?;
    let before_free = free_physical_memory_bytes()?;
    let started = Instant::now();
    let transport = run.transport()?;
    let monitor = ChildWorkingSetMonitor::start(transport.id().ok_or("server process id")?);
    let client =
        tokio::time::timeout(INITIALIZE_TIMEOUT, VersionedClient.serve(transport)).await??;
    assert_eq!(
        client
            .peer_info()
            .ok_or("initialized server")?
            .protocol_version,
        ProtocolVersion::V_2025_11_25
    );
    let tools = client.list_all_tools().await?;
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "search");
    let query = fs::read_to_string(run.fixture.join("query.txt"))?;
    let response = tokio::time::timeout(
        SEARCH_WATCHDOG,
        client.call_tool(
            CallToolRequestParams::new("search")
                .with_arguments(JsonObject::from_iter([("query".to_owned(), json!(query))])),
        ),
    )
    .await??;
    let elapsed = started.elapsed();
    assert_eq!(response.is_error, Some(false));
    let public = response.structured_content.ok_or("structured response")?;
    assert_public_oracle(&public, &run.oracle)?;
    let serialized_response_bytes = serde_json::to_vec(&public)?.len();
    assert!(serialized_response_bytes <= 131_072);
    assert!(elapsed <= Duration::from_secs(35));
    client.cancel().await?;
    let peak_working_set_bytes = monitor.finish()?;
    assert!(peak_working_set_bytes <= 8_u64 * 1024 * 1024 * 1024);
    run.assert_isolated()?;

    fs::write(
        &run.evidence_out,
        serde_json::to_vec_pretty(&json!({
            "schema": 1,
            "gate": "G-RESOURCE@TS-DT4-01",
            "candidate_input": "isolated-release-mcp",
            "models": run.revisions,
            "corpus_generation": 1,
            "candidate_slots": 15,
            "public_results": 6,
            "elapsed_ms": elapsed.as_millis(),
            "serialized_response_bytes": serialized_response_bytes,
            "peak_working_set_bytes": peak_working_set_bytes,
            "free_physical_memory_before_bytes": before_free,
            "free_physical_memory_after_bytes": free_physical_memory_bytes()?,
            "limits": {
                "deadline_ms": 30_000,
                "initialize_timeout_ms": 10_000,
                "eof_timeout_ms": 10_000,
                "busy_timeout_ms": 2_000,
                "search_watchdog_ms": 35_000,
                "working_set_bytes": 8_u64 * 1024 * 1024 * 1024,
                "free_before_bytes": 12_u64 * 1024 * 1024 * 1024,
                "reserve_bytes": 4_u64 * 1024 * 1024 * 1024,
                "response_bytes": 131_072
            },
            "isolation": {
                "unique_workspace": true,
                "unique_product_home": true,
                "preparation_outside_server": true,
                "child_product_path_override": ["FASTSEARCH_HOME"],
                "legacy_and_test_environment_removed": true,
                "global_product_home_unchanged": true,
                "global_huggingface_cache_unchanged": true,
                "product_tree_unchanged_during_child": true,
                "model_cache_template_unchanged": true
            }
        }))?,
    )?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires pinned TS-DT4-01 fixture, model cache and release binary"]
async fn cancelled_call_emits_no_late_response() -> Result<(), Box<dyn std::error::Error>> {
    let run = IsolatedRun::prepare()?;
    let mut child = run.spawn();
    let mut writer = child.stdin.take().ok_or("server stdin")?;
    let mut reader = BufReader::new(child.stdout.take().ok_or("server stdout")?);
    initialize(&mut writer, &mut reader).await?;
    let query = fs::read_to_string(run.fixture.join("query.txt"))?;
    send_json(&mut writer, &json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search","arguments":{"query":query}}})).await?;
    send_json(
        &mut writer,
        &json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":2}}),
    )
    .await?;
    send_json(
        &mut writer,
        &json!({"jsonrpc":"2.0","id":3,"method":"ping"}),
    )
    .await?;
    let ids = collect_ids(&mut reader, SEARCH_WATCHDOG).await?;
    assert!(ids.contains(&3));
    assert!(!ids.contains(&2));
    drop(writer);
    wait_for_child(&mut child).await?;
    run.assert_isolated()?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires pinned TS-DT4-01 fixture, model cache and release binary"]
async fn eof_stops_server() -> Result<(), Box<dyn std::error::Error>> {
    let run = IsolatedRun::prepare()?;
    let mut child = run.spawn();
    drop(child.stdin.take());
    let mut stdout = child.stdout.take().ok_or("server stdout")?;
    let (status, stdout) = tokio::time::timeout(EOF_TIMEOUT, async {
        let mut output = String::new();
        stdout.read_to_string(&mut output).await?;
        Ok::<_, std::io::Error>((child.wait().await?, output))
    })
    .await??;
    assert!(status.success());
    assert!(stdout.is_empty());
    run.assert_isolated()?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires pinned TS-DT4-01 fixture, model cache and release binary"]
async fn third_call_is_busy() -> Result<(), Box<dyn std::error::Error>> {
    let run = IsolatedRun::prepare()?;
    let mut child = run.spawn();
    let mut writer = child.stdin.take().ok_or("server stdin")?;
    let mut reader = BufReader::new(child.stdout.take().ok_or("server stdout")?);
    initialize(&mut writer, &mut reader).await?;
    let query = fs::read_to_string(run.fixture.join("query.txt"))?;
    for id in 2..=4 {
        send_json(&mut writer, &json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"search","arguments":{"query":query}}})).await?;
        if id < 4 {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
    let started = Instant::now();
    let busy = read_id(&mut reader, 4, BUSY_TIMEOUT).await?;
    assert!(started.elapsed() <= BUSY_TIMEOUT);
    assert_eq!(busy["result"]["structuredContent"]["error"]["code"], "BUSY");
    assert_eq!(
        busy["result"]["structuredContent"]["error"]["retryable"],
        true
    );
    for id in [2, 3] {
        send_json(
            &mut writer,
            &json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":id}}),
        )
        .await?;
    }
    let ids = collect_ids(&mut reader, SEARCH_WATCHDOG).await?;
    assert!(!ids.contains(&2));
    assert!(!ids.contains(&3));
    drop(writer);
    wait_for_child(&mut child).await?;
    run.assert_isolated()?;
    Ok(())
}

struct IsolatedRun {
    root: PathBuf,
    fixture: PathBuf,
    product_home: PathBuf,
    workspace: PathBuf,
    evidence_out: PathBuf,
    oracle: Value,
    revisions: BTreeMap<String, String>,
    global_before: Vec<(String, u64, u128)>,
    global_hf_before: Vec<(String, u64, u128)>,
    repository_before: Vec<(String, u64, u128)>,
    cache_before: Vec<(String, u64, u128)>,
    model_cache: PathBuf,
}

impl IsolatedRun {
    fn prepare() -> Result<Self, Box<dyn std::error::Error>> {
        let fixture = required_path("FASTSEARCH_DT4_FIXTURE_ROOT")?;
        let model_cache = required_path("FASTSEARCH_DT4_MODEL_CACHE")?;
        let oracle_path = required_path("FASTSEARCH_DT4_ORACLE")?;
        let evidence_out = absolute_output("FASTSEARCH_DT4_EVIDENCE_OUT")?;
        let oracle: Value = serde_json::from_slice(&fs::read(&oracle_path)?)?;
        verify_fixture(&fixture, &oracle)?;
        verify_model_cache(&fixture, &model_cache)?;
        let global = env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|path| path.join("FastSearch"));
        let global_before = global
            .as_deref()
            .map(fingerprint)
            .transpose()?
            .unwrap_or_default();
        let global_hf = env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .map(|path| path.join(".cache/huggingface"));
        let global_hf_before = global_hf
            .as_deref()
            .map(fingerprint)
            .transpose()?
            .unwrap_or_default();
        let repository_before = product_tree_fingerprint(Path::new(env!("CARGO_MANIFEST_DIR")))?;
        let cache_before = fingerprint(&model_cache)?;
        let root = unique_temp();
        let workspace = root.join("workspace");
        let product_home = root.join("product-home");
        copy_tree(&fixture.join("prepared-workspace"), &workspace)?;
        copy_tree(&model_cache, &product_home)?;
        verify_model_cache(&fixture, &product_home)?;

        let startup: Value =
            serde_json::from_slice(&fs::read(fixture.join("startup-manifest.json"))?)?;
        let manifest: Value =
            serde_json::from_slice(&fs::read(fixture.join("model-manifest.json"))?)?;
        let revisions = manifest["models"]
            .as_array()
            .ok_or("models")?
            .iter()
            .map(|model| {
                Ok((
                    model["slug"].as_str().ok_or("slug")?.to_owned(),
                    model["revision"].as_str().ok_or("revision")?.to_owned(),
                ))
            })
            .collect::<Result<BTreeMap<_, _>, Box<dyn std::error::Error>>>()?;
        let previous_home = env::var_os("FASTSEARCH_HOME");
        let previous_hf = env::var_os("HF_HOME");
        // SAFETY: every ignored integration invocation is exact and single-threaded.
        unsafe {
            env::set_var("FASTSEARCH_HOME", &product_home);
            env::set_var("HF_HOME", product_home.join("models/huggingface"));
        }
        let store = WorkspaceStore::open(&workspace)?;
        let runtime = ProductionRuntime::open(store.production_config())?;
        for projection in startup["projections"].as_array().ok_or("projections")? {
            let slug = projection["model_slug"].as_str().ok_or("projection slug")?;
            let revision = projection["model_revision"]
                .as_str()
                .ok_or("projection revision")?;
            assert_eq!(revisions.get(slug).map(String::as_str), Some(revision));
            assert_eq!(projection["corpus_generation"], 1);
            assert!(
                workspace
                    .join(".fastsearch/local")
                    .join(
                        projection["relative_path"]
                            .as_str()
                            .ok_or("projection path")?
                    )
                    .is_dir()
            );
        }
        for model in MODELS {
            let status = runtime.model_partition_status(model);
            assert_eq!(status.freshness(), IndexFreshness::Current);
            assert_eq!(status.state_generation(), 1);
            assert_eq!(status.projection_generation(), Some(1));
        }
        // SAFETY: restore the process environment before the test starts its child.
        unsafe {
            restore("FASTSEARCH_HOME", previous_home);
            restore("HF_HOME", previous_hf);
        }
        Ok(Self {
            root,
            fixture,
            product_home,
            workspace,
            evidence_out,
            oracle,
            revisions,
            global_before,
            global_hf_before,
            repository_before,
            cache_before,
            model_cache,
        })
    }

    fn spawn(&self) -> Child {
        self.command().spawn().expect("isolated release server")
    }

    fn transport(&self) -> Result<TokioChildProcess, std::io::Error> {
        TokioChildProcess::new(self.command().configure(|command| {
            command.stderr(Stdio::piped());
        }))
    }

    fn command(&self) -> Command {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        let binary = repo
            .parent()
            .expect("repo parent")
            .join(".cargo-target/FastSearch/release/fastsearch.exe");
        assert!(
            binary.is_file(),
            "release binary missing: {}",
            binary.display()
        );
        let mut command = Command::new(binary);
        command
            .arg("mcp")
            .arg("--workspace")
            .arg(&self.workspace)
            .current_dir(&self.workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for name in PATH_ENVIRONMENT {
            command.env_remove(name);
        }
        command
            .env("FASTSEARCH_HOME", &self.product_home)
            .env("HF_HUB_OFFLINE", "1")
            .env("TRANSFORMERS_OFFLINE", "1");
        command
    }

    fn assert_isolated(&self) -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(fingerprint(&self.model_cache)?, self.cache_before);
        let global = env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|path| path.join("FastSearch"));
        assert_eq!(
            global
                .as_deref()
                .map(fingerprint)
                .transpose()?
                .unwrap_or_default(),
            self.global_before
        );
        let global_hf = env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .map(|path| path.join(".cache/huggingface"));
        assert_eq!(
            global_hf
                .as_deref()
                .map(fingerprint)
                .transpose()?
                .unwrap_or_default(),
            self.global_hf_before
        );
        assert_eq!(
            product_tree_fingerprint(Path::new(env!("CARGO_MANIFEST_DIR")))?,
            self.repository_before
        );
        assert!(self.root.is_dir());
        Ok(())
    }
}

impl Drop for IsolatedRun {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

unsafe fn restore(name: &str, value: Option<std::ffi::OsString>) {
    if let Some(value) = value {
        unsafe { env::set_var(name, value) };
    } else {
        unsafe { env::remove_var(name) };
    }
}

fn required_path(name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(fs::canonicalize(
        env::var_os(name).ok_or_else(|| format!("{name} is required"))?,
    )?)
}
fn absolute_output(name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = PathBuf::from(env::var_os(name).ok_or_else(|| format!("{name} is required"))?);
    if !path.is_absolute() {
        return Err(format!("{name} must be absolute").into());
    }
    Ok(path)
}
fn unique_temp() -> PathBuf {
    let root = env::temp_dir().join(format!(
        "fastsearch-dt4-thin-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("temp root");
    root
}

fn verify_fixture(root: &Path, oracle: &Value) -> Result<(), Box<dyn std::error::Error>> {
    let startup: Value = serde_json::from_slice(&fs::read(root.join("startup-manifest.json"))?)?;
    for (field, file) in [
        ("corpus_manifest_sha256", "corpus-manifest.json"),
        ("model_manifest_sha256", "model-manifest.json"),
        ("query_sha256", "query.txt"),
        ("oracle_contract_sha256", "oracle-contract.json"),
    ] {
        let hash = sha256(&root.join(file))?;
        assert_eq!(startup["inputs"][field], hash);
        assert_eq!(oracle[field], hash);
    }
    for template in startup["templates"].as_array().ok_or("templates")? {
        assert_eq!(
            sha256(&root.join(template["source"].as_str().ok_or("template source")?))?,
            template["sha256"]
        );
    }
    let corpus: Value = serde_json::from_slice(&fs::read(root.join("corpus-manifest.json"))?)?;
    assert_eq!(corpus["fragments"].as_array().ok_or("fragments")?.len(), 10);
    for record in corpus["fragments"].as_array().ok_or("fragments")? {
        assert_eq!(
            sha256(&root.join(record["path"].as_str().ok_or("corpus path")?))?,
            record["sha256"]
        );
    }
    Ok(())
}

fn verify_model_cache(fixture: &Path, home: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let manifest: Value = serde_json::from_slice(&fs::read(fixture.join("model-manifest.json"))?)?;
    assert_eq!(
        sha256(&home.join("models/model-manifest.json"))?,
        sha256(&fixture.join("model-manifest.json"))?
    );
    for model in manifest["models"].as_array().ok_or("models")? {
        let slug = model["slug"].as_str().ok_or("slug")?;
        let repo = format!(
            "models--{}",
            model["repository"]
                .as_str()
                .ok_or("repo")?
                .replace('/', "--")
        );
        let revision = model["revision"].as_str().ok_or("revision")?;
        let snapshot = if slug == "qwen3-reranker-0.6b" {
            home.join("models").join(slug).join(revision)
        } else if slug == "nomic-embed-text-v2-moe" {
            home.join("models/huggingface/hub")
                .join(repo)
                .join("snapshots")
                .join(revision)
        } else {
            home.join("models")
                .join(slug)
                .join("runtime")
                .join(repo)
                .join("snapshots")
                .join(revision)
        };
        for asset in model["assets"].as_array().ok_or("assets")? {
            let path = snapshot.join(asset["path"].as_str().ok_or("asset path")?);
            assert_eq!(
                fs::metadata(&path)?.len(),
                asset["bytes"].as_u64().ok_or("asset bytes")?
            );
            assert_eq!(sha256(&path)?, asset["sha256"]);
        }
    }
    Ok(())
}

fn sha256(path: &Path) -> Result<String, std::io::Error> {
    let mut file = fs::File::open(path)?;
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
fn copy_tree(source: &Path, target: &Path) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if is_reparse(&metadata) {
            return Err(format!("reparse source rejected: {}", entry.path().display()).into());
        }
        let destination = target.join(entry.file_name());
        if metadata.is_dir() {
            copy_tree(&entry.path(), &destination)?;
        } else if metadata.is_file() {
            fs::copy(entry.path(), destination)?;
        } else {
            return Err("unsupported cache entry".into());
        }
    }
    Ok(())
}
#[cfg(windows)]
fn is_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}
#[cfg(not(windows))]
fn is_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}
fn fingerprint(root: &Path) -> Result<Vec<(String, u64, u128)>, std::io::Error> {
    fingerprint_excluding(root, &[])
}

fn fingerprint_excluding(
    root: &Path,
    excluded_roots: &[&str],
) -> Result<Vec<(String, u64, u128)>, std::io::Error> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut output = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap();
            if relative
                .components()
                .next()
                .and_then(|component| component.as_os_str().to_str())
                .is_some_and(|component| excluded_roots.contains(&component))
            {
                continue;
            }
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.is_dir() {
                pending.push(path)
            } else if metadata.is_file() {
                output.push((
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                    metadata.len(),
                    metadata
                        .modified()?
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos(),
                ))
            }
        }
    }
    output.sort();
    Ok(output)
}

fn product_tree_fingerprint(root: &Path) -> Result<Vec<(String, u64, u128)>, std::io::Error> {
    const OPERATIONAL_ROOTS: &[&str] =
        &[".agents", ".code-review-graph", ".dtree", ".git", "target"];
    fingerprint_excluding(root, OPERATIONAL_ROOTS)
}

async fn initialize<W: AsyncWrite + Unpin, R: tokio::io::AsyncRead + Unpin>(
    writer: &mut W,
    reader: &mut BufReader<R>,
) -> Result<(), Box<dyn std::error::Error>> {
    send_json(writer,&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"dt4-thin-slice","version":"1"}}})).await?;
    let value = read_id(reader, 1, INITIALIZE_TIMEOUT).await?;
    assert_eq!(value["result"]["protocolVersion"], "2025-11-25");
    send_json(
        writer,
        &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await?;
    Ok(())
}
async fn send_json<W: AsyncWrite + Unpin>(
    writer: &mut W,
    value: &Value,
) -> Result<(), Box<dyn std::error::Error>> {
    writer
        .write_all(serde_json::to_string(value)?.as_bytes())
        .await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}
async fn read_id<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    id: u64,
    timeout: Duration,
) -> Result<Value, Box<dyn std::error::Error>> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let mut line = String::new();
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let read = tokio::time::timeout(remaining, reader.read_line(&mut line)).await??;
        if read == 0 {
            return Err("server stdout closed".into());
        }
        let value: Value = serde_json::from_str(line.trim())?;
        if value["id"] == id {
            return Ok(value);
        }
    }
}
async fn collect_ids<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    window: Duration,
) -> Result<Vec<u64>, Box<dyn std::error::Error>> {
    let deadline = tokio::time::Instant::now() + window;
    let mut ids = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let mut line = String::new();
        match tokio::time::timeout(remaining, reader.read_line(&mut line)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(_)) => {
                let value: Value = serde_json::from_str(line.trim())?;
                if let Some(id) = value["id"].as_u64() {
                    ids.push(id)
                }
            }
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => break,
        }
    }
    Ok(ids)
}
async fn wait_for_child(child: &mut Child) -> Result<(), Box<dyn std::error::Error>> {
    let status = tokio::time::timeout(EOF_TIMEOUT, child.wait()).await??;
    assert!(status.success());
    Ok(())
}

#[test]
fn control_plane_windows_are_bounded_without_shortening_heavy_search() {
    assert_eq!(INITIALIZE_TIMEOUT, Duration::from_secs(10));
    assert_eq!(EOF_TIMEOUT, Duration::from_secs(10));
    assert_eq!(BUSY_TIMEOUT, Duration::from_secs(2));
    assert_eq!(SEARCH_WATCHDOG, Duration::from_secs(35));
    assert!(INITIALIZE_TIMEOUT < SEARCH_WATCHDOG);
    assert!(EOF_TIMEOUT < SEARCH_WATCHDOG);
}
fn assert_public_oracle(actual: &Value, oracle: &Value) -> Result<(), Box<dyn std::error::Error>> {
    let actual = actual["results"].as_array().ok_or("actual results")?;
    let expected = oracle["results"].as_array().ok_or("oracle results")?;
    assert_eq!(actual.len(), 6);
    for (actual, expected) in actual.iter().zip(expected) {
        assert_eq!(actual["rank"], expected["rank"]);
        assert_eq!(actual["title"], expected["title"]);
        assert_eq!(actual["project_scope"], expected["project_scope"]);
        assert_eq!(actual["document_status"], expected["document_status"]);
        assert!(
            actual["path"].as_str().ok_or("path")?.ends_with(
                expected["normalized_path"]
                    .as_str()
                    .ok_or("normalized path")?
            )
        );
        assert_eq!(
            format!(
                "{:x}",
                Sha256::digest(actual["content"].as_str().ok_or("content")?.as_bytes())
            ),
            expected["content_sha256"].as_str().ok_or("hash")?
        );
        for forbidden in ["stable_id", "model", "score", "provenance", "freshness"] {
            assert!(actual.get(forbidden).is_none())
        }
    }
    Ok(())
}

struct ChildWorkingSetMonitor {
    stop: Arc<AtomicBool>,
    peak: Arc<AtomicU64>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ChildWorkingSetMonitor {
    fn start(process_id: u32) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicU64::new(0));
        let worker_stop = Arc::clone(&stop);
        let worker_peak = Arc::clone(&peak);
        let worker = thread::spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                if let Some(value) = child_working_set_bytes(process_id) {
                    worker_peak.fetch_max(value, Ordering::AcqRel);
                }
                thread::sleep(Duration::from_millis(10));
            }
            if let Some(value) = child_working_set_bytes(process_id) {
                worker_peak.fetch_max(value, Ordering::AcqRel);
            }
        });
        Self {
            stop,
            peak,
            worker: Some(worker),
        }
    }

    fn finish(mut self) -> Result<u64, Box<dyn std::error::Error>> {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.join().map_err(|_| "working-set monitor panicked")?;
        }
        let peak = self.peak.load(Ordering::Acquire);
        if peak == 0 {
            return Err("child working set was not observable".into());
        }
        Ok(peak)
    }
}

impl Drop for ChildWorkingSetMonitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(windows)]
fn child_working_set_bytes(process_id: u32) -> Option<u64> {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::{
            ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
            Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ},
        },
    };
    let handle = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, process_id) };
    if handle.is_null() {
        return None;
    }
    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: u32::try_from(std::mem::size_of::<PROCESS_MEMORY_COUNTERS>()).ok()?,
        ..PROCESS_MEMORY_COUNTERS::default()
    };
    let ok = unsafe { GetProcessMemoryInfo(handle, (&raw mut counters).cast(), counters.cb) };
    unsafe { CloseHandle(handle) };
    (ok != 0).then_some(counters.WorkingSetSize as u64)
}

#[cfg(not(windows))]
fn child_working_set_bytes(_process_id: u32) -> Option<u64> {
    None
}

#[cfg(windows)]
fn free_physical_memory_bytes() -> Result<u64, Box<dyn std::error::Error>> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..unsafe { std::mem::zeroed() }
    };
    if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(status.ullAvailPhys)
}
#[cfg(not(windows))]
fn free_physical_memory_bytes() -> Result<u64, Box<dyn std::error::Error>> {
    Err("Windows TS-DT4-01 required".into())
}
