use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::Value;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let nonce = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time is after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("fastsearch-cli-ux-{nanos}-{nonce}"));
        fs::create_dir_all(&path).expect("temporary directory");
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

struct Fixture {
    _temp: Temp,
    documents: PathBuf,
    code: PathBuf,
    service: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = Temp::new();
        let documents = temp.child("document sources");
        let code = temp.child("code sources");
        let service = temp.child("service state");
        fs::create_dir_all(&documents).expect("documents directory");
        fs::create_dir_all(&code).expect("code directory");
        fs::write(
            documents.join("guide.md"),
            "# FastSearch CLI\n\nУдобный интерактивный поиск.",
        )
        .expect("document fixture");
        fs::write(code.join("lib.rs"), "pub fn cli_fixture() {}\n").expect("code fixture");
        Self {
            _temp: temp,
            documents,
            code,
            service,
        }
    }

    fn arguments(&self, command: &[&str]) -> Vec<String> {
        let mut result = command.iter().map(ToString::to_string).collect::<Vec<_>>();
        result.extend([
            path_text(&self.documents),
            path_text(&self.code),
            path_text(&self.service),
        ]);
        result
    }
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_fastsearch"))
}

fn run(arguments: &[String]) -> Output {
    binary()
        .args(arguments)
        .output()
        .expect("FastSearch CLI starts")
}

fn run_with_input(arguments: &[String], input: &str, no_color: bool) -> Output {
    let mut command = binary();
    command
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if no_color {
        command.env("NO_COLOR", "1");
    }
    let mut child = command.spawn().expect("FastSearch CLI starts");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(input.as_bytes())
        .expect("interactive input");
    child.wait_with_output().expect("FastSearch CLI finishes")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr is UTF-8")
}

fn assert_no_ansi(text: &str) {
    assert!(
        !text.contains("\u{1b}["),
        "unexpected ANSI sequence: {text:?}"
    );
}

#[test]
fn help_aliases_and_version_are_successful_stdout_commands() {
    for alias in ["help", "--help", "-h"] {
        let output = run(&[alias.to_owned()]);
        assert_eq!(output.status.code(), Some(0), "alias={alias}");
        let text = stdout(&output);
        assert!(text.contains("FastSearch"), "alias={alias}: {text}");
        assert!(text.contains("search"), "alias={alias}: {text}");
        assert!(stderr(&output).is_empty(), "alias={alias}");
        assert_no_ansi(&text);
    }

    for alias in ["version", "--version", "-V"] {
        let output = run(&[alias.to_owned()]);
        assert_eq!(output.status.code(), Some(0), "alias={alias}");
        let text = stdout(&output);
        assert!(
            text.to_ascii_lowercase().contains("fastsearch"),
            "alias={alias}: {text}"
        );
        assert!(
            text.contains(env!("CARGO_PKG_VERSION")),
            "alias={alias}: {text}"
        );
        assert!(stderr(&output).is_empty(), "alias={alias}");
        assert_no_ansi(&text);
    }
}

#[test]
fn direct_without_format_flag_keeps_the_legacy_machine_contract() {
    let fixture = Fixture::new();
    let output = run(&fixture.arguments(&["status"]));
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("freshness="), "{text}");
    assert!(text.contains("state-generation="), "{text}");
    assert!(text.contains("projection-generation="), "{text}");
    assert!(!text.trim_start().starts_with('{'), "{text}");
    assert!(stderr(&output).is_empty());
    assert_no_ansi(&text);
}

#[test]
fn leading_json_option_returns_versioned_success_and_error_documents() {
    let fixture = Fixture::new();
    let mut arguments = vec!["--json".to_owned()];
    arguments.extend(fixture.arguments(&["status"]));
    let success = run(&arguments);
    assert!(success.status.success(), "{}", stderr(&success));
    assert!(stderr(&success).is_empty());
    let success_text = stdout(&success);
    assert_no_ansi(&success_text);
    let success_json: Value = serde_json::from_str(&success_text).expect("success JSON");
    assert_eq!(success_json["schema_version"], 1);
    assert_eq!(success_json["status"], "ok");
    assert_eq!(success_json["kind"], "index_status");
    assert!(success_json["capabilities"].is_array());

    let failure = run(&["--json".to_owned(), "unknown-command".to_owned()]);
    assert_eq!(failure.status.code(), Some(2));
    assert!(stdout(&failure).is_empty());
    let failure_text = stderr(&failure);
    assert_no_ansi(&failure_text);
    let failure_json: Value = serde_json::from_str(&failure_text).expect("error JSON");
    assert_eq!(failure_json["schema_version"], 1);
    assert_eq!(failure_json["status"], "error");
    assert_eq!(failure_json["error"]["code"], "usage");
    assert!(
        failure_json["error"]["message"]
            .as_str()
            .is_some_and(|message| !message.trim().is_empty())
    );
}

