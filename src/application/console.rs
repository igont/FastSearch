mod commands;
mod comparison;
mod guidance;
mod index;
mod model;

use commands::{workspace_catalog, workspace_help_catalog};
use comparison::{ComparisonTransition, run_comparison};
use guidance as ui_guidance;
use index::run_index;
use model::provision_model_with_ui;

use std::{
    io::{self, BufRead, IsTerminal, Write},
    path::{Path, PathBuf},
    time::Instant,
};

use terminal_dialogue::{
    ActionItem, ChatSession, ColorPolicy, CommandResolution, NavigationAction, NextStep,
    NoticeDocument, ProgressDashboard, ProgressDocument, ProgressPhase, ProgressPort,
    ProgressState, ProgressTaskSpec, ProgressUnit, PromptFeedback, PromptOutcome, ReportDocument,
    ReportSection, ResultDocument, ResultItem, ResultPager, SectionDocument, SessionConfig,
    TableColumn, TableDocument, TableRow, TerminalDocument, TextStyle, UserErrorDocument,
    run_progress_dashboard,
};

use crate::{
    domain::{
        DeviceCapabilityStatus, EmbeddingModelId, ExecutionDevice, IndexFreshness, RecordKind,
        RelatedQuery, SearchHit, SearchMode, SearchQuery, StableId,
    },
    ports::AgentSurface,
};

use super::comparison::{ComparisonModelStage, ComparisonSharedStage, ComparisonUpdateProgress};
use super::model_cache::{
    configured_model_device, model_device_capability, set_configured_model_device,
};
use super::{
    ComparisonCoordinator, ComparisonReadiness, ComparisonRun, MODEL_CATALOG, ProductionRuntime,
    cli::{CommandOutcome, human_outcome_document, presenters::human_freshness},
    embedding_model_cache_status, model_descriptor, model_runtime_capabilities,
    workspace::{DiscoveryReport, WorkspaceCatalog, WorkspaceProfile, WorkspaceStore},
};

#[derive(Debug)]
struct SearchSession {
    pager: ResultPager<SearchHit>,
    query: String,
    model: String,
    latency_ms: u128,
}

#[derive(Clone, Copy)]
struct SourceUpdateFailure {
    code: &'static str,
    hint: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkspaceTransition {
    Switch,
    Exit,
}

/// Runs the human-oriented workspace console through terminal-dialogue.
pub fn run_interactive() -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let input_is_echoed = stdin.is_terminal();
    let output_is_terminal = stdout.is_terminal();
    let mut input = stdin.lock();
    let mut output = stdout.lock();
    run_interactive_session(
        &mut input,
        &mut output,
        output_is_terminal,
        input_is_echoed,
        std::env::current_dir()?,
    )
}

fn run_interactive_session<R: BufRead>(
    input: &mut R,
    output: &mut dyn Write,
    output_is_terminal: bool,
    input_is_echoed: bool,
    current_dir: PathBuf,
) -> io::Result<()> {
    let config = SessionConfig::for_terminal(output_is_terminal, input_is_echoed)
        .with_color_policy(ColorPolicy::Auto);
    let mut chat = ChatSession::standard(input, output, config);
    chat.session(
        &SectionDocument::new("FastSearch")
            .with_paragraph("Локальный поиск по документации и исходному коду.")
            .to_dialogue_document(&terminal_dialogue::LanguagePack::russian()),
    )?;

    let mut catalog = match WorkspaceCatalog::load_default() {
        Ok(catalog) => catalog,
        Err(error) => {
            show_error(
                &mut chat,
                "CATALOG_UNAVAILABLE",
                error.message(),
                "Исправьте или переместите catalog.json в каталоге FastSearch.",
            )?;
            return Ok(());
        }
    };
    let mut preferred = discover_current_workspace(&current_dir, &mut catalog, &mut chat)?;
    loop {
        let store = match preferred.take() {
            Some(store) => Some(store),
            None => select_workspace(&mut chat, &mut catalog, &current_dir)?,
        };
        let Some(store) = store else {
            show_notice(&mut chat, "FastSearch закрыт.")?;
            return Ok(());
        };
        match run_workspace(&mut chat, &mut catalog, store)? {
            WorkspaceTransition::Switch => {}
            WorkspaceTransition::Exit => return Ok(()),
        }
    }
}

fn discover_current_workspace<R: BufRead>(
    current_dir: &Path,
    catalog: &mut WorkspaceCatalog,
    chat: &mut ChatSession<'_, R>,
) -> io::Result<Option<WorkspaceStore>> {
    if let Some(root) = workspace_marker_ancestor(current_dir) {
        match WorkspaceStore::open(&root) {
            Ok(store) => {
                register_workspace(catalog, &store, chat)?;
                return Ok(Some(store));
            }
            Err(error) => show_error(
                chat,
                "WORKSPACE_INVALID",
                error.message(),
                "Исправьте .fastsearch/workspace.toml или выберите другую область.",
            )?,
        }
    }
    let Some(entry) = catalog.resolve_path(current_dir) else {
        return Ok(None);
    };
    match WorkspaceStore::open(entry.path()) {
        Ok(store) => Ok(Some(store)),
        Err(error) => {
            show_error(
                chat,
                "WORKSPACE_UNAVAILABLE",
                error.message(),
                "Область могла быть перемещена. Выберите или создайте её заново.",
            )?;
            Ok(None)
        }
    }
}

