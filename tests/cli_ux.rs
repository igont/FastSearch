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
    temp: Temp,
    documents: PathBuf,
    code: PathBuf,
    service: PathBuf,
    catalog_home: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = Temp::new();
        let documents = temp.child("document sources");
        let code = temp.child("code sources");
        let service = temp.child("service state");
        let catalog_home = temp.child("catalog home");
        fs::create_dir_all(&documents).expect("documents directory");
        fs::create_dir_all(&code).expect("code directory");
        fs::write(
            documents.join("guide.md"),
            "# FastSearch CLI\n\nУдобный интерактивный поиск.",
        )
        .expect("document fixture");
        fs::write(code.join("lib.rs"), "pub fn cli_fixture() {}\n").expect("code fixture");
        Self {
            temp,
            documents,
            code,
            service,
            catalog_home,
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

    fn root(&self) -> &Path {
        &self.temp.0
    }
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn binary() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fastsearch"));
    command.env("FASTSEARCH_TEST_DISABLE_MODEL_AUTO_DOWNLOAD", "1");
    command
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

fn run_workspace_input(fixture: &Fixture, input: &str, no_color: bool) -> Output {
    run_workspace_input_from(fixture, fixture.root(), input, no_color)
}

fn run_workspace_input_from(
    fixture: &Fixture,
    current_dir: &Path,
    input: &str,
    no_color: bool,
) -> Output {
    let mut command = binary();
    command
        .current_dir(current_dir)
        .env("FASTSEARCH_HOME", &fixture.catalog_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if no_color {
        command.env("NO_COLOR", "1");
    }
    let mut child = command
        .spawn()
        .expect("FastSearch workspace console starts");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(input.as_bytes())
        .expect("interactive input");
    child
        .wait_with_output()
        .expect("FastSearch workspace console finishes")
}

fn create_workspace_then(commands: &str) -> String {
    format!("n\n\n\n{commands}")
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
    let fixture = Fixture::new();
    let output = run_workspace_input(&fixture, "", true);
    assert_eq!(output.status.code(), Some(0));
    assert!(stderr(&output).is_empty());
    let text = stdout(&output);
    assert!(text.contains("FastSearch"), "{text}");
    assert!(text.contains("РАБОЧИЕ ОБЛАСТИ"), "{text}");
    assert!(
        text.contains("Выберите действие:\n  N — создать рабочую область\n  Q — выйти"),
        "{text}"
    );
    assert!(!text.contains("Папка документов"), "{text}");
    assert!(!text.contains("service:"), "{text}");
    assert_no_ansi(&text);
}

#[test]
fn workspace_picker_marks_the_remove_target_as_a_number_placeholder() {
    let fixture = Fixture::new();
    let created = run_workspace_input(&fixture, &create_workspace_then("/exit\n"), true);
    assert!(created.status.success(), "{}", stderr(&created));

    let output =
        run_workspace_input_from(&fixture, std::env::temp_dir().as_path(), "/exit\n", true);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("<номер>"), "{text}");
    assert!(text.contains("открыть область из списка"), "{text}");
    assert!(
        text.contains("R <номер> — удалить область из списка"),
        "{text}"
    );
    assert!(!text.contains("R N — удалить область из списка"), "{text}");
}

#[test]
fn interactive_visual_contract_matches_dtree_typography_and_separator() {
    let fixture = Fixture::new();
    let output = run_workspace_input(&fixture, "/exit\n", true);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    let mut lines = text.lines();

    assert_eq!(lines.next(), Some("FASTSEARCH"));
    assert_eq!(
        lines.next(),
        Some("  Локальный поиск по документации и исходному коду.")
    );
    assert!(text.contains("РАБОЧИЕ ОБЛАСТИ"), "{text}");
    assert!(!text.contains("FastSearch — поиск"), "{text}");
    assert!(!text.contains('─'), "{text}");

    let separator = text
        .lines()
        .find(|line| line.starts_with("====") && line.contains('.'))
        .expect("timestamp separator");
    assert_eq!(separator.chars().count(), 110, "{separator}");
    let timestamp = separator
        .rsplit_once(' ')
        .and_then(|(date_and_fill, time)| {
            date_and_fill
                .rsplit_once(' ')
                .map(|(_, date)| format!("{date} {time}"))
        })
        .expect("date and time suffix");
    assert_eq!(timestamp.len(), 16, "{timestamp}");
    assert_eq!(timestamp.as_bytes()[2], b'.');
    assert_eq!(timestamp.as_bytes()[5], b'.');
    assert_eq!(timestamp.as_bytes()[10], b' ');
    assert_eq!(timestamp.as_bytes()[13], b':');
}

#[test]
fn exit_alias_closes_the_onboarding_without_creating_a_context() {
    for alias in ["exit", "quit", "выход", "/exit"] {
        let fixture = Fixture::new();
        let output = run_workspace_input(&fixture, &format!("{alias}\n"), true);
        assert!(
            output.status.success(),
            "alias={alias}: {}",
            stderr(&output)
        );
        let text = stdout(&output);
        assert!(text.contains("закрыт"), "alias={alias}: {text}");
        assert!(!fixture.root().join(".fastsearch").exists());
    }
}

#[test]
fn workspace_console_keeps_context_and_recovers_after_an_unknown_command() {
    let fixture = Fixture::new();
    let input = create_workspace_then("/status\n/unknown-command\n/status\n/help\n/exit\n");
    let output = run_workspace_input(&fixture, &input, true);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(stderr(&output).is_empty());
    let text = stdout(&output);
    assert_no_ansi(&text);
    assert!(text.contains("Рабочая область создана"), "{text}");
    assert!(text.to_lowercase().contains("документац"), "{text}");
    assert!(text.to_lowercase().contains("код"), "{text}");
    assert!(text.matches("FASTSEARCH").count() >= 3, "{text}");
    assert!(text.to_lowercase().contains("ошибка"), "{text}");
    assert!(
        text.matches("FASTSEARCH").count() >= 3,
        "chat did not continue after an error: {text}"
    );
    assert!(
        text.contains("Команды") || text.contains("команды"),
        "{text}"
    );
}

#[test]
fn workspace_creation_uses_default_model_and_model_command_remains_selectable() {
    let fixture = Fixture::new();
    let input = "n\n\n\n/model qwen\n/model\n/model info e5-small\n/status\n/exit\n";
    let output = run_workspace_input(&fixture, input, true);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("МОДЕЛЬ ПОИСКА"), "{text}");
    assert!(text.contains("Qwen3 Embedding 0.6B"), "{text}");
    assert!(text.contains("МОДЕЛЬ: Qwen3 Embedding 0.6B"), "{text}");
    assert!(text.contains("МОДЕЛЬ"), "{text}");
    assert!(text.contains("СОСТОЯНИЕ"), "{text}");
    assert!(text.contains("CPU"), "{text}");
    assert!(text.contains("GPU"), "{text}");
    assert!(text.contains("ЗАГРУЗКА"), "{text}");
    assert!(text.contains("ИНДЕКС"), "{text}");
    assert!(text.contains("?"), "{text}");
    assert!(text.contains("intfloat/multilingual-e5-small"), "{text}");
    let profile = fs::read_to_string(fixture.root().join(".fastsearch/workspace.toml")).unwrap();
    assert!(profile.contains("qwen3-embedding-0.6b"), "{profile}");
    assert!(
        !fixture
            .root()
            .join(".fastsearch/local/index/cross/vector")
            .exists()
    );
}

