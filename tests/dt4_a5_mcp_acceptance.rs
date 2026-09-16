use std::{
    collections::BTreeSet,
    fs,
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant},
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

const READ_TIMEOUT: Duration = Duration::from_secs(10);
const SEARCH_WATCHDOG: Duration = Duration::from_secs(35);

#[derive(Clone)]
struct VersionedClient;

impl ClientHandler for VersionedClient {
    fn get_info(&self) -> ClientInfo {
        let mut info = ClientInfo::default();
        info.protocol_version = ProtocolVersion::V_2025_11_25;
        info
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires explicitly prepared TS-DT4-01 workspace and release binary"]
async fn release_mcp_response_matches_independent_oracle() -> Result<(), Box<dyn std::error::Error>>
{
    let client = VersionedClient.serve(transport()?).await?;
    assert_eq!(
        client
            .peer_info()
            .expect("initialized server")
            .protocol_version,
        ProtocolVersion::V_2025_11_25
    );
    let tools = client.list_all_tools().await?;
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "search");
    assert!(tools[0].output_schema.is_some());
    assert_eq!(
        tools[0]
            .annotations
            .as_ref()
            .and_then(|value| value.read_only_hint),
        Some(true)
    );
    let query = fs::read_to_string(required_path("FASTSEARCH_A5_QUERY")?)?;
    let response = tokio::time::timeout(
        SEARCH_WATCHDOG,
        client.call_tool(
            CallToolRequestParams::new("search")
                .with_arguments(JsonObject::from_iter([("query".to_owned(), json!(query))])),
        ),
    )
    .await??;
    assert_eq!(response.is_error, Some(false));
    let actual = response.structured_content.expect("structured response");
    assert_oracle(&actual)?;
    client.cancel().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires an explicitly unprepared workspace and release binary"]
async fn release_mcp_reports_not_ready_without_building_or_downloading()
-> Result<(), Box<dyn std::error::Error>> {
    let workspace = required_path("FASTSEARCH_A5_UNREADY_WORKSPACE")?;
    let local = workspace.join(".fastsearch").join("local");
    let before = directory_fingerprint(&local)?;
    let client = VersionedClient.serve(transport_for(&workspace)?).await?;
    let response = client
        .call_tool(
            CallToolRequestParams::new("search").with_arguments(JsonObject::from_iter([(
                "query".to_owned(),
                json!("automatic local model preparation"),
            )])),
        )
        .await?;
    assert_eq!(response.is_error, Some(true));
    assert_eq!(
        response.structured_content.expect("structured error")["error"]["code"],
        "NOT_READY"
    );
    client.cancel().await?;
    assert_eq!(before, directory_fingerprint(&local)?);
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicitly prepared TS-DT4-01 workspace and release binary"]
async fn third_search_is_busy_and_cancelled_responses_are_suppressed()
-> Result<(), Box<dyn std::error::Error>> {
    let query = fs::read_to_string(required_path("FASTSEARCH_A5_QUERY")?)?;
    let mut child = spawn_raw();
    let mut writer = child.stdin.take().expect("server stdin");
    let stdout = child.stdout.take().expect("server stdout");
    let mut reader = BufReader::new(stdout);
    initialize_raw(&mut writer, &mut reader).await?;
    for id in 2..=4 {
        send_json(
            &mut writer,
            &json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"search","arguments":{"query":query}}}),
        )
        .await?;
    }
    let started = Instant::now();
    let busy = read_id(&mut reader, 4, Duration::from_secs(2)).await?;
    assert!(started.elapsed() <= Duration::from_secs(2));
    assert_eq!(busy["result"]["isError"], true);
    assert_eq!(busy["result"]["structuredContent"]["error"]["code"], "BUSY");
    for id in [2, 3] {
        send_json(
            &mut writer,
            &json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":id}}),
        )
        .await?;
    }
    send_json(
        &mut writer,
        &json!({"jsonrpc":"2.0","id":5,"method":"ping"}),
    )
    .await?;
    let ids = collect_ids_for_window(&mut reader, SEARCH_WATCHDOG).await?;
    assert!(ids.contains(&5));
    assert!(!ids.contains(&2));
    assert!(!ids.contains(&3));
    drop(writer);
    wait_for_child(&mut child, READ_TIMEOUT).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires release binary"]
async fn eof_before_initialize_is_clean_and_graceful() -> Result<(), Box<dyn std::error::Error>> {
    let mut child = spawn_raw();
    drop(child.stdin.take());
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .expect("server stdout")
        .read_to_string(&mut stdout)
        .await?;
    let status = tokio::time::timeout(READ_TIMEOUT, child.wait()).await??;
    assert!(status.success());
    assert!(stdout.is_empty());
    Ok(())
}

fn transport() -> std::io::Result<TokioChildProcess> {
    TokioChildProcess::new(command().configure(|command| {
        command.stderr(Stdio::piped());
    }))
}

fn transport_for(workspace: &std::path::Path) -> std::io::Result<TokioChildProcess> {
    let mut command = Command::new(required_path("FASTSEARCH_A5_RELEASE_BINARY").expect("binary"));
    command.arg("mcp").arg("--workspace").arg(workspace);
    TokioChildProcess::new(command.configure(|command| {
        command.stderr(Stdio::piped());
    }))
}

fn command() -> Command {
    let mut command = Command::new(required_path("FASTSEARCH_A5_RELEASE_BINARY").expect("binary"));
    command
        .arg("mcp")
        .arg("--workspace")
        .arg(required_path("FASTSEARCH_A5_WORKSPACE").expect("workspace"));
    command
}

fn spawn_raw() -> Child {
    command()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn release server")
}

fn required_path(name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(PathBuf::from(
        std::env::var_os(name).ok_or_else(|| format!("{name} is required"))?,
    ))
}

fn directory_fingerprint(
    path: &std::path::Path,
) -> Result<Vec<(String, u64)>, Box<dyn std::error::Error>> {
    let mut entries = Vec::new();
    if !path.exists() {
        return Ok(entries);
    }
    let mut pending = vec![path.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                entries.push((
                    entry
                        .path()
                        .strip_prefix(path)?
                        .to_string_lossy()
                        .replace('\\', "/"),
                    metadata.len(),
                ));
            }
        }
    }
    entries.sort();
    Ok(entries)
}