fn select_workspace<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    catalog: &mut WorkspaceCatalog,
    current_dir: &Path,
) -> io::Result<Option<WorkspaceStore>> {
    loop {
        if catalog.entries().is_empty() {
            chat.show_typed(&terminal_dialogue::EmptyStateDocument {
                heading: "Рабочие области".to_owned(),
                explanation: "FastSearch ещё не подключил ни одной рабочей области.".to_owned(),
                next_step: NextStep::instruction("Выберите действие:")
                    .with_action(ActionItem::new("N", "создать рабочую область"))
                    .with_action(ActionItem::new("Q", "выйти")),
            })?;
        } else {
            let document = catalog.entries().iter().fold(
                ResultDocument::new(
                    "Недавние области",
                    "Выберите область по номеру или создайте новую.",
                ),
                |document, entry| {
                    document.with_item(ResultItem::new(entry.name(), display_path(entry.path())))
                },
            );
            chat.show_typed(
                &document.with_next_step(
                    NextStep::instruction("Выберите действие:")
                        .with_action(ActionItem::new("<номер>", "открыть область из списка"))
                        .with_action(ActionItem::new("N", "создать рабочую область"))
                        .with_action(ActionItem::new("R <номер>", "удалить область из списка"))
                        .with_action(ActionItem::new("Q", "выйти")),
                ),
            )?;
        }
        let Some(input) = chat.read_command("fastsearch")? else {
            return Ok(None);
        };
        let input = input.trim();
        if is_exit(input) || input.eq_ignore_ascii_case("q") {
            return Ok(None);
        }
        if input.eq_ignore_ascii_case("n") || input.eq_ignore_ascii_case("new") {
            return create_workspace(chat, catalog, current_dir);
        }
        if let Some(number) = input
            .strip_prefix("r ")
            .or_else(|| input.strip_prefix("R "))
            .and_then(|value| value.trim().parse::<usize>().ok())
        {
            let Some(entry) = catalog.entries().get(number.saturating_sub(1)) else {
                show_error(
                    chat,
                    "WORKSPACE_NUMBER",
                    "Области с таким номером нет.",
                    "Выберите номер из показанного списка.",
                )?;
                continue;
            };
            let id = entry.id().to_owned();
            catalog.remove(&id);
            if let Err(error) = catalog.save_default() {
                show_error(
                    chat,
                    "CATALOG_WRITE",
                    error.message(),
                    "Повторите действие.",
                )?;
            } else {
                show_notice(chat, "Область удалена только из глобального списка.")?;
            }
            continue;
        }
        let Some(index) = input
            .parse::<usize>()
            .ok()
            .and_then(|number| number.checked_sub(1))
        else {
            show_error(
                chat,
                "WORKSPACE_SELECTION",
                "Введите номер области, N или Q.",
                "Команды показаны под списком.",
            )?;
            continue;
        };
        let Some(entry) = catalog.entries().get(index) else {
            show_error(
                chat,
                "WORKSPACE_NUMBER",
                "Области с таким номером нет.",
                "Выберите номер из показанного списка.",
            )?;
            continue;
        };
        match WorkspaceStore::open(entry.path()) {
            Ok(store) => {
                register_workspace(catalog, &store, chat)?;
                return Ok(Some(store));
            }
            Err(error) => show_error(
                chat,
                "WORKSPACE_UNAVAILABLE",
                error.message(),
                "Область можно удалить из списка и подключить заново.",
            )?,
        }
    }
}

fn create_workspace<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    catalog: &mut WorkspaceCatalog,
    current_dir: &Path,
) -> io::Result<Option<WorkspaceStore>> {
    chat.show_typed(
        &SectionDocument::new("Новая рабочая область")
            .with_paragraph("Укажите один корневой каталог. Enter использует текущую папку."),
    )?;
    let root = match chat.prompt_field_with_cancellation("root", is_exit, |value| {
        let candidate = if value.trim().is_empty() {
            current_dir.to_path_buf()
        } else {
            PathBuf::from(value.trim())
        };
        candidate
            .canonicalize()
            .ok()
            .filter(|path| path.is_dir())
            .ok_or_else(|| {
                PromptFeedback::invalid_input("Корневая папка не найдена.")
                    .with_hint("Введите существующий каталог или Enter для текущей папки.")
            })
    })? {
        PromptOutcome::Value(root) => root,
        PromptOutcome::Cancelled | PromptOutcome::EndOfInput => return Ok(None),
    };
    if root.join(".fastsearch/workspace.toml").is_file() {
        return match WorkspaceStore::open(&root) {
            Ok(store) => {
                register_workspace(catalog, &store, chat)?;
                Ok(Some(store))
            }
            Err(error) => {
                show_error(
                    chat,
                    "WORKSPACE_INVALID",
                    error.message(),
                    "Исправьте существующую конфигурацию перед повторным подключением.",
                )?;
                Ok(None)
            }
        };
    }

    chat.show_typed(&ProgressDocument::new(
        "Поиск источников",
        ProgressState::Running,
        "FastSearch исследует выбранную область…",
    ))?;
    let discovery = match DiscoveryReport::scan(&root) {
        Ok(report) => report,
        Err(error) => {
            show_error(
                chat,
                "DISCOVERY_FAILED",
                error.message(),
                "Проверьте доступ к каталогам и повторите создание области.",
            )?;
            return Ok(None);
        }
    };
    show_discovery(chat, &root, &discovery, ui_guidance::discovery_create())?;
    let decision =
        match chat.prompt_field_with_cancellation("create", is_exit, |value| {
            match value.trim().to_lowercase().as_str() {
                "" | "yes" | "y" | "да" => Ok(true),
                "e" | "edit" | "изменить" => Ok(false),
                _ => Err(PromptFeedback::invalid_input(
                    "Enter создаёт область; E изменяет источники; /exit отменяет.",
                )),
            }
        })? {
            PromptOutcome::Value(value) => value,
            PromptOutcome::Cancelled | PromptOutcome::EndOfInput => return Ok(None),
        };
    let (documents, code) = if decision {
        (
            discovery.documentation_roots().to_vec(),
            discovery.code_roots().to_vec(),
        )
    } else {
        edit_source_roots(
            chat,
            &root,
            discovery.documentation_roots(),
            discovery.code_roots(),
        )?
    };
    let name = root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("Workspace");
    let model = EmbeddingModelId::default();
    let profile = match WorkspaceProfile::from_roots(&root, name, documents, code)
        .map(|profile| profile.with_embedding_model(model))
    {
        Ok(profile) => profile,
        Err(error) => {
            show_error(
                chat,
                "SOURCE_ADMISSION",
                error.message(),
                "Все источники должны быть каталогами внутри рабочей области.",
            )?;
            return Ok(None);
        }
    };
    match WorkspaceStore::create(&root, profile) {
        Ok(store) => {
            register_workspace(catalog, &store, chat)?;
            show_notice(chat, "Рабочая область создана.")?;
            Ok(Some(store))
        }
        Err(error) => {
            show_error(
                chat,
                "WORKSPACE_CREATE",
                error.message(),
                "Проверьте права записи в корневой каталог.",
            )?;
            Ok(None)
        }
    }
}

fn edit_source_roots<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    workspace_root: &Path,
    documents: &[PathBuf],
    code: &[PathBuf],
) -> io::Result<(Vec<PathBuf>, Vec<PathBuf>)> {
    show_notice(
        chat,
        "Укажите несколько roots через `;`. Enter сохраняет найденные, `-` отключает contour.",
    )?;
    let documents = prompt_roots(chat, "documentation", workspace_root, documents)?;
    let code = prompt_roots(chat, "code", workspace_root, code)?;
    Ok((documents, code))
}

fn prompt_roots<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    label: &str,
    workspace_root: &Path,
    current: &[PathBuf],
) -> io::Result<Vec<PathBuf>> {
    let current = current.to_vec();
    match chat.prompt_field_with_cancellation(label, is_exit, |value| {
        let value = value.trim();
        if value.is_empty() {
            return Ok(current.clone());
        }
        if value == "-" {
            return Ok(Vec::new());
        }
        let mut roots = Vec::new();
        for raw in value
            .split(';')
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let path = PathBuf::from(raw);
            let path = if path.is_absolute() {
                path
            } else {
                workspace_root.join(path)
            };
            let Some(path) = path.canonicalize().ok().filter(|path| path.is_dir()) else {
                return Err(PromptFeedback::invalid_input(format!(
                    "Каталог не найден: {raw}"
                )));
            };
            if !path.starts_with(workspace_root) {
                return Err(PromptFeedback::invalid_input(
                    "Источник должен находиться внутри рабочей области.",
                ));
            }
            roots.push(path);
        }
        Ok(roots)
    })? {
        PromptOutcome::Value(roots) => Ok(roots),
        PromptOutcome::Cancelled | PromptOutcome::EndOfInput => Ok(current),
    }
}