#[test]
fn model_catalog_accepts_plain_number_as_the_next_selection() {
    let fixture = Fixture::new();
    let input = "n\n\n\n1\n/model\n2\n/status\n/exit\n";
    let output = run_workspace_input(&fixture, input, true);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(!text.contains("UNKNOWN_COMMAND"), "{text}");
    assert!(text.contains("МОДЕЛЬ: Arctic Embed L v2"), "{text}");
    assert!(text.contains("Модель: Arctic Embed L v2"), "{text}");
    let profile = fs::read_to_string(fixture.root().join(".fastsearch/workspace.toml")).unwrap();
    assert!(profile.contains("arctic-embed-l-v2"), "{profile}");
}

#[test]
fn model_command_accepts_a_number_without_opening_the_catalog_first() {
    let fixture = Fixture::new();
    let input = "n\n\n\n1\n/model 2\n/status\n/exit\n";
    let output = run_workspace_input(&fixture, input, true);

    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(!text.contains("UNKNOWN_COMMAND"), "{text}");
    assert!(text.contains("МОДЕЛЬ: Arctic Embed L v2"), "{text}");
    assert_eq!(
        text.matches("МОДЕЛЬ ПОИСКА").count(),
        2,
        "workspace open and one model selection must render one catalog each: {text}"
    );
    let profile = fs::read_to_string(fixture.root().join(".fastsearch/workspace.toml")).unwrap();
    assert!(profile.contains("arctic-embed-l-v2"), "{profile}");
}

