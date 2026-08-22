use std::{collections::BTreeSet, process::Stdio, time::Duration};

use rmcp::{
    ClientHandler, ServiceExt,
    model::{CallToolRequestParams, ClientInfo, JsonObject, ProtocolVersion},
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    process::{Child, Command},
};

const HELPER_ENV: &str = "FASTSEARCH_DT4_MCP_PREFLIGHT_HELPER";
const READ_TIMEOUT: Duration = Duration::from_secs(10);
#[derive(Clone)]
struct VersionedClient(ProtocolVersion);

impl ClientHandler for VersionedClient {
    fn get_info(&self) -> ClientInfo {
        let mut info = ClientInfo::default();
        info.protocol_version = self.0.clone();
        info
    }
}

#[tokio::test]
async fn rmcp_client_crosses_real_stdio_without_working_search()
-> Result<(), Box<dyn std::error::Error>> {
    let transport = helper_transport("normal")?;
    let client = VersionedClient(ProtocolVersion::V_2025_11_25)
        .serve(transport)
        .await?;
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
    let response = client
        .call_tool(
            CallToolRequestParams::new("search").with_arguments(JsonObject::from_iter([(
                "query".to_owned(),
                json!("protocol admission only"),
            )])),
        )
        .await?;
    assert_eq!(
        response
            .structured_content
            .as_ref()
            .and_then(|value| value.get("working_search_invoked")),
        Some(&json!(false))
    );
    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn incompatible_client_stops_after_negotiation() -> Result<(), Box<dyn std::error::Error>> {
    let transport = helper_transport("normal")?;
    let client = VersionedClient(ProtocolVersion::V_2026_07_28)
        .serve(transport)
        .await?;
    assert_eq!(
        client
            .peer_info()
            .expect("initialized server")
            .protocol_version,
        ProtocolVersion::V_2025_11_25
    );
    client.cancel().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_suppresses_late_response() -> Result<(), Box<dyn std::error::Error>> {
    let mut child = spawn_raw_helper("cancel");
    let mut writer = child.stdin.take().expect("helper stdin");
    let stdout = child.stdout.take().expect("helper stdout");
    let mut reader = BufReader::new(stdout);
    initialize_raw(&mut writer, &mut reader).await?;
    send_json(
        &mut writer,
        &json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search","arguments":{"query":"cancel"}}}),
    )
    .await?;
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
    let ids = collect_ids_until(&mut reader, 3).await?;
    assert!(ids.contains(&3));
    assert!(!ids.contains(&2));
    drop(writer);
    wait_for_child(&mut child).await;
    Ok(())
}

#[tokio::test]
async fn eof_stops_stdio_server_without_output_noise() -> Result<(), Box<dyn std::error::Error>> {
    let mut child = spawn_raw_helper("normal");
    drop(child.stdin.take());
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .expect("helper stdout")
        .read_to_string(&mut stdout)
        .await?;
    let status = tokio::time::timeout(READ_TIMEOUT, child.wait()).await??;
    assert!(status.success());
    assert!(stdout.is_empty());
    Ok(())
}

fn helper_command(mode: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dt4_mcp_preflight_server"));
    command.env(HELPER_ENV, mode);
    command
}

fn helper_transport(mode: &str) -> std::io::Result<TokioChildProcess> {
    TokioChildProcess::new(helper_command(mode).configure(|command| {
        command.stderr(Stdio::piped());
    }))
}

fn spawn_raw_helper(mode: &str) -> Child {
    helper_command(mode)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn helper")
}

async fn initialize_raw<W, R>(
    writer: &mut W,
    reader: &mut BufReader<R>,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: AsyncWrite + Unpin,
    R: tokio::io::AsyncRead + Unpin,
{
    send_json(
        writer,
        &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"dt4-preflight","version":"1"}}}),
    )
    .await?;
    assert!(collect_ids_until(reader, 1).await?.contains(&1));
    send_json(
        writer,
        &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await?;
    Ok(())
}

async fn send_json<W: AsyncWrite + Unpin>(
    writer: &mut W,
    message: &Value,
) -> Result<(), Box<dyn std::error::Error>> {
    writer
        .write_all(serde_json::to_string(message)?.as_bytes())
        .await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}

async fn collect_ids_until<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    stop_id: u64,
) -> Result<BTreeSet<u64>, Box<dyn std::error::Error>> {
    let mut ids = BTreeSet::new();
    let mut deadline = tokio::time::Instant::now() + READ_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let mut line = String::new();
        let Ok(result) = tokio::time::timeout(remaining, reader.read_line(&mut line)).await else {
            break;
        };
        if result? == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(trimmed)?;
        if let Some(id) = value.get("id").and_then(Value::as_u64) {
            ids.insert(id);
            if id == stop_id {
                deadline = tokio::time::Instant::now() + Duration::from_millis(300);
            }
        }
    }
    Ok(ids)
}

async fn wait_for_child(child: &mut Child) {
    let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
    if child.id().is_some() {
        let _ = child.kill().await;
    }
}