fn assert_oracle(actual: &Value) -> Result<(), Box<dyn std::error::Error>> {
    let oracle: Value = serde_json::from_slice(&fs::read(required_path("FASTSEARCH_A5_ORACLE")?)?)?;
    let actual_results = actual["results"]
        .as_array()
        .ok_or("actual results missing")?;
    let expected = oracle["results"]
        .as_array()
        .ok_or("oracle results missing")?;
    let cutoff: Value =
        serde_json::from_str(include_str!("../tests/fixtures/relevance/evaluation.json")).unwrap();
    let expected = expected
        .iter()
        .filter(|row| {
            oracle["qwen_probabilities"][row["stable_id"].as_str().unwrap()]
                .as_f64()
                .unwrap()
                >= cutoff["threshold"].as_f64().unwrap()
        })
        .take(5)
        .collect::<Vec<_>>();
    assert_eq!(actual["count"], expected.len());
    assert_eq!(actual_results.len(), expected.len());
    for (actual, expected) in actual_results.iter().zip(expected) {
        assert_eq!(actual["rank"], expected["rank"]);
        assert_eq!(actual["title"], expected["title"]);
        assert_eq!(actual["project_scope"], expected["project_scope"]);
        assert_eq!(actual["document_status"], expected["document_status"]);
        assert!(
            actual["path"]
                .as_str()
                .expect("path")
                .ends_with(expected["normalized_path"].as_str().expect("oracle path"))
        );
        let digest = format!(
            "{:x}",
            Sha256::digest(actual["content"].as_str().expect("content").as_bytes())
        );
        assert_eq!(
            digest,
            expected["content_sha256"].as_str().expect("content digest")
        );
        for forbidden in ["model", "score", "stable_id", "provenance", "freshness"] {
            assert!(actual.get(forbidden).is_none(), "leaked {forbidden}");
        }
    }
    Ok(())
}

async fn initialize_raw<W, R>(
    writer: &mut W,
    reader: &mut BufReader<R>,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: AsyncWrite + Unpin,
    R: tokio::io::AsyncRead + Unpin,
{
    send_json(writer, &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"dt4-a5","version":"1"}}})).await?;
    let value = read_id(reader, 1, READ_TIMEOUT).await?;
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
        if value.get("id").and_then(Value::as_u64) == Some(id) {
            return Ok(value);
        }
    }
}

async fn collect_ids_for_window<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    timeout: Duration,
) -> Result<BTreeSet<u64>, Box<dyn std::error::Error>> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut ids = BTreeSet::new();
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let mut line = String::new();
        let Ok(read) = tokio::time::timeout(remaining, reader.read_line(&mut line)).await else {
            break;
        };
        if read? == 0 {
            break;
        }
        let value: Value = serde_json::from_str(line.trim())?;
        if let Some(id) = value.get("id").and_then(Value::as_u64) {
            ids.insert(id);
        }
    }
    Ok(ids)
}

async fn wait_for_child(
    child: &mut Child,
    timeout: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let status = tokio::time::timeout(timeout, child.wait()).await??;
    assert!(status.success());
    Ok(())
}