#[test]
fn index_status_and_inspection_offer_actions_and_hide_internal_windows_prefixes() {
    let fixture = Fixture::new();
    let input = create_workspace_then("/index update\n/index\n/index inspect\n/exit\n");
    let output = run_workspace_input(&fixture, &input, true);

    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    let after_status = text
        .split_once("СОСТОЯНИЕ ИНДЕКСА")
        .map(|(_, tail)| tail)
        .expect("index status is rendered");
    for action in ["/index update", "/index rebuild", "/index inspect"] {
        assert!(after_status.contains(action), "missing {action}: {text}");
    }
    let after_export = text
        .rsplit_once("Выгрузка создана:")
        .map(|(_, tail)| tail)
        .expect("inspection report is rendered");
    for action in ["/index inspect", "/index", "/index update"] {
        assert!(after_export.contains(action), "missing {action}: {text}");
    }
    assert!(!after_export.contains(r"\\?\"), "{text}");
}

#[test]
fn index_rebuild_starts_without_a_confirmation_preview() {
    let fixture = Fixture::new();
    let input = create_workspace_then("/index rebuild\n/exit\n");
    let output = run_workspace_input(&fixture, &input, true);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("ПЕРЕСТРОЕНИЕ ИНДЕКСА — ГОТОВО"), "{text}");
    assert!(!text.contains("Подтверждение"), "{text}");
    assert!(!text.contains("Введите да — выполнить действие"), "{text}");
    assert!(fixture.root().join(".fastsearch/workspace.toml").is_file());
}

#[test]
fn index_rebuild_shows_progress_for_the_active_model() {
    let fixture = Fixture::new();
    let input = create_workspace_then("/index rebuild\n/exit\n");
    let output = run_workspace_input(&fixture, &input, true);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("ПЕРЕСТРОЕНИЕ ИНДЕКСА — ГОТОВО"), "{text}");
    assert!(text.contains("Готово: 1/1"), "{text}");
    assert!(
        text.contains("Индекс рабочей области · E5 Small · CPU"),
        "{text}"
    );
    assert!(!text.contains("этапов"), "{text}");
}

#[test]
fn index_clear_removes_all_or_one_model_partition() {
    let fixture = Fixture::new();
    let created = run_workspace_input(&fixture, "n\n\n\n\n/exit\n", true);
    assert!(created.status.success(), "{}", stderr(&created));

    let vector_root = fixture.root().join(".fastsearch/local/index/vector");
    let selected = vector_root.join("multilingual-e5-small/revision-a");
    let retained = vector_root.join("multilingual-e5-base/revision-b");
    fs::create_dir_all(&selected).unwrap();
    fs::create_dir_all(&retained).unwrap();
    fs::write(selected.join("vectors.bin"), "selected").unwrap();
    fs::write(retained.join("vectors.bin"), "retained").unwrap();

    let one = run_workspace_input(&fixture, "1\n/index clear e5-small\n/exit\n", true);
    assert!(one.status.success(), "{}", stderr(&one));
    assert!(!selected.exists());
    assert!(retained.exists());
    assert!(stdout(&one).contains("Индекс модели E5 Small очищен."));

    let all = run_workspace_input(&fixture, "1\n/index clear\n/exit\n", true);
    assert!(all.status.success(), "{}", stderr(&all));
    assert!(!vector_root.exists());
    assert!(stdout(&all).contains("Индексы всех моделей очищены."));
}

