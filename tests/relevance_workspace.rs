//! Real-model acceptance; the fast suite separately checks model-free refusals.
use rmcp::{
    ClientHandler, ServiceExt,
    model::{CallToolRequestParams, ClientInfo, ProtocolVersion},
    transport::TokioChildProcess,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};

#[derive(Clone)]
struct Client;
impl ClientHandler for Client {
    fn get_info(&self) -> ClientInfo {
        let mut info = ClientInfo::default();
        info.protocol_version = ProtocolVersion::V_2025_11_25;
        info
    }
}

fn binary() -> PathBuf {
    std::env::var_os("FASTSEARCH_RELEVANCE_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_fastsearch")))
}

fn cli(root: &std::path::Path, query: &str, json_output: bool) -> std::process::Output {
    let mut command = Command::new(binary());
    command.args(["search", "--workspace"]).arg(root).arg(query);
    if json_output {
        command.arg("--json");
    }
    command.output().expect("CLI process")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires prepared production model set and FASTSEARCH_RELEVANCE_WORKSPACE with frozen relevance documents"]
async fn console_json_mcp_agree_and_reject_questions_without_answers()
-> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(
        std::env::var_os("FASTSEARCH_RELEVANCE_WORKSPACE").ok_or("missing workspace")?,
    );
    let query = "Что происходит с ручным решением, если его цель удалена?";
    let output = cli(&root, query, true);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let expected: Value = serde_json::from_slice(&output.stdout)?;
    let results = expected["results"].as_array().ok_or("results")?;
    assert!(!results.is_empty() && results.len() <= 5);
    assert!(
        results[0]["content"]
            .as_str()
            .unwrap()
            .contains("Удаление цели")
    );
    assert!(
        results
            .iter()
            .all(|result| result.as_object().unwrap().len() == 6)
    );

    let mut command = tokio::process::Command::new(binary());
    command
        .arg("mcp")
        .arg("--workspace")
        .arg(&root)
        .stderr(Stdio::piped());
    let partition = root.join(".fastsearch/local/index/vector/arctic-embed-l-v2");
    let unpublished = root.join(".fastsearch/local/index/arctic-pending");
    fs::rename(&partition, &unpublished)?;
    let restore = RestoreProjection {
        partition,
        unpublished,
    };
    let client = Client.serve(TokioChildProcess::new(command)?).await?;
    let unavailable = client
        .call_tool(
            CallToolRequestParams::new("search")
                .with_arguments(json!({"query":query}).as_object().unwrap().clone()),
        )
        .await?;
    // Publish the prepared projection while keeping the same MCP process alive.
    fs::rename(&restore.unpublished, &restore.partition)?;
    assert_eq!(unavailable.is_error, Some(true));
    assert_eq!(
        unavailable.structured_content.as_ref().unwrap()["error"]["code"],
        "NOT_READY"
    );
    for (question, expected_count) in [
        (query, None),
        (
            "Какая стоимость лицензии ЛИРА для ста рабочих мест?",
            Some(0),
        ),
    ] {
        let response = tokio::time::timeout(
            Duration::from_secs(35),
            client.call_tool(
                CallToolRequestParams::new("search")
                    .with_arguments(json!({"query":question}).as_object().unwrap().clone()),
            ),
        )
        .await??;
        assert_eq!(response.is_error, Some(false), "{response:?}");
        let actual = response
            .structured_content
            .ok_or("structured MCP response")?;
        let direct = cli(&root, question, true);
        assert!(
            direct.status.success(),
            "{}",
            String::from_utf8_lossy(&direct.stderr)
        );
        assert_eq!(actual, serde_json::from_slice::<Value>(&direct.stdout)?);
        if let Some(count) = expected_count {
            assert_eq!(actual["count"], count);
        } else {
            assert_eq!(actual, expected);
        }
    }
    client.cancel().await?;

    let absent = cli(
        &root,
        "Какая стоимость лицензии ЛИРА для ста рабочих мест?",
        false,
    );
    assert!(absent.status.success());
    assert!(String::from_utf8_lossy(&absent.stdout).contains("Убедительных совпадений не найдено"));
    let mut chat = Command::new(binary())
        .arg("chat")
        .current_dir(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    writeln!(
        chat.stdin.take().ok_or("chat stdin")?,
        "{query}\n/repeat\n/open 1\n/exit"
    )?;
    let chat = chat.wait_with_output()?;
    assert!(
        chat.status.success(),
        "{}",
        String::from_utf8_lossy(&chat.stderr)
    );
    let text = String::from_utf8(chat.stdout)?;
    assert!(text.matches("РЕЗУЛЬТАТЫ ПОИСКА").count() >= 2, "{text}");
    assert!(text.contains("СОДЕРЖИМОЕ"), "{text}");
    assert!(
        text.contains("перенос на похожую либо ближайшую сущность"),
        "{text}"
    );
    assert!(
        !text.contains("100%") && !text.contains("ТОЧН") && !text.contains("Страница 1"),
        "{text}"
    );
    fs::write(
        root.join("acceptance.json"),
        serde_json::to_vec_pretty(&json!({
            "status":"PASS", "positive_response":expected,
            "checks":["JSON/MCP equality", "no-answer rejection", "console repeat and full open", "no relative accuracy percentages"]
        }))?,
    )?;
    Ok(())
}

struct RestoreProjection {
    partition: PathBuf,
    unpublished: PathBuf,
}

impl Drop for RestoreProjection {
    fn drop(&mut self) {
        if self.unpublished.exists() {
            let _ = fs::rename(&self.unpublished, &self.partition);
        }
    }
}

#[test]
fn invalid_public_filters_are_rejected_before_workspace_creation() {
    let root = std::env::temp_dir().join(format!("fs-invalid-search-{}", std::process::id()));
    assert!(!root.exists());
    let output = Command::new(env!("CARGO_BIN_EXE_fastsearch"))
        .args(["search", "--workspace"])
        .arg(&root)
        .args(["question", "--project-scope", "bogus", "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], "INVALID_REQUEST");
    assert!(!root.exists());
}