fn prompt_embedding_model<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    current: EmbeddingModelId,
) -> io::Result<Option<EmbeddingModelId>> {
    show_embedding_models(chat, current, None)?;
    match chat.prompt_field_with_cancellation("model", is_exit, |value| {
        let value = value.trim();
        if value.is_empty() {
            return Ok(current);
        }
        if let Some(model) = resolve_embedding_model(value) {
            return Ok(model);
        }
        Err(PromptFeedback::invalid_input(
            "Введите номер модели, её slug либо /exit для отмены.",
        ))
    })? {
        PromptOutcome::Value(model) => Ok(Some(model)),
        PromptOutcome::Cancelled | PromptOutcome::EndOfInput => Ok(None),
    }
}

fn resolve_embedding_model(value: &str) -> Option<EmbeddingModelId> {
    value
        .trim()
        .parse::<usize>()
        .ok()
        .and_then(|number| MODEL_CATALOG.get(number.saturating_sub(1)))
        .map(|descriptor| descriptor.id)
        .or_else(|| EmbeddingModelId::parse(value.trim()))
}

fn show_embedding_models<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    current: EmbeddingModelId,
    runtime: Option<&ProductionRuntime>,
) -> io::Result<()> {
    let document = MODEL_CATALOG.iter().fold(
        TableDocument::new(
            "Модель поиска",
            vec![
                TableColumn::new(""),
                TableColumn::new("№").right_aligned(),
                TableColumn::new("МОДЕЛЬ"),
                TableColumn::new("СОСТОЯНИЕ"),
                TableColumn::new("CPU"),
                TableColumn::new("GPU"),
                TableColumn::new("ПРОФИЛЬ"),
                TableColumn::new("DIM").right_aligned(),
                TableColumn::new("ЗАГРУЗКА").right_aligned(),
                TableColumn::new("ИНДЕКС").right_aligned(),
            ],
        ),
        |document, descriptor| {
            let selected = if descriptor.id == current { "✓" } else { "" };
            let ready =
                embedding_model_cache_status(descriptor.id).is_ok_and(|status| status.ready());
            let assigned = configured_model_device(descriptor.id).unwrap_or_default();
            let cpu_capability = model_device_capability(descriptor.id, ExecutionDevice::Cpu)
                .unwrap_or(DeviceCapabilityStatus::Unknown);
            let gpu_capability =
                model_device_capability(descriptor.id, ExecutionDevice::GpuDirectMl)
                    .unwrap_or(DeviceCapabilityStatus::Unknown);
            let (cpu, cpu_style) =
                device_assignment_cell(assigned, ExecutionDevice::Cpu, cpu_capability);
            let (gpu, gpu_style) =
                device_assignment_cell(assigned, ExecutionDevice::GpuDirectMl, gpu_capability);
            let number = MODEL_CATALOG
                .iter()
                .position(|candidate| candidate.id == descriptor.id)
                .map_or(1, |index| index + 1);
            document.with_row(
                TableRow::new(vec![
                    selected.to_owned(),
                    number.to_string(),
                    descriptor.id.display_name().to_owned(),
                    if ready {
                        "ГОТОВА"
                    } else {
                        "НУЖНА ЗАГРУЗКА"
                    }
                    .to_owned(),
                    cpu.to_owned(),
                    gpu.to_owned(),
                    descriptor
                        .profile
                        .rsplit_once(" · ")
                        .filter(|(_, suffix)| suffix.contains("измер"))
                        .map_or(descriptor.profile, |(profile, _)| profile)
                        .to_owned(),
                    descriptor.id.dimension().to_string(),
                    format!(
                        "~{:.2} ГБ",
                        descriptor.approximate_download_bytes as f64 / 1_000_000_000.0
                    ),
                    runtime
                        .and_then(|runtime| {
                            runtime
                                .model_partition_metrics(descriptor.id)
                                .ok()
                                .flatten()
                        })
                        .map_or_else(
                            || "НЕТ".to_owned(),
                            |metrics| human_bytes(metrics.size_bytes()),
                        ),
                ])
                .with_cell_style(4, cpu_style)
                .with_cell_style(5, gpu_style),
            )
        },
    );
    chat.show_typed(&document.with_next_step(ui_guidance::model_catalog()))
}

fn device_assignment_cell(
    assigned: ExecutionDevice,
    candidate: ExecutionDevice,
    capability: DeviceCapabilityStatus,
) -> (&'static str, TextStyle) {
    if assigned == candidate {
        return ("✓", TextStyle::Success);
    }
    match capability {
        DeviceCapabilityStatus::Ready => ("", TextStyle::Body),
        DeviceCapabilityStatus::Unknown => ("?", TextStyle::Body),
        DeviceCapabilityStatus::Unavailable => ("✗", TextStyle::Error),
    }
}

fn show_embedding_model<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    model: EmbeddingModelId,
) -> io::Result<()> {
    let descriptor = model_descriptor(model);
    let capabilities = model_runtime_capabilities(model).ok();
    let assigned = configured_model_device(model).unwrap_or_default();
    let cpu = if assigned == ExecutionDevice::Cpu {
        "✓"
    } else {
        ""
    };
    let gpu = if assigned == ExecutionDevice::GpuDirectMl {
        "✓"
    } else {
        capabilities
            .as_ref()
            .map_or("?", |capability| match capability.gpu() {
                DeviceCapabilityStatus::Ready => "",
                other => other.marker(),
            })
    };
    let gpu_backend = capabilities
        .as_ref()
        .and_then(|capability| capability.gpu_backend())
        .unwrap_or("ещё не проверен");
    let gpu_detail = capabilities
        .as_ref()
        .and_then(|capability| capability.gpu_detail());
    let mut runtime_section = ReportSection::new("Устройства")
        .with_line(format!("Назначено: {}", assigned.label()))
        .with_line(format!("CPU: {cpu}"))
        .with_line(format!("GPU: {gpu} · {gpu_backend}"));
    if let Some(detail) = gpu_detail {
        runtime_section = runtime_section.with_line(format!("Проверка GPU: {detail}"));
    }
    chat.show_typed(
        &ReportDocument::new()
            .with_section(
                ReportSection::new(model.display_name())
                    .with_line(format!("Slug: {}", model.slug()))
                    .with_line(format!("Профиль: {}", descriptor.profile))
                    .with_line(format!("Размерность: {}", model.dimension()))
                    .with_line(format!(
                        "Размер загрузки: ~{:.2} ГБ",
                        descriptor.approximate_download_bytes as f64 / 1_000_000_000.0
                    ))
                    .with_line(format!("Источник: {}", descriptor.source_url))
                    .with_line(format!("Ревизия: {}", descriptor.revision)),
            )
            .with_section(runtime_section)
            .with_next_step(ui_guidance::model_detail()),
    )
}