#[test]
fn model_device_assignment_is_applied_and_survives_a_restart() {
    let fixture = Fixture::new();
    let capability_root = fixture
        .catalog_home
        .join("models")
        .join("multilingual-e5-small");
    fs::create_dir_all(&capability_root).unwrap();
    fs::write(
        capability_root.join("runtime-capabilities.toml"),
        "schema = 1\nmodel_revision = \"614241f622f53c4eeff9890bdc4f31cfecc418b3\"\ncpu = \"ready\"\ngpu = \"ready\"\ngpu_backend = \"DirectML\"\ngpu_detail = \"test probe\"\n",
    )
    .unwrap();

    let first = run_workspace_input(
        &fixture,
        "n\n\n\n\n/model device e5-small gpu\n/index rebuild\n/model\n/exit\n",
        true,
    );
    assert!(first.status.success(), "{}", stderr(&first));
    let first_text = stdout(&first);
    assert!(
        first_text.contains("назначено GPU · DirectML"),
        "{first_text}"
    );
    assert!(
        first_text.contains("Индекс рабочей области · E5 Small · GPU · DirectML"),
        "{first_text}"
    );
    let preferences =
        fs::read_to_string(fixture.catalog_home.join("device-preferences.toml")).unwrap();
    assert!(
        preferences.contains("multilingual-e5-small"),
        "{preferences}"
    );
    assert!(preferences.contains("gpu"), "{preferences}");

    let restarted = run_workspace_input(&fixture, "1\n/model info e5-small\n/exit\n", true);
    assert!(restarted.status.success(), "{}", stderr(&restarted));
    let restarted_text = stdout(&restarted);
    assert!(
        restarted_text.contains("Назначено: GPU · DirectML"),
        "{restarted_text}"
    );
}

#[test]
fn result_navigation_before_search_has_actionable_error_and_chat_recovers() {
    let fixture = Fixture::new();
    let input = create_workspace_then("/next\n/status\n/exit\n");
    let output = run_workspace_input(&fixture, &input, true);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("Сначала выполните поиск"), "{text}");
    assert!(text.matches("FASTSEARCH").count() >= 2, "{text}");
}

#[test]
fn search_results_can_be_repeated_and_opened_by_stable_number() {
    let fixture = Fixture::new();
    let input = create_workspace_then("/index update\nFastSearch\n/repeat\n/open 1\n/exit\n");
    let output = run_workspace_input(&fixture, &input, true);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.matches("РЕЗУЛЬТАТЫ").count() >= 2,
        "repeat did not render the stored search page: {text}"
    );
    assert!(text.contains("Страница 1 из 1"), "{text}");
    assert!(
        text.contains("ЗАПИСЬ"),
        "open did not render a record: {text}"
    );
    assert!(text.contains("КОНТЕКСТ ПОИСКА"), "{text}");
    assert!(text.contains("№  ТОЧН"), "{text}");
    assert!(text.contains("1  100%"), "{text}");
    assert!(text.contains("guide.md"), "{text}");
    assert!(text.contains("Удобный интерактивный поиск."), "{text}");
    assert!(
        text.contains("1  100%  guide.md\n           Удобный интерактивный поиск."),
        "result card must keep source and excerpt aligned without labels: {text}"
    );
}