#[test]
fn json_is_position_independent_and_duplicate_options_are_idempotent() {
    let fixture = Fixture::new();
    let leading = run(&[
        "--json".to_owned(),
        "status".to_owned(),
        path_text(&fixture.documents),
        path_text(&fixture.code),
        path_text(&fixture.service),
    ]);
    let middle = run(&[
        "status".to_owned(),
        path_text(&fixture.documents),
        "--json".to_owned(),
        path_text(&fixture.code),
        path_text(&fixture.service),
    ]);
    let trailing = run(&[
        "status".to_owned(),
        path_text(&fixture.documents),
        path_text(&fixture.code),
        path_text(&fixture.service),
        "--json".to_owned(),
    ]);
    let duplicate = run(&[
        "--json".to_owned(),
        "status".to_owned(),
        path_text(&fixture.documents),
        "--json".to_owned(),
        path_text(&fixture.code),
        path_text(&fixture.service),
        "--json".to_owned(),
    ]);

    let outputs = [&leading, &middle, &trailing, &duplicate];
    for output in outputs {
        assert!(output.status.success(), "{}", stderr(output));
        assert!(stderr(output).is_empty());
        assert_no_ansi(&stdout(output));
    }

    let documents =
        outputs.map(|output| serde_json::from_str::<Value>(&stdout(output)).expect("status JSON"));
    assert_eq!(documents[0], documents[1]);
    assert_eq!(documents[0], documents[2]);
    assert_eq!(documents[0], documents[3]);
}

#[test]
fn option_terminator_preserves_a_literal_json_argument() {
    let fixture = Fixture::new();
    let output = run(&[
        "search".to_owned(),
        path_text(&fixture.documents),
        path_text(&fixture.code),
        path_text(&fixture.service),
        "balanced".to_owned(),
        "--".to_owned(),
        "--json".to_owned(),
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("hits="), "{text}");
    assert!(!text.trim_start().starts_with('{'), "{text}");
}

#[test]
fn no_arguments_show_onboarding_and_eof_is_a_clean_exit() {
    let output = run(&[]);
    assert_eq!(output.status.code(), Some(0));
    assert!(stderr(&output).is_empty());
    let text = stdout(&output);
    assert!(text.contains("FastSearch"), "{text}");
    assert!(
        text.contains("документ") || text.contains("Документ"),
        "{text}"
    );
    assert!(text.contains("Пример") || text.contains("пример"), "{text}");
    assert!(text.contains("Enter"), "{text}");
    assert_no_ansi(&text);
}

#[test]
fn exit_alias_closes_the_onboarding_without_creating_a_context() {
    for alias in ["exit", "quit", "выход"] {
        let output = run_with_input(&[], &format!("{alias}\n"), true);
        assert!(
            output.status.success(),
            "alias={alias}: {}",
            stderr(&output)
        );
        let text = stdout(&output);
        assert!(text.contains("закрыт"), "alias={alias}: {text}");
        assert!(!text.contains("Контекст готов"), "alias={alias}: {text}");
    }
}

#[test]
fn chat_keeps_context_runs_multiple_commands_recovers_and_toggles_json() {
    let fixture = Fixture::new();
    let input = format!(
        "{}\n{}\n{}\n\ncontext\nstatus\nunknown-command\nstatus\njson\nstatus\njson\nhelp\nexit\n",
        path_text(&fixture.documents),
        path_text(&fixture.code),
        path_text(&fixture.service),
    );
    let output = run_with_input(&[], &input, true);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(stderr(&output).is_empty());
    let text = stdout(&output);
    assert_no_ansi(&text);
    assert!(text.contains(&path_text(&fixture.documents)), "{text}");
    assert!(text.contains(&path_text(&fixture.code)), "{text}");
    assert!(text.contains(&path_text(&fixture.service)), "{text}");
    assert!(text.contains("Состояние индекса"), "{text}");
    assert!(text.contains("Ошибка") || text.contains("ошибка"), "{text}");
    assert!(
        text.matches("Состояние индекса").count() >= 2,
        "chat did not continue after an error: {text}"
    );
    assert!(text.contains("\"schema_version\""), "{text}");
    assert!(text.contains("\"status\": \"ok\""), "{text}");
    assert!(
        text.contains("Команды") || text.contains("команды"),
        "{text}"
    );
}

#[test]
fn redirected_output_and_no_color_never_emit_ansi() {
    let help = run_with_input(&["--help".to_owned()], "", true);
    assert!(help.status.success());
    assert_no_ansi(&stdout(&help));
    assert_no_ansi(&stderr(&help));

    let onboarding = run_with_input(&[], "", true);
    assert!(onboarding.status.success());
    assert_no_ansi(&stdout(&onboarding));
    assert_no_ansi(&stderr(&onboarding));
}