fn run_workspace<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    catalog: &mut WorkspaceCatalog,
    mut store: WorkspaceStore,
) -> io::Result<WorkspaceTransition> {
    let mut runtime = open_workspace_runtime(chat, &store)?;
    show_workspace(chat, &store, runtime.as_ref())?;
    let legacy = store.legacy_locations();
    if !legacy.is_empty() {
        show_notice(
            chat,
            "Обнаружено legacy-состояние (.cfknowledge или .search). Оно оставлено нетронутым; рабочий индекс находится в .fastsearch/local.",
        )?;
    }
    if store.profile().contour_count() == 0 {
        show_no_sources(chat)?;
    }
    let commands = workspace_catalog();
    let mut last_search: Option<SearchSession> = None;
    let mut model_number_expected = false;
    loop {
        let Some(line) = chat.read_command("fastsearch")? else {
            show_notice(chat, "Ввод завершён. FastSearch закрыт.")?;
            return Ok(WorkspaceTransition::Exit);
        };
        let trimmed = line.trim();
        let numeric_model_selection = model_number_expected
            && !trimmed.is_empty()
            && trimmed.chars().all(|character| character.is_ascii_digit());
        model_number_expected = false;
        let (name, arguments) = if numeric_model_selection {
            ("model set".to_owned(), trimmed.to_owned())
        } else {
            match commands.resolve(&line) {
                CommandResolution::Empty => continue,
                CommandResolution::Unknown { .. } if !line.trim_start().starts_with('/') => {
                    ("search".to_owned(), line.trim().to_owned())
                }
                CommandResolution::Unknown { suggestion, .. } => {
                    let mut error = UserErrorDocument::new("Неизвестная команда.")
                        .with_code("UNKNOWN_COMMAND")
                        .with_hint("Введите /help, чтобы увидеть доступные действия.");
                    if let Some(suggestion) = suggestion {
                        error = error.with_action(ActionItem::new(
                            format!("/{suggestion}"),
                            "возможная команда",
                        ));
                    }
                    chat.show_typed(&error)?;
                    continue;
                }
                CommandResolution::Match { index, arguments } => {
                    (commands.commands()[index].name.clone(), arguments)
                }
            }
        };

        if matches!(name.as_str(), "open" | "next" | "prev" | "page" | "repeat") {
            handle_navigation(
                chat,
                runtime.as_ref(),
                last_search.as_mut(),
                &name,
                &arguments,
            )?;
            continue;
        }

        match name.as_str() {
            "exit" => {
                show_notice(chat, "FastSearch закрыт.")?;
                return Ok(WorkspaceTransition::Exit);
            }
            "workspace" => return Ok(WorkspaceTransition::Switch),
            "help" => {
                chat.show_typed(&workspace_help_catalog().welcome_document(
                    "Команды FastSearch",
                    "Обычный текст выполняет поиск в открытой рабочей области.",
                ))?;
            }
            "version" => show_notice(chat, &version_text())?,
            "status" => show_workspace(chat, &store, runtime.as_ref())?,
            "sources" => show_sources(chat, &store)?,
            "sources discover" => {
                handle_source_discovery(chat, catalog, &mut store, &mut runtime, &mut last_search)?;
            }
            "sources set" => {
                handle_source_edit(chat, catalog, &mut store, &mut runtime, &mut last_search)?;
            }
            "index" => show_index_status(chat, runtime.as_ref())?,
            "model" if arguments.trim().is_empty() => {
                show_embedding_models(chat, store.profile().embedding_model(), runtime.as_ref())?;
                model_number_expected = true;
            }
            "model" => handle_model_selection(
                chat,
                &mut store,
                &mut runtime,
                &mut last_search,
                &mut model_number_expected,
                &arguments,
            )?,
            "model info" => {
                let Some(model) = resolve_embedding_model(&arguments) else {
                    show_error(
                        chat,
                        "MODEL_INFO",
                        "Модель не распознана.",
                        "Укажите номер или slug из /model.",
                    )?;
                    continue;
                };
                show_embedding_model(chat, model)?;
            }
            "model device" => {
                handle_model_device(chat, &store, &mut runtime, &mut last_search, &arguments)?
            }
            "compare" => {
                let Some(runtime) = runtime.as_mut() else {
                    show_no_sources(chat)?;
                    continue;
                };
                if run_comparison(chat, runtime)? == ComparisonTransition::Exit {
                    show_notice(chat, "FastSearch закрыт.")?;
                    return Ok(WorkspaceTransition::Exit);
                }
            }
            "model set" => handle_model_selection(
                chat,
                &mut store,
                &mut runtime,
                &mut last_search,
                &mut model_number_expected,
                &arguments,
            )?,
            "experiment record" => {
                let Some(search) = last_search.as_ref() else {
                    show_error(
                        chat,
                        "EXPERIMENT_NO_SEARCH",
                        "Сначала выполните поисковый запрос.",
                        "После выдачи используйте /experiment record <оценка>.",
                    )?;
                    continue;
                };
                if arguments.trim().is_empty() {
                    show_error(
                        chat,
                        "EXPERIMENT_NOTE",
                        "Добавьте краткую оценку качества результатов.",
                        "Например: /experiment record релевантный файл на первом месте.",
                    )?;
                    continue;
                }
                match store.record_embedding_experiment(
                    &search.query,
                    search.pager.total_items(),
                    search.latency_ms,
                    &arguments,
                ) {
                    Ok(path) => show_notice(
                        chat,
                        &format!("Результат эксперимента записан: {}", display_path(&path)),
                    )?,
                    Err(error) => show_error(
                        chat,
                        "EXPERIMENT_WRITE",
                        error.message(),
                        "Проверьте доступ к .fastsearch/knowledge/experiments.",
                    )?,
                }
            }
            "index update" => {
                run_index(chat, runtime.as_mut(), false)?;
                last_search = None;
            }
            "index rebuild" => {
                if let Some(runtime) = runtime.as_mut() {
                    run_index(chat, Some(runtime), true)?;
                    last_search = None;
                } else {
                    show_no_sources(chat)?;
                }
            }
            "index clear" => {
                let selector = arguments.trim();
                let selected_model = if selector.is_empty() {
                    None
                } else if let Some(model) = resolve_embedding_model(selector) {
                    Some(model)
                } else {
                    show_error(
                        chat,
                        "MODEL_INDEX_CLEAR",
                        "Модель не распознана.",
                        "Укажите номер или slug из /model, либо не указывайте аргумент для очистки всех моделей.",
                    )?;
                    continue;
                };

                // Drop the active runtime before deleting its partition so
                // Windows never retains an in-memory view of cleared files.
                drop(runtime.take());
                match store.clear_model_indexes(selected_model) {
                    Ok(()) => {
                        last_search = None;
                        show_notice(
                            chat,
                            &match selected_model {
                                Some(model) => {
                                    format!("Индекс модели {} очищен.", model.display_name())
                                }
                                None => "Индексы всех моделей очищены.".to_owned(),
                            },
                        )?;
                    }
                    Err(error) => show_error(
                        chat,
                        "MODEL_INDEX_CLEAR",
                        error.message(),
                        "Проверьте доступ к .fastsearch/local/index/vector и повторите действие.",
                    )?,
                }
                runtime = open_workspace_runtime(chat, &store)?;
            }
            "search" => {
                let Some(runtime) = runtime.as_ref() else {
                    show_no_sources(chat)?;
                    continue;
                };
                let freshness = runtime.index_status().freshness();
                if freshness != IndexFreshness::Current {
                    show_search_unavailable(chat, freshness)?;
                    continue;
                }
                let query = arguments.trim().to_owned();
                if query.is_empty() {
                    show_error(
                        chat,
                        "EMPTY_QUERY",
                        "Поисковый запрос не должен быть пустым.",
                        "Введите текст обычной строкой.",
                    )?;
                    continue;
                }
                chat.show_typed(&ProgressDocument::new(
                    "Поиск",
                    ProgressState::Running,
                    "FastSearch выполняет запрос…",
                ))?;
                let query_text = query.clone();
                let started = Instant::now();
                match SearchQuery::new(query, SearchMode::Balanced)
                    .and_then(|query| runtime.search(&query))
                {
                    Ok(response) => {
                        last_search = Some(SearchSession {
                            pager: ResultPager::new(response.hits().to_vec(), 5)
                                .expect("FastSearch page size is non-zero"),
                            query: query_text,
                            model: store.profile().embedding_model().display_name().to_owned(),
                            latency_ms: started.elapsed().as_millis(),
                        });
                        show_search_page(
                            chat,
                            last_search.as_ref().expect("search session stored"),
                        )?;
                    }
                    Err(error) => show_error(
                        chat,
                        "SEARCH_FAILED",
                        error.message(),
                        "Проверьте /status или обновите индекс командой /index update.",
                    )?,
                }
            }
            "related" => {
                let Some(runtime) = runtime.as_ref() else {
                    show_no_sources(chat)?;
                    continue;
                };
                let selected_result_id = arguments
                    .parse::<usize>()
                    .ok()
                    .and_then(|number| last_search.as_ref()?.pager.select(number))
                    .map(|hit| hit.record().id().clone());
                let id = match selected_result_id
                    .map(Ok)
                    .unwrap_or_else(|| StableId::parse(arguments.clone()))
                {
                    Ok(id) => id,
                    Err(error) => {
                        show_error(
                            chat,
                            "INVALID_RESULT",
                            error.message(),
                            "Укажите номер из текущей выдачи.",
                        )?;
                        continue;
                    }
                };
                let outcome = runtime
                    .related(&RelatedQuery::new(id))
                    .map(CommandOutcome::Related);
                match outcome {
                    Ok(outcome) => chat.show_typed(&human_outcome_document(&outcome))?,
                    Err(error) => show_error(
                        chat,
                        "LOOKUP_FAILED",
                        error.message(),
                        "Проверьте номер результата и актуальность индекса.",
                    )?,
                }
            }
            _ => unreachable!("static FastSearch command catalog"),
        }
    }
}