#[test]
fn bare_console_text_is_a_balanced_search_query() {
    let fixture = Fixture::new();
    let input =
        create_workspace_then("/index update\nкак быстро создать дом из ригелей и стоек\n/exit\n");
    let output = run_workspace_input(&fixture, &input, false);

    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(!text.contains("UNKNOWN_COMMAND"), "{text}");
    assert!(text.contains("ПОИСК — ВЫПОЛНЯЕТСЯ"), "{text}");
    assert!(text.contains("НИЧЕГО НЕ НАЙДЕНО"), "{text}");
}

#[test]
fn interactive_search_treats_ranking_mode_words_as_query_text() {
    let fixture = Fixture::new();
    fs::write(
        fixture.documents.join("ranking.md"),
        "# current\n\nRanking behavior",
    )
    .expect("ranking fixture");
    let input = create_workspace_then("/index update\n/search current\n/exit\n");
    let output = run_workspace_input(&fixture, &input, true);

    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("Запрос: «current»"), "{text}");
}

#[test]
fn catalog_reoffers_a_known_workspace_outside_its_directory() {
    let fixture = Fixture::new();
    let first = run_workspace_input(&fixture, &create_workspace_then("/exit\n"), true);
    assert!(first.status.success(), "{}", stderr(&first));

    let second =
        run_workspace_input_from(&fixture, std::env::temp_dir().as_path(), "1\n/exit\n", true);
    assert!(second.status.success(), "{}", stderr(&second));
    let text = stdout(&second);
    assert!(text.contains("НЕДАВНИЕ ОБЛАСТИ"), "{text}");
    assert!(text.contains(&path_text(fixture.root())), "{text}");
    assert!(text.contains("Источники: документация · код"), "{text}");
}

#[test]
fn zero_source_workspace_is_valid_and_explains_how_to_continue() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.documents.join("guide.md")).expect("remove document fixture");
    fs::remove_file(fixture.code.join("lib.rs")).expect("remove code fixture");

    let output = run_workspace_input(&fixture, &create_workspace_then("/sources\n/exit\n"), true);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("Источники: не настроены"), "{text}");
    assert!(text.contains("/sources set"), "{text}");
    assert!(fixture.root().join(".fastsearch/workspace.toml").is_file());
}

#[test]
fn sources_screen_explains_navigation_and_hides_internal_windows_paths() {
    let fixture = Fixture::new();
    let output = run_workspace_input(&fixture, &create_workspace_then("/sources\n/exit\n"), true);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);

    assert!(text.contains("ИСТОЧНИКИ"), "{text}");
    assert!(
        text.contains("Здесь показаны папки, включённые в поиск."),
        "{text}"
    );
    assert!(
        text.contains("символ `-` означает: не использовать этот тип источников"),
        "{text}"
    );
    assert!(
        text.contains("Это справочный экран; отдельный режим не открыт."),
        "{text}"
    );
    assert!(
        text.contains(
            "/status           — вернуться к сводке рабочей области\n  /help             — показать все команды\n  /exit             — закрыть FastSearch"
        ),
        "{text}"
    );
    assert!(text.contains(&path_text(&fixture.documents)), "{text}");
    assert!(!text.contains(r"\\?\"), "{text}");
}

#[test]
fn sources_discover_restores_a_contour_without_technical_paths() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.code.join("lib.rs")).expect("remove code fixture");
    let created = run_workspace_input(&fixture, &create_workspace_then("/exit\n"), true);
    assert!(created.status.success(), "{}", stderr(&created));

    fs::write(fixture.code.join("lib.rs"), "pub fn restored() {}\n").expect("restore code fixture");
    let output = run_workspace_input(&fixture, "/sources discover\n\n/exit\n", true);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("ОБНАРУЖЕННЫЙ КОД"), "{text}");
    assert!(text.contains("КОД\n\n  code sources"), "{text}");
    assert!(!text.contains("service:"), "{text}");
}

#[test]
fn one_contour_workspace_does_not_require_the_other_contour() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.code.join("lib.rs")).expect("remove code fixture");

    let output = run_workspace_input(
        &fixture,
        &create_workspace_then("/index update\nFastSearch\n/exit\n"),
        true,
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains("Источники: документация · 1 корней"),
        "{text}"
    );
    assert!(text.contains("РЕЗУЛЬТАТЫ"), "{text}");
    assert!(!text.contains("Папка исходного кода"), "{text}");
}