fn handle_source_discovery<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    catalog: &mut WorkspaceCatalog,
    store: &mut WorkspaceStore,
    runtime: &mut Option<ProductionRuntime>,
    last_search: &mut Option<SearchSession>,
) -> io::Result<()> {
    let discovery = match DiscoveryReport::scan(store.root()) {
        Ok(discovery) => discovery,
        Err(error) => {
            return show_error(
                chat,
                "SOURCE_DISCOVERY",
                error.message(),
                "Проверьте доступ к рабочей области.",
            );
        }
    };
    show_discovery(
        chat,
        store.root(),
        &discovery,
        ui_guidance::discovery_apply(),
    )?;
    let apply =
        match chat.prompt_field_with_cancellation("apply", is_exit, |value| {
            match value.trim().to_lowercase().as_str() {
                "" | "yes" | "y" | "да" => Ok(true),
                "no" | "n" | "нет" => Ok(false),
                _ => Err(PromptFeedback::invalid_input(
                    "Enter применяет roots; нет или /exit отменяет.",
                )),
            }
        })? {
            PromptOutcome::Value(value) => value,
            PromptOutcome::Cancelled | PromptOutcome::EndOfInput => false,
        };
    if !apply {
        return show_notice(chat, "Обнаруженные roots не применены.");
    }
    let updated = WorkspaceProfile::from_roots(
        store.root(),
        store.profile().name(),
        discovery.documentation_roots().to_vec(),
        discovery.code_roots().to_vec(),
    )
    .map(|profile| profile.with_embedding_model(store.profile().embedding_model()))
    .and_then(|profile| WorkspaceStore::create(store.root(), profile));
    apply_source_update(
        chat,
        catalog,
        store,
        runtime,
        last_search,
        updated,
        SourceUpdateFailure {
            code: "SOURCE_DISCOVERY_APPLY",
            hint: "Измените roots вручную через /sources set.",
        },
    )
}

fn handle_source_edit<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    catalog: &mut WorkspaceCatalog,
    store: &mut WorkspaceStore,
    runtime: &mut Option<ProductionRuntime>,
    last_search: &mut Option<SearchSession>,
) -> io::Result<()> {
    let documents = store
        .profile()
        .documentation_roots()
        .iter()
        .map(|source| source.resolve(store.root()))
        .collect::<Vec<_>>();
    let code = store
        .profile()
        .code_roots()
        .iter()
        .map(|source| source.resolve(store.root()))
        .collect::<Vec<_>>();
    let (documents, code) = edit_source_roots(chat, store.root(), &documents, &code)?;
    let updated =
        WorkspaceProfile::from_roots(store.root(), store.profile().name(), documents, code)
            .map(|profile| profile.with_embedding_model(store.profile().embedding_model()))
            .and_then(|profile| WorkspaceStore::create(store.root(), profile));
    apply_source_update(
        chat,
        catalog,
        store,
        runtime,
        last_search,
        updated,
        SourceUpdateFailure {
            code: "SOURCE_UPDATE",
            hint: "Проверьте roots и повторите настройку.",
        },
    )
}

fn apply_source_update<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    catalog: &mut WorkspaceCatalog,
    store: &mut WorkspaceStore,
    runtime: &mut Option<ProductionRuntime>,
    last_search: &mut Option<SearchSession>,
    updated: Result<WorkspaceStore, crate::domain::FastSearchError>,
    failure_context: SourceUpdateFailure,
) -> io::Result<()> {
    match updated {
        Ok(updated) => {
            *store = updated;
            register_workspace(catalog, store, chat)?;
            *runtime = open_workspace_runtime(chat, store)?;
            *last_search = None;
            show_sources(chat, store)
        }
        Err(error) => show_error(
            chat,
            failure_context.code,
            error.message(),
            failure_context.hint,
        ),
    }
}