#[test]
fn opening_a_workspace_never_updates_the_index_implicitly() {
    let fixture = Fixture::new();
    let output = run_workspace_input(&fixture, &create_workspace_then("/exit\n"), true);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("Индекс: устарел"), "{text}");
    assert!(text.contains("Поиск пока недоступен"), "{text}");
    assert!(text.contains("/index update"), "{text}");
    for action in [
        "/index update",
        "/compare",
        "/sources",
        "/model",
        "/help",
        "/exit",
    ] {
        assert!(text.contains(action), "missing {action}: {text}");
    }
    assert!(!text.contains("ОБНОВЛЕНИЕ ИНДЕКСА"), "{text}");
}

#[test]
fn current_workspace_shows_the_primary_step_and_vertical_navigation() {
    let fixture = Fixture::new();
    let indexed = run_workspace_input(
        &fixture,
        &create_workspace_then("/index update\n/exit\n"),
        true,
    );
    assert!(indexed.status.success(), "{}", stderr(&indexed));

    let output = run_workspace_input(&fixture, "/exit\n", true);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("Индекс: актуален"), "{text}");
    for action in [
        "<текст запроса>",
        "/model",
        "/model <номер|slug>",
        "/compare",
        "/index",
        "/sources",
        "/help",
        "/exit",
    ] {
        assert!(text.contains(action), "missing {action}: {text}");
    }
}

#[test]
fn stale_workspace_rejects_bare_search_before_starting_search_progress() {
    let fixture = Fixture::new();
    let output = run_workspace_input(
        &fixture,
        &create_workspace_then("FastSearch\n/exit\n"),
        true,
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("SEARCH_NOT_READY"), "{text}");
    assert!(text.contains("индекс требует обновления"), "{text}");
    assert!(!text.contains("ПОИСК — ВЫПОЛНЯЕТСЯ"), "{text}");
}

#[test]
fn compare_entry_is_read_only_and_returns_to_the_workspace() {
    let fixture = Fixture::new();
    let input = create_workspace_then(
        "/compare\n/status\nпроверка единого запроса\n/back\n/status\n/exit\n",
    );
    let output = run_workspace_input(&fixture, &input, true);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("ГОТОВНОСТЬ СРАВНЕНИЯ"), "{text}");
    assert!(text.contains("Готово моделей:"), "{text}");
    assert!(text.contains("проверка не загружает модели"), "{text}");
    assert!(text.contains("НЕ ГОТОВ · устарел"), "{text}");
    assert!(
        text.contains("Сначала подготовьте модельные индексы"),
        "{text}"
    );
    assert!(
        text.contains("/update — подготовить модельные индексы"),
        "{text}"
    );
    assert!(text.contains("РАЗМЕР"), "{text}");
    assert!(text.contains("ПОСТРОЕНИЕ"), "{text}");
    assert!(text.contains("Нет ни одной готовой модели"), "{text}");
    assert!(!text.contains("ПОДГОТОВКА СРАВНЕНИЯ"), "{text}");
    assert!(text.matches("FASTSEARCH").count() >= 2, "{text}");
    assert!(
        !fixture
            .root()
            .join(".fastsearch/local/index/vector")
            .read_dir()
            .is_ok_and(|mut entries| entries.next().is_some())
    );
}

#[test]
fn redirected_output_and_no_color_never_emit_ansi() {
    let help = run_with_input(&["--help".to_owned()], "", true);
    assert!(help.status.success());
    assert_no_ansi(&stdout(&help));
    assert_no_ansi(&stderr(&help));

    let fixture = Fixture::new();
    let onboarding = run_workspace_input(&fixture, "", true);
    assert!(onboarding.status.success());
    assert_no_ansi(&stdout(&onboarding));
    assert_no_ansi(&stderr(&onboarding));
}