fn handle_model_device<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    store: &WorkspaceStore,
    runtime: &mut Option<ProductionRuntime>,
    last_search: &mut Option<SearchSession>,
    arguments: &str,
) -> io::Result<()> {
    let mut parts = arguments.split_whitespace().collect::<Vec<_>>();
    let explicit_device = parts.last().and_then(|value| ExecutionDevice::parse(value));
    if explicit_device.is_some() {
        parts.pop();
    }
    let selector = parts.join(" ");
    let Some(model) = resolve_embedding_model(&selector) else {
        return show_error(
            chat,
            "MODEL_DEVICE",
            "Модель не распознана.",
            "Используйте /model device <номер|slug> [cpu|gpu].",
        );
    };
    let current = match configured_model_device(model) {
        Ok(device) => device,
        Err(error) => {
            return show_error(
                chat,
                "MODEL_DEVICE_READ",
                error.message(),
                "Проверьте локальный device-preferences.toml.",
            );
        }
    };
    let requested = explicit_device.unwrap_or_else(|| current.toggled());
    match set_configured_model_device(model, requested) {
        Ok(()) => {
            show_notice(
                chat,
                &format!(
                    "Для {} назначено {}. Настройка сохранена на этом устройстве.",
                    model.display_name(),
                    requested.label()
                ),
            )?;
            if model == store.profile().embedding_model() {
                *runtime = open_workspace_runtime(chat, store)?;
                *last_search = None;
            } else {
                show_embedding_models(chat, store.profile().embedding_model(), runtime.as_ref())?;
            }
            Ok(())
        }
        Err(error) => show_error(
            chat,
            "MODEL_DEVICE_UNAVAILABLE",
            error.message(),
            "Выберите CPU либо модель E5 с успешно проверенным DirectML.",
        ),
    }
}

fn handle_model_selection<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    store: &mut WorkspaceStore,
    runtime: &mut Option<ProductionRuntime>,
    last_search: &mut Option<SearchSession>,
    model_number_expected: &mut bool,
    arguments: &str,
) -> io::Result<()> {
    let selected = if arguments.trim().is_empty() {
        prompt_embedding_model(chat, store.profile().embedding_model())?
    } else {
        resolve_embedding_model(arguments)
    };
    let Some(selected) = selected else {
        return show_error(
            chat,
            "MODEL_SELECTION",
            "Модель не распознана.",
            "Введите /model и используйте номер, slug или краткое имя.",
        );
    };
    if !(cfg!(debug_assertions)
        && std::env::var_os("FASTSEARCH_TEST_DISABLE_MODEL_AUTO_DOWNLOAD").is_some())
        && provision_model_with_ui(chat, selected)?.is_none()
    {
        return show_notice(
            chat,
            "Прежняя активная модель сохранена: новая модель не прошла подготовку.",
        );
    }
    match store.set_embedding_model(selected) {
        Ok(changed) => {
            if changed {
                show_notice(
                    chat,
                    "Модель изменена. Существующие векторы больше не используются; индексирование не запущено.",
                )?;
            }
            *runtime = open_workspace_runtime(chat, store)?;
            *last_search = None;
            show_embedding_models(chat, selected, runtime.as_ref())?;
            *model_number_expected = true;
            Ok(())
        }
        Err(error) => show_error(
            chat,
            "MODEL_WRITE",
            error.message(),
            "Проверьте .fastsearch/workspace.toml и повторите выбор.",
        ),
    }
}

fn open_workspace_runtime<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    store: &WorkspaceStore,
) -> io::Result<Option<ProductionRuntime>> {
    if store.profile().contour_count() == 0 {
        return Ok(None);
    }
    let selected = store.profile().embedding_model();
    let model = if cfg!(debug_assertions)
        && std::env::var_os("FASTSEARCH_TEST_DISABLE_MODEL_AUTO_DOWNLOAD").is_some()
    {
        None
    } else {
        provision_model_with_ui(chat, selected)?
    };
    let execution_device = match configured_model_device(selected) {
        Ok(device) => device,
        Err(error) => {
            show_error(
                chat,
                "MODEL_DEVICE_READ",
                error.message(),
                "Используется CPU; проверьте локальный device-preferences.toml.",
            )?;
            ExecutionDevice::Cpu
        }
    };
    let config = match model {
        Some(model) => store
            .production_config()
            .with_embedding_model(model.model(), model.root().to_path_buf())
            .with_execution_device(execution_device),
        None => store
            .production_config()
            .with_execution_device(execution_device),
    };
    let runtime = match ProductionRuntime::open(config) {
        Ok(runtime) => runtime,
        Err(error) => {
            show_error(
                chat,
                "WORKSPACE_OPEN",
                error.message(),
                "Проверьте /sources и состояние .fastsearch/local.",
            )?;
            return Ok(None);
        }
    };
    show_embedding_models(chat, selected, Some(&runtime))?;
    Ok(Some(runtime))
}

fn handle_navigation<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    runtime: Option<&ProductionRuntime>,
    search: Option<&mut SearchSession>,
    name: &str,
    arguments: &str,
) -> io::Result<()> {
    let Some(search) = search else {
        return show_error(
            chat,
            "NO_RESULTS",
            "Сначала выполните поиск.",
            "Введите поисковый запрос обычным текстом.",
        );
    };
    let input = if arguments.is_empty() {
        name.to_owned()
    } else {
        format!("{name} {arguments}")
    };
    let Some(action) = NavigationAction::parse(&input) else {
        return show_error(
            chat,
            "RESULT_ACTION",
            "Некорректное действие над результатами.",
            "Используйте /open <номер>, /next, /prev, /page <номер> или /repeat.",
        );
    };
    if let NavigationAction::Open(number) = action {
        let Some(hit) = search.pager.select(number) else {
            return show_error(
                chat,
                "RESULT_NUMBER",
                "Результата с таким номером нет.",
                "Выберите номер из текущей выдачи.",
            );
        };
        let Some(runtime) = runtime else {
            return show_no_sources(chat);
        };
        match runtime.get(hit.record().id()) {
            Ok(Some(record)) => {
                let summary = ReportSection::new("Запись")
                    .with_line(format!("Заголовок: {}", record.title()))
                    .with_line(format!("Тип: {}", record_label(record.kind())))
                    .with_line(format!("Файл: {}", record.locator().path()));
                chat.show_typed(
                    &ReportDocument::new()
                        .with_section(summary)
                        .with_section(
                            ReportSection::new("Контекст поиска")
                                .with_line(format!("Запрос: {}", search.query))
                                .with_line(format!("Канал: {:?}", hit.channel()))
                                .with_line(format!("Оценка: {:.4}", hit.score())),
                        )
                        .with_section(
                            ReportSection::new("Содержимое").with_line(record.searchable_content()),
                        )
                        .with_next_step(ui_guidance::result_detail()),
                )
            }
            Ok(None) => show_error(
                chat,
                "RESULT_MISSING",
                "Запись больше не находится в текущем индексе.",
                "Обновите индекс и повторите поиск.",
            ),
            Err(error) => show_error(
                chat,
                "RESULT_OPEN",
                error.message(),
                "Обновите индекс и повторите поиск.",
            ),
        }
    } else {
        if let Err(error) = search.pager.navigate(action) {
            return show_error(
                chat,
                "RESULT_NAVIGATION",
                &error.to_string(),
                "Проверьте номер страницы.",
            );
        }
        show_search_page(chat, search)
    }
}

fn show_workspace<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    store: &WorkspaceStore,
    runtime: Option<&ProductionRuntime>,
) -> io::Result<()> {
    let profile = store.profile();
    let freshness = runtime.map(|runtime| runtime.index_status().freshness());
    let status = freshness.map_or("не настроен", human_freshness);
    let next_step = ui_guidance::workspace(freshness);
    chat.show_typed(
        &ReportDocument::new()
            .with_section(
                ReportSection::new("FastSearch")
                    .with_line(format!("Область: {}", profile.name()))
                    .with_line(format!("Корень: {}", display_path(store.root())))
                    .with_line(format!(
                        "Источники: {}",
                        contour_summary(
                            profile.documentation_roots().len(),
                            profile.code_roots().len()
                        )
                    ))
                    .with_line(format!(
                        "Модель: {}",
                        profile.embedding_model().display_name()
                    ))
                    .with_line(format!("Индекс: {status}")),
            )
            .with_next_step(next_step),
    )
}

fn show_search_unavailable<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    freshness: IndexFreshness,
) -> io::Result<()> {
    let Some(error) = ui_guidance::search_unavailable(freshness) else {
        return Ok(());
    };
    chat.show_typed(&error)
}

fn show_sources<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    store: &WorkspaceStore,
) -> io::Result<()> {
    let documents = roots_lines(store, store.profile().documentation_roots());
    let code = roots_lines(store, store.profile().code_roots());
    let mut report = ReportDocument::new()
        .with_section(
            ReportSection::new("Источники")
                .with_line("Здесь показаны папки, включённые в поиск.")
                .with_line(
                    "В ручной настройке символ `-` означает: не использовать этот тип источников.",
                ),
        )
        .with_section(documents.into_iter().fold(
            ReportSection::new("Документация"),
            |section, line| section.with_line(line),
        ));
    report = report.with_section(
        code.into_iter()
            .fold(ReportSection::new("Код"), |section, line| {
                section.with_line(line)
            }),
    );
    chat.show_typed(&report.with_next_step(ui_guidance::sources()))
}

fn roots_lines(store: &WorkspaceStore, roots: &[super::workspace::SourceRoot]) -> Vec<String> {
    if roots.is_empty() {
        return vec!["Не подключена".to_owned()];
    }
    roots
        .iter()
        .map(|source| {
            format!(
                "{} · {}",
                display_relative_path(source.relative_path()),
                display_path(&source.resolve(store.root()))
            )
        })
        .collect()
}

fn show_discovery<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    root: &Path,
    discovery: &DiscoveryReport,
    next_step: NextStep,
) -> io::Result<()> {
    let documents = relative_lines(root, discovery.documentation_roots());
    let code = relative_lines(root, discovery.code_roots());
    let report = ReportDocument::new()
        .with_section(documents.into_iter().fold(
            ReportSection::new("Обнаруженная документация"),
            |section, line| section.with_line(line),
        ))
        .with_section(code.into_iter().fold(
            ReportSection::new("Обнаруженный код"),
            |section, line| section.with_line(line),
        ))
        .with_next_step(next_step);
    chat.show_typed(&report)
}

fn relative_lines(root: &Path, paths: &[PathBuf]) -> Vec<String> {
    if paths.is_empty() {
        return vec!["Не обнаружена".to_owned()];
    }
    paths
        .iter()
        .map(|path| {
            path.strip_prefix(root)
                .ok()
                .filter(|relative| !relative.as_os_str().is_empty())
                .map_or_else(|| ".".to_owned(), |relative| relative.display().to_string())
        })
        .collect()
}

fn show_index_status<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    runtime: Option<&ProductionRuntime>,
) -> io::Result<()> {
    let Some(runtime) = runtime else {
        return show_no_sources(chat);
    };
    chat.show_typed(&human_outcome_document(&CommandOutcome::Status {
        status: runtime.index_status(),
        capabilities: runtime.status(),
    }))
}

fn show_no_sources<R: BufRead>(chat: &mut ChatSession<'_, R>) -> io::Result<()> {
    chat.show_typed(&terminal_dialogue::EmptyStateDocument {
        heading: "Источники не настроены".to_owned(),
        explanation: "FastSearch пока не знает, где искать.".to_owned(),
        next_step: NextStep::instruction(
            "Используйте /sources set, чтобы добавить документацию, код или оба contour.",
        ),
    })
}

fn show_search_page<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    search: &SearchSession,
) -> io::Result<()> {
    if search.pager.is_empty() {
        return chat.show_typed(&terminal_dialogue::EmptyStateDocument {
            heading: "Ничего не найдено".to_owned(),
            explanation: "Совпадений в доступных источниках нет.".to_owned(),
            next_step: NextStep::instruction("Сократите запрос или проверьте /status."),
        });
    }
    let start_index = search.pager.absolute_number(1).unwrap_or(1);
    let best_score = search.pager.select(1).map_or(0.0, SearchHit::score);
    let document = search.pager.visible().iter().fold(
        ResultDocument::new(
            "Результаты",
            format!(
                "Модель: {}\nЗапрос: «{}»\nНайдено: {} · Страница {} из {}",
                search.model,
                search.query,
                search.pager.total_items(),
                search.pager.page_number(),
                search.pager.total_pages(),
            ),
        )
        .with_start_index(start_index),
        |document, hit| {
            document.with_item(
                ResultItem::new(hit.record().title(), hit.record().locator().path())
                    .with_excerpt(full_trigger(hit.record().searchable_content()))
                    .with_match_percent(relative_match_percent(hit.score(), best_score)),
            )
        },
    );
    chat.show_typed(&document.with_next_step(ui_guidance::search_results()))
}

fn relative_match_percent(score: f64, best_score: f64) -> u8 {
    if !score.is_finite() || !best_score.is_finite() || best_score <= 0.0 {
        return 0;
    }
    ((score.max(0.0) / best_score * 100.0)
        .round()
        .clamp(0.0, 100.0)) as u8
}

/// Keeps every searchable character while making the trigger one terminal row.
fn full_trigger(content: &str) -> String {
    let trigger = content.split_whitespace().collect::<Vec<_>>().join(" ");
    if trigger.is_empty() {
        "нет текстового фрагмента".to_owned()
    } else {
        trigger
    }
}

fn display_path(path: &Path) -> String {
    let value = path.display().to_string();
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else {
        value.strip_prefix(r"\\?\").unwrap_or(&value).to_owned()
    }
}

fn display_relative_path(path: &str) -> String {
    path.chars()
        .map(|character| match character {
            '/' | '\\' => std::path::MAIN_SEPARATOR,
            other => other,
        })
        .collect()
}

fn register_workspace<R: BufRead>(
    catalog: &mut WorkspaceCatalog,
    store: &WorkspaceStore,
    chat: &mut ChatSession<'_, R>,
) -> io::Result<()> {
    if let Err(error) = catalog
        .register(store.root(), store.profile())
        .and_then(|()| catalog.save_default())
    {
        show_error(
            chat,
            "CATALOG_WRITE",
            error.message(),
            "Область открыта, но список недавних областей не обновлён.",
        )?;
    }
    Ok(())
}

fn workspace_marker_ancestor(path: &Path) -> Option<PathBuf> {
    let mut current = path.canonicalize().ok()?;
    loop {
        if current.join(".fastsearch/workspace.toml").is_file() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

fn contour_summary(document_roots: usize, code_roots: usize) -> String {
    match (document_roots, code_roots) {
        (0, 0) => "не настроены".to_owned(),
        (documents, 0) => format!("документация · {documents} корней"),
        (0, code) => format!("код · {code} корней"),
        (documents, code) => {
            format!("документация · код · {documents} + {code} корней")
        }
    }
}

fn human_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let bytes = bytes as f64;
    if bytes >= GIB {
        format!("{:.2} GiB", bytes / GIB)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes / MIB)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes / KIB)
    } else {
        format!("{bytes:.0} B")
    }
}

fn record_label(kind: RecordKind) -> &'static str {
    match kind {
        RecordKind::MarkdownSection | RecordKind::RegistryRow => "DOC",
        RecordKind::CodeMap | RecordKind::CodeSymbol => "CODE",
    }
}

fn show_error<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    code: &str,
    message: &str,
    hint: &str,
) -> io::Result<()> {
    chat.show_typed(
        &UserErrorDocument::new(message)
            .with_code(code)
            .with_hint(hint),
    )
}

fn show_notice<R: BufRead>(chat: &mut ChatSession<'_, R>, message: &str) -> io::Result<()> {
    chat.show_typed(&NoticeDocument::new(message))
}

pub use commands::help_text;

#[must_use]
pub fn version_text() -> String {
    format!("FastSearch {}", env!("CARGO_PKG_VERSION"))
}

fn is_exit(value: &str) -> bool {
    matches!(
        value.trim().trim_start_matches('/').to_lowercase().as_str(),
        "exit" | "quit" | "выход"
    )
}

#[cfg(test)]
mod tests {
    use super::{
        contour_summary, device_assignment_cell, display_path, display_relative_path, full_trigger,
        help_text, relative_match_percent, ui_guidance, workspace_catalog, workspace_help_catalog,
    };
    use crate::domain::{DeviceCapabilityStatus, ExecutionDevice};
    use std::path::Path;
    use terminal_dialogue::{CommandResolution, LanguagePack, TerminalDocument, TextStyle};

    #[test]
    fn model_device_cells_mark_only_the_assignment_and_reject_unavailable_devices() {
        assert_eq!(
            device_assignment_cell(
                ExecutionDevice::Cpu,
                ExecutionDevice::Cpu,
                DeviceCapabilityStatus::Ready,
            ),
            ("✓", TextStyle::Success)
        );
        assert_eq!(
            device_assignment_cell(
                ExecutionDevice::Cpu,
                ExecutionDevice::GpuDirectMl,
                DeviceCapabilityStatus::Ready,
            ),
            ("", TextStyle::Body)
        );
        assert_eq!(
            device_assignment_cell(
                ExecutionDevice::Cpu,
                ExecutionDevice::GpuDirectMl,
                DeviceCapabilityStatus::Unavailable,
            ),
            ("✗", TextStyle::Error)
        );
    }

    #[test]
    fn root_help_omits_navigation_and_model_device_uses_the_longest_command_match() {
        let catalog = workspace_catalog();
        assert!(matches!(
            catalog.resolve("/model device 2 gpu"),
            CommandResolution::Match { arguments, .. } if arguments == "2 gpu"
        ));
        assert!(matches!(
            catalog.resolve("/index clear 2"),
            CommandResolution::Match { arguments, .. } if arguments == "2"
        ));
        let help = workspace_help_catalog()
            .welcome_document("Команды", "Сводка")
            .to_dialogue_document(&LanguagePack::russian())
            .render(false);
        for heading in [
            "ПОИСК",
            "ИСТОЧНИКИ И ИНДЕКС",
            "МОДЕЛИ И СРАВНЕНИЕ",
            "ПРИЛОЖЕНИЕ",
        ] {
            assert!(help.contains(heading), "{help}");
        }
        assert!(!help.contains("НАВИГАЦИЯ"), "{help}");
        assert!(!help.contains("/open <номер>"), "{help}");

        let commands = [
            ui_guidance::model_catalog(),
            ui_guidance::model_detail(),
            ui_guidance::result_detail(),
            ui_guidance::search_results(),
        ]
        .into_iter()
        .flat_map(|next_step| next_step.actions)
        .map(|action| action.command)
        .chain(help_text().lines().map(str::to_owned))
        .collect::<Vec<_>>();
        assert!(
            commands.iter().all(|command| !command.contains(" N")),
            "ambiguous numeric placeholder: {commands:?}"
        );
    }

    #[test]
    fn result_percent_is_relative_and_bounded() {
        assert_eq!(relative_match_percent(0.0156, 0.0156), 100);
        assert_eq!(relative_match_percent(0.0153, 0.0156), 98);
        assert_eq!(relative_match_percent(-1.0, 0.0156), 0);
        assert_eq!(relative_match_percent(f64::NAN, 0.0156), 0);
    }

    #[test]
    fn full_trigger_keeps_every_word_in_a_single_terminal_row() {
        let trigger = full_trigger("Первый абзац.\n\nВторой абзац.");
        assert_eq!(trigger, "Первый абзац. Второй абзац.");
    }

    #[test]
    fn contour_summary_counts_types_not_roots() {
        assert_eq!(contour_summary(0, 0), "не настроены");
        assert_eq!(contour_summary(3, 0), "документация · 3 корней");
        assert_eq!(contour_summary(3, 2), "документация · код · 3 + 2 корней");
    }

    #[test]
    fn source_paths_use_platform_separators_consistently() {
        let separator = std::path::MAIN_SEPARATOR;
        assert_eq!(
            display_relative_path("Governance/01-Decisions\\Scripts"),
            format!("Governance{separator}01-Decisions{separator}Scripts")
        );
    }

    #[cfg(windows)]
    #[test]
    fn extended_windows_prefix_is_never_shown_to_the_user() {
        assert_eq!(
            display_path(Path::new(r"\\?\C:\Obsidian\Docs")),
            r"C:\Obsidian\Docs"
        );
    }
}
