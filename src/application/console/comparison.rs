use super::*;

use std::{
    sync::mpsc::{self, RecvTimeoutError},
    thread,
    time::{Duration, Instant},
};

use super::progress::{ProgressEstimator, ProgressForecast, human_duration as progress_duration};

const MODEL_PHASE_UNITS: u64 = 1_000;
const MODEL_PROGRESS_TOTAL: u64 = MODEL_PHASE_UNITS * 3;

fn model_progress(current: u64, label: &str) -> ProgressValue {
    ProgressValue::new(current, MODEL_PROGRESS_TOTAL).with_label(label)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ComparisonTaskPhase {
    Shared,
    Waiting,
    Checking,
    Downloading,
    Validating,
    Vectorizing,
    Saving,
    Finished,
}

#[derive(Clone, Debug)]
struct ComparisonResultRef {
    code: String,
    source: String,
    hit: SearchHit,
}

#[derive(Clone, Debug)]
struct ComparisonSearchSession {
    query: String,
    results: Vec<ComparisonResultRef>,
}

#[derive(Clone, Debug)]
struct ComparisonTask {
    model: Option<EmbeddingModelId>,
    label: String,
    state: TaskState,
    detail: String,
    progress: Option<ProgressValue>,
    rendered_bar_step: Option<u64>,
    phase: ComparisonTaskPhase,
    stage_started: Option<Instant>,
    last_progress_at: Option<Instant>,
    work_completed: u64,
    work_total: u64,
    activity: String,
    forecast: Option<ProgressForecast>,
    estimator: ProgressEstimator,
}

impl ComparisonTask {
    fn enter_phase(&mut self, phase: ComparisonTaskPhase, activity: impl Into<String>) {
        if self.phase != phase {
            self.phase = phase;
            self.stage_started = Some(Instant::now());
            self.last_progress_at = None;
            self.work_completed = 0;
            self.work_total = 0;
            self.forecast = None;
            self.estimator = ProgressEstimator::default();
        }
        self.activity = activity.into();
    }

    fn observe_work(&mut self, completed: u64, total: u64, download: bool) {
        let now = Instant::now();
        let started = self.stage_started.get_or_insert(now);
        if completed > self.work_completed {
            self.last_progress_at = Some(now);
        }
        self.work_completed = completed;
        self.work_total = total;
        self.forecast = self.estimator.observe(completed, total, started.elapsed());
        self.refresh_detail_at(now, download);
    }

    fn refresh_detail_at(&mut self, now: Instant, download: bool) {
        let elapsed = self.stage_started.map_or(Duration::ZERO, |started| {
            now.saturating_duration_since(started)
        });
        let silence = self
            .last_progress_at
            .map_or(elapsed, |last| now.saturating_duration_since(last));
        let silence_note = (silence >= Duration::from_secs(5))
            .then(|| format!(" · новых данных нет {}", progress_duration(silence)));
        self.detail = match self.phase {
            ComparisonTaskPhase::Downloading => {
                let transfer = if self.work_total == 0 {
                    "объём пока неизвестен".to_owned()
                } else {
                    format!(
                        "{} из {}",
                        human_bytes(self.work_completed),
                        human_bytes(self.work_total)
                    )
                };
                let forecast = self.forecast.map_or_else(
                    || format!("прошло {}", progress_duration(elapsed)),
                    |forecast| {
                        if download {
                            forecast.download_detail()
                        } else {
                            forecast.detail()
                        }
                    },
                );
                format!(
                    "{} · {transfer} · {forecast}{}",
                    self.activity,
                    silence_note.unwrap_or_default()
                )
            }
            ComparisonTaskPhase::Vectorizing => {
                let forecast = self.forecast.map_or_else(
                    || {
                        format!(
                            "оценка после первых записей · прошло {}",
                            progress_duration(elapsed)
                        )
                    },
                    ProgressForecast::detail,
                );
                format!(
                    "{} · {forecast}{}",
                    self.activity,
                    silence_note.unwrap_or_default()
                )
            }
            ComparisonTaskPhase::Checking
            | ComparisonTaskPhase::Validating
            | ComparisonTaskPhase::Saving
            | ComparisonTaskPhase::Shared => format!(
                "{} · прошло {}{}",
                self.activity,
                progress_duration(elapsed),
                silence_note.unwrap_or_default()
            ),
            ComparisonTaskPhase::Waiting | ComparisonTaskPhase::Finished => self.activity.clone(),
        };
    }
}

#[derive(Clone, Debug)]
struct ComparisonTaskBoard {
    tasks: Vec<ComparisonTask>,
}

impl ComparisonTaskBoard {
    fn new() -> Self {
        let mut tasks = vec![ComparisonTask {
            model: None,
            label: "Общий корпус и лексический индекс".to_owned(),
            state: TaskState::Running,
            detail: "подготовка".to_owned(),
            progress: None,
            rendered_bar_step: None,
            phase: ComparisonTaskPhase::Shared,
            stage_started: Some(Instant::now()),
            last_progress_at: None,
            work_completed: 0,
            work_total: 0,
            activity: "подготовка".to_owned(),
            forecast: None,
            estimator: ProgressEstimator::default(),
        }];
        tasks.extend(
            EmbeddingModelId::ALL
                .into_iter()
                .map(|model| ComparisonTask {
                    model: Some(model),
                    label: model.display_name().to_owned(),
                    state: TaskState::Pending,
                    detail: "ожидает".to_owned(),
                    progress: Some(model_progress(0, "этап 1/3 · ожидает загрузки")),
                    rendered_bar_step: None,
                    phase: ComparisonTaskPhase::Waiting,
                    stage_started: None,
                    last_progress_at: None,
                    work_completed: 0,
                    work_total: 0,
                    activity: "ожидает".to_owned(),
                    forecast: None,
                    estimator: ProgressEstimator::default(),
                }),
        );
        Self { tasks }
    }

    fn apply(&mut self, event: ComparisonUpdateProgress) -> bool {
        match event {
            ComparisonUpdateProgress::Shared { stage, .. } => {
                let task = &mut self.tasks[0];
                task.state = TaskState::Running;
                let activity = match stage {
                    ComparisonSharedStage::Sources => "чтение исходных документов и кода",
                    ComparisonSharedStage::State => "согласование общего корпуса",
                    ComparisonSharedStage::Lexical => "построение полнотекстового индекса",
                };
                task.enter_phase(ComparisonTaskPhase::Shared, activity);
                task.refresh_detail_at(Instant::now(), false);
                task.progress = None;
                true
            }
            ComparisonUpdateProgress::SharedCompleted => {
                let task = &mut self.tasks[0];
                task.state = TaskState::Completed;
                task.detail = "готово".to_owned();
                task.phase = ComparisonTaskPhase::Finished;
                task.progress = None;
                true
            }
            ComparisonUpdateProgress::SharedFailed { message } => {
                let task = &mut self.tasks[0];
                task.state = TaskState::Failed;
                task.detail = message;
                task.phase = ComparisonTaskPhase::Finished;
                task.progress = None;
                true
            }
            ComparisonUpdateProgress::Model { model, stage } => {
                let task = self
                    .tasks
                    .iter_mut()
                    .find(|task| task.model == Some(model))
                    .expect("comparison board covers the static model catalog");
                match stage {
                    ComparisonModelStage::Checking => {
                        task.state = TaskState::Running;
                        task.enter_phase(ComparisonTaskPhase::Checking, "проверка готовности");
                        task.refresh_detail_at(Instant::now(), false);
                        task.progress = Some(model_progress(0, "этап 1/3 · проверка весов"));
                        true
                    }
                    ComparisonModelStage::Downloading {
                        asset,
                        completed_bytes,
                        total_bytes,
                    } => {
                        task.state = TaskState::Running;
                        let activity = asset.map_or_else(
                            || "проверка и загрузка весов".to_owned(),
                            |asset| format!("загрузка весов: {asset}"),
                        );
                        task.enter_phase(ComparisonTaskPhase::Downloading, activity);
                        let Some((completed, total)) = completed_bytes.zip(total_bytes) else {
                            task.work_total = model_descriptor(model).approximate_download_bytes;
                            task.refresh_detail_at(Instant::now(), true);
                            task.progress = Some(model_progress(0, "этап 1/3 · загрузка весов"));
                            task.rendered_bar_step = None;
                            return true;
                        };
                        let phase_progress =
                            completed.saturating_mul(MODEL_PHASE_UNITS) / total.max(1);
                        let overall = phase_progress.min(MODEL_PHASE_UNITS);
                        let bar_step = overall.saturating_mul(32) / MODEL_PROGRESS_TOTAL;
                        task.progress = Some(model_progress(
                            overall,
                            &format!(
                                "этап 1/3 · загрузка · {}%",
                                completed.saturating_mul(100) / total.max(1)
                            ),
                        ));
                        task.observe_work(completed, total, true);
                        if task.rendered_bar_step == Some(bar_step) && completed < total {
                            false
                        } else {
                            task.rendered_bar_step = Some(bar_step);
                            true
                        }
                    }
                    ComparisonModelStage::Validating => {
                        task.state = TaskState::Running;
                        task.enter_phase(
                            ComparisonTaskPhase::Validating,
                            "проверка загруженной модели",
                        );
                        task.refresh_detail_at(Instant::now(), false);
                        task.progress = Some(model_progress(
                            MODEL_PHASE_UNITS,
                            "этап 1/3 · веса загружены · проверка runtime",
                        ));
                        task.rendered_bar_step = Some(10);
                        true
                    }
                    ComparisonModelStage::Indexing {
                        completed_records,
                        total_records,
                    } => {
                        task.state = TaskState::Running;
                        task.enter_phase(ComparisonTaskPhase::Vectorizing, "векторизация записей");
                        let vector_progress = completed_records
                            .saturating_mul(MODEL_PHASE_UNITS)
                            .checked_div(total_records)
                            .unwrap_or(0);
                        let overall = MODEL_PHASE_UNITS
                            .saturating_add(vector_progress.min(MODEL_PHASE_UNITS));
                        task.progress = Some(model_progress(
                            overall,
                            &format!(
                                "этап 2/3 · векторизация · {completed_records}/{total_records} записей"
                            ),
                        ));
                        task.observe_work(completed_records, total_records, false);
                        let bar_step = overall.saturating_mul(32) / MODEL_PROGRESS_TOTAL;
                        if task.rendered_bar_step == Some(bar_step)
                            && completed_records < total_records
                        {
                            false
                        } else {
                            task.rendered_bar_step = Some(bar_step);
                            true
                        }
                    }
                    ComparisonModelStage::Saving => {
                        task.state = TaskState::Running;
                        task.enter_phase(
                            ComparisonTaskPhase::Saving,
                            "атомарная запись модельного индекса",
                        );
                        task.refresh_detail_at(Instant::now(), false);
                        task.progress = Some(model_progress(
                            MODEL_PHASE_UNITS * 2,
                            "этап 3/3 · сохранение индекса",
                        ));
                        task.rendered_bar_step = Some(21);
                        true
                    }
                    ComparisonModelStage::Completed { reused } => {
                        task.state = TaskState::Completed;
                        task.detail = if reused {
                            "уже готово, перестроение не требуется"
                        } else {
                            "веса и индекс готовы"
                        }
                        .to_owned();
                        task.progress = Some(model_progress(MODEL_PROGRESS_TOTAL, "готово"));
                        task.rendered_bar_step = Some(32);
                        task.phase = ComparisonTaskPhase::Finished;
                        task.stage_started = None;
                        true
                    }
                    ComparisonModelStage::Failed { message } => {
                        task.state = TaskState::Failed;
                        task.detail = message;
                        task.phase = ComparisonTaskPhase::Finished;
                        task.rendered_bar_step = None;
                        true
                    }
                }
            }
        }
    }

    fn heartbeat(&mut self) -> bool {
        self.heartbeat_at(Instant::now())
    }

    fn heartbeat_at(&mut self, now: Instant) -> bool {
        for task in &mut self.tasks {
            if task.state == TaskState::Running {
                task.refresh_detail_at(now, task.phase == ComparisonTaskPhase::Downloading);
            }
        }
        true
    }

    fn finish_aborted(&mut self) {
        for task in &mut self.tasks {
            if matches!(task.state, TaskState::Pending | TaskState::Running) {
                task.state = TaskState::Skipped;
                task.detail = "не запускалось после предыдущей ошибки".to_owned();
                task.phase = ComparisonTaskPhase::Finished;
                if task.progress.is_none() {
                    task.progress = Some(model_progress(0, "не запускалось"));
                }
                task.rendered_bar_step = None;
            }
        }
    }

    fn document(&self) -> TaskListDocument {
        let finished = self
            .tasks
            .iter()
            .filter(|task| {
                matches!(
                    task.state,
                    TaskState::Completed | TaskState::Failed | TaskState::Skipped
                )
            })
            .count();
        let failures = self
            .tasks
            .iter()
            .filter(|task| task.state == TaskState::Failed)
            .count();
        let state = if finished == self.tasks.len() {
            if failures == 0 {
                ProgressState::Completed
            } else {
                ProgressState::Failed
            }
        } else {
            ProgressState::Running
        };
        self.tasks.iter().fold(
            TaskListDocument::new("Подготовка сравнения", state).with_summary(format!(
                "Завершено: {finished} из {} задач{}.",
                self.tasks.len(),
                if failures == 0 {
                    String::new()
                } else {
                    format!(" · ошибок: {failures}")
                }
            )),
            |document, task| {
                let mut item = TaskItem::new(&task.label, task.state).with_detail(&task.detail);
                if let Some(progress) = &task.progress {
                    item = item.with_progress(progress.clone());
                }
                document.with_task(item)
            },
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ComparisonTransition {
    Back,
    Exit,
}

pub(super) fn run_comparison<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    runtime: &mut ProductionRuntime,
) -> io::Result<ComparisonTransition> {
    let commands = comparison_catalog();
    let mut last_search: Option<ComparisonSearchSession> = None;
    show_comparison_readiness(chat, &ComparisonCoordinator::new(runtime).readiness())?;
    loop {
        let Some(line) = chat.read_command("compare")? else {
            return Ok(ComparisonTransition::Exit);
        };
        let (name, arguments) = match commands.resolve(&line) {
            CommandResolution::Empty => continue,
            CommandResolution::Unknown {
                suggestion: None, ..
            } if !line.trim_start().starts_with('/') => {
                ("search".to_owned(), line.trim().to_owned())
            }
            CommandResolution::Unknown { suggestion, .. } => {
                let mut error = UserErrorDocument::new("Неизвестная команда сравнения.")
                    .with_code("COMPARE_UNKNOWN_COMMAND")
                    .with_hint("Введите /help, чтобы увидеть действия режима сравнения.");
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
        };

        match name.as_str() {
            "back" => return Ok(ComparisonTransition::Back),
            "exit" => return Ok(ComparisonTransition::Exit),
            "help" => chat.show_typed(&commands.welcome_document(
                "Сравнение моделей",
                "Обычный текст выполняет один и тот же запрос всеми готовыми моделями.",
            ))?,
            "status" => {
                show_comparison_readiness(chat, &ComparisonCoordinator::new(runtime).readiness())?
            }
            "update" => {
                let prepared = PreparedAction::new(
                    (),
                    PreviewDocument::new(
                        "Подготовка сравнения",
                        "FastSearch проверит общий корпус и подготовит недостающие модельные индексы.",
                    )
                    .with_change("Готовые и актуальные модельные индексы будут сохранены без изменений.")
                    .with_change("Отсутствующие модели будут автоматически загружены в локальный кеш.")
                    .with_warning("Полный набор моделей требует несколько гигабайт диска и может индексироваться долго."),
                );
                match chat.confirm_prepared(prepared)? {
                    PreparedOutcome::Confirmed(_) => {
                        let mut board = ComparisonTaskBoard::new();
                        let mut output_error = None;
                        let result = {
                            let mut region = chat.live_region();
                            region.update_typed(&board.document())?;
                            let (sender, receiver) = mpsc::channel();
                            let runtime_for_update = &mut *runtime;
                            let result = thread::scope(|scope| {
                                let worker = scope.spawn(move || {
                                    ComparisonCoordinator::new(runtime_for_update)
                                        .update_required_with_progress(false, |event| {
                                            let _ = sender.send(event);
                                        })
                                });
                                let mut next_refresh = Instant::now() + Duration::from_secs(5);
                                loop {
                                    let wait =
                                        next_refresh.saturating_duration_since(Instant::now());
                                    match receiver.recv_timeout(wait) {
                                        Ok(event) => {
                                            if output_error.is_none() && board.apply(event) {
                                                output_error =
                                                    region.update_typed(&board.document()).err();
                                            }
                                        }
                                        Err(RecvTimeoutError::Timeout) => {
                                            if output_error.is_none() && board.heartbeat() {
                                                output_error =
                                                    region.update_typed(&board.document()).err();
                                            }
                                            next_refresh = Instant::now() + Duration::from_secs(5);
                                        }
                                        Err(RecvTimeoutError::Disconnected) => break,
                                    }
                                    if Instant::now() >= next_refresh {
                                        if output_error.is_none() && board.heartbeat() {
                                            output_error =
                                                region.update_typed(&board.document()).err();
                                        }
                                        next_refresh = Instant::now() + Duration::from_secs(5);
                                    }
                                }
                                worker.join().unwrap_or_else(|_| {
                                    Err(crate::domain::FastSearchError::new(
                                        crate::domain::ErrorKind::ProjectionFailure,
                                        "comparison preparation worker terminated unexpectedly",
                                    ))
                                })
                            });
                            if result.is_err() && output_error.is_none() {
                                board.finish_aborted();
                                output_error = region.update_typed(&board.document()).err();
                            }
                            result
                        };
                        if let Some(error) = output_error {
                            return Err(error);
                        }
                        match result {
                            Ok(outcomes) => {
                                let readiness = ComparisonCoordinator::new(runtime).readiness();
                                show_comparison_readiness(chat, &readiness)?;
                                for outcome in outcomes.iter().filter(|item| item.error().is_some())
                                {
                                    show_error(
                                        chat,
                                        "COMPARE_MODEL_UPDATE",
                                        outcome.error().unwrap_or("Неизвестная ошибка модели."),
                                        &format!(
                                            "Модель {} оставлена недоступной; остальные модели можно сравнивать.",
                                            outcome.model().display_name()
                                        ),
                                    )?;
                                }
                            }
                            Err(error) => show_error(
                                chat,
                                "COMPARE_UPDATE",
                                error.message(),
                                "Проверьте sources, подключение и свободное место, затем повторите /update.",
                            )?,
                        }
                    }
                    PreparedOutcome::Cancelled => {
                        show_notice(chat, "Подготовка сравнения отменена.")?
                    }
                    PreparedOutcome::EndOfInput => return Ok(ComparisonTransition::Exit),
                }
            }
            "open" => {
                let Some(search) = last_search.as_ref() else {
                    show_error(
                        chat,
                        "COMPARE_NO_RESULTS",
                        "Сначала выполните сравнительный запрос.",
                        "Введите запрос обычной строкой.",
                    )?;
                    continue;
                };
                let code = arguments.trim().to_ascii_uppercase();
                let Some(selected) = search.results.iter().find(|item| item.code == code) else {
                    show_error(
                        chat,
                        "COMPARE_RESULT_CODE",
                        "Результата с таким кодом нет.",
                        "Используйте код из выдачи, например /open A1 или /open L1.",
                    )?;
                    continue;
                };
                show_comparison_record(chat, runtime, search, selected)?;
            }
            "search" => {
                let query_text = arguments.trim();
                if query_text.is_empty() {
                    show_error(
                        chat,
                        "COMPARE_EMPTY_QUERY",
                        "Поисковый запрос не должен быть пустым.",
                        "Введите один запрос обычной строкой.",
                    )?;
                    continue;
                }
                let readiness = match ComparisonCoordinator::new(runtime).readiness() {
                    Ok(readiness) => readiness,
                    Err(error) => {
                        show_error(
                            chat,
                            "COMPARE_READINESS",
                            error.message(),
                            "Повторите /status или проверьте локальный кеш моделей.",
                        )?;
                        continue;
                    }
                };
                if !readiness.iter().any(ComparisonReadiness::ready) {
                    show_error(
                        chat,
                        "COMPARE_NOT_READY",
                        "Нет ни одной готовой модели для сравнения.",
                        "Используйте /update и подтвердите подготовку индексов.",
                    )?;
                    continue;
                }
                let query = match SearchQuery::new(query_text, SearchMode::Balanced) {
                    Ok(query) => query,
                    Err(error) => {
                        show_error(
                            chat,
                            "COMPARE_QUERY",
                            error.message(),
                            "Уточните текст запроса.",
                        )?;
                        continue;
                    }
                };
                chat.show_typed(&ProgressDocument::new(
                    "Сравнение моделей",
                    ProgressState::Running,
                    "FastSearch выполняет один запрос по общей лексической базе и всем готовым модельным индексам…",
                ))?;
                match ComparisonCoordinator::new(runtime).run(&query, 5) {
                    Ok(run) => {
                        last_search = Some(show_comparison_run(chat, query_text, &run)?);
                    }
                    Err(error) => show_error(
                        chat,
                        "COMPARE_SEARCH",
                        error.message(),
                        "Используйте /update, чтобы актуализировать общий и модельные индексы.",
                    )?,
                }
            }
            _ => unreachable!("static comparison command catalog"),
        }
    }
}

fn show_comparison_readiness<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    readiness: &Result<Vec<ComparisonReadiness>, crate::domain::FastSearchError>,
) -> io::Result<()> {
    let readiness = match readiness {
        Ok(readiness) => readiness,
        Err(error) => {
            return show_error(
                chat,
                "COMPARE_READINESS",
                error.message(),
                "Проверьте локальный кеш FastSearch и повторите /status.",
            );
        }
    };
    let ready = readiness.iter().filter(|item| item.ready()).count();
    let document = readiness.iter().fold(
        TableDocument::new(
            "Готовность сравнения",
            vec![
                TableColumn::new("МОДЕЛЬ"),
                TableColumn::new("ВЕСА"),
                TableColumn::new("ИНДЕКС"),
                TableColumn::new("РАЗМЕР").right_aligned(),
                TableColumn::new("ПОСТРОЕНИЕ").right_aligned(),
            ],
        )
        .with_summary(format!(
                "Готово моделей: {ready} из {} · проверка не загружает модели и не запускает индексацию.",
                readiness.len()
            )),
        |document, item| {
            let index_ready = item.index_status().freshness() == IndexFreshness::Current;
            let (size, duration) = item.index_metrics().map_or_else(
                || ("—".to_owned(), "—".to_owned()),
                |metrics| {
                    (
                        human_bytes(metrics.size_bytes()),
                        human_duration(metrics.build_duration_ms()),
                    )
                },
            );
            document.with_row(TableRow::new(vec![
                item.model().display_name().to_owned(),
                if item.weights_ready() { "ГОТОВЫ" } else { "НЕТ" }.to_owned(),
                if index_ready {
                    "ГОТОВ".to_owned()
                } else {
                    format!(
                        "НЕ ГОТОВ · {}",
                        human_freshness(item.index_status().freshness())
                    )
                },
                size,
                duration,
            ]))
        },
    );
    let next_step = if ready == readiness.len() {
        NextStep::instruction("Введите единый поисковый запрос или выберите действие:")
            .with_action(ActionItem::new("/status", "проверить готовность"))
            .with_action(ActionItem::new("/back", "вернуться в рабочую область"))
    } else if ready == 0 {
        NextStep::instruction("Сначала подготовьте модельные индексы:")
            .with_action(ActionItem::new("/update", "подготовить модельные индексы"))
            .with_action(ActionItem::new("/back", "вернуться в рабочую область"))
    } else {
        NextStep::instruction("Введите единый запрос для готовых моделей или выберите действие:")
            .with_action(ActionItem::new(
                "/update",
                "подготовить недостающие индексы",
            ))
            .with_action(ActionItem::new("/back", "вернуться в рабочую область"))
    };
    chat.show_typed(&document.with_next_step(next_step))
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

fn human_duration(milliseconds: u64) -> String {
    if milliseconds < 1_000 {
        return format!("{milliseconds} мс");
    }
    let seconds = milliseconds / 1_000;
    if seconds < 60 {
        return format!("{seconds} с");
    }
    let minutes = seconds / 60;
    let remainder = seconds % 60;
    if minutes < 60 {
        return format!("{minutes} мин {remainder} с");
    }
    format!("{} ч {} мин", minutes / 60, minutes % 60)
}

fn comparison_catalog() -> CommandCatalog {
    CommandCatalog::new(vec![
        CommandSpec::new(
            "search",
            "выполнить единый сравнительный запрос",
            "/search <запрос>",
        ),
        CommandSpec::new(
            "update",
            "загрузить и проиндексировать только недостающее",
            "/update",
        ),
        CommandSpec::new("status", "проверить готовность всех моделей", "/status"),
        CommandSpec::new("open", "открыть результат по коду", "/open <A1|B1|L1>"),
        CommandSpec::new("back", "вернуться в рабочую область", "/back"),
        CommandSpec::new("help", "показать команды сравнения", "/help")
            .with_alias("--help")
            .with_alias("-h"),
        CommandSpec::new("exit", "закрыть FastSearch", "/exit")
            .with_alias("quit")
            .with_alias("выход"),
    ])
    .expect("comparison command catalog is static and valid")
}

fn show_comparison_run<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    query: &str,
    run: &ComparisonRun,
) -> io::Result<ComparisonSearchSession> {
    let mut results = Vec::new();
    let lexical = run.lexical_hits().iter().enumerate().fold(
        ResultDocument::new(
            "Лексическая база",
            format!("Запрос: {query} · общий контрольный результат."),
        ),
        |document, (index, hit)| {
            let code = format!("L{}", index + 1);
            results.push(ComparisonResultRef {
                code: code.clone(),
                source: "Лексическая база".to_owned(),
                hit: hit.clone(),
            });
            document.with_item(comparison_result_item(&code, hit))
        },
    );
    chat.show_typed(&lexical)?;

    for (model_index, model) in run.models().iter().enumerate() {
        let prefix = char::from(b'A' + u8::try_from(model_index).unwrap_or(0));
        if let Some(error) = model.error() {
            chat.show_typed(
                &ReportDocument::new().with_section(
                    ReportSection::new(model.model().display_name())
                        .with_line("Статус: НЕДОСТУПНА ДЛЯ ЭТОГО ЗАПРОСА")
                        .with_line(format!("Причина: {error}")),
                ),
            )?;
            continue;
        }
        let document = model.hits().iter().enumerate().fold(
            ResultDocument::new(
                model.model().display_name(),
                format!(
                    "Векторный поиск · {} мс · оценки сопоставимы только внутри этого блока.",
                    model.latency_ms()
                ),
            ),
            |document, (index, hit)| {
                let code = format!("{prefix}{}", index + 1);
                results.push(ComparisonResultRef {
                    code: code.clone(),
                    source: model.model().display_name().to_owned(),
                    hit: hit.clone(),
                });
                document.with_item(comparison_result_item(&code, hit))
            },
        );
        chat.show_typed(&document)?;
    }
    chat.show_typed(
        &NoticeDocument::new("Сравнение завершено.").with_next_step(
            NextStep::instruction("Введите следующий единый запрос или выберите действие:")
                .with_action(ActionItem::new("/open A1", "открыть результат"))
                .with_action(ActionItem::new("/back", "вернуться в рабочую область")),
        ),
    )?;
    Ok(ComparisonSearchSession {
        query: query.to_owned(),
        results,
    })
}

fn comparison_result_item(code: &str, hit: &SearchHit) -> ResultItem {
    ResultItem::new(
        format!(
            "[{code}] [{}] {}",
            record_label(hit.record().kind()),
            hit.record().title()
        ),
        hit.record().locator().path(),
    )
    .with_excerpt(result_excerpt(hit.record().searchable_content()))
}

fn show_comparison_record<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    runtime: &ProductionRuntime,
    search: &ComparisonSearchSession,
    selected: &ComparisonResultRef,
) -> io::Result<()> {
    match runtime.get(selected.hit.record().id()) {
        Ok(Some(record)) => chat.show_typed(
            &ReportDocument::new()
                .with_section(
                    ReportSection::new(format!("Результат {}", selected.code))
                        .with_line(format!("Источник сравнения: {}", selected.source))
                        .with_line(format!("Заголовок: {}", record.title()))
                        .with_line(format!("Тип: {}", record_label(record.kind())))
                        .with_line(format!("Файл: {}", record.locator().path())),
                )
                .with_section(
                    ReportSection::new("Контекст сравнения")
                        .with_line(format!("Запрос: {}", search.query))
                        .with_line(format!("Канал: {:?}", selected.hit.channel()))
                        .with_line(format!("Оценка внутри блока: {:.4}", selected.hit.score())),
                )
                .with_section(
                    ReportSection::new("Содержимое").with_line(record.searchable_content()),
                )
                .with_next_step(
                    NextStep::instruction("Введите новый запрос или выберите действие:")
                        .with_action(ActionItem::new("/status", "проверить готовность"))
                        .with_action(ActionItem::new("/back", "вернуться в рабочую область")),
                ),
        ),
        Ok(None) => show_error(
            chat,
            "COMPARE_RESULT_MISSING",
            "Запись больше не находится в общем индексе.",
            "Используйте /update и повторите сравнительный запрос.",
        ),
        Err(error) => show_error(
            chat,
            "COMPARE_RESULT_OPEN",
            error.message(),
            "Используйте /update и повторите сравнительный запрос.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use terminal_dialogue::{LanguagePack, ProgressState, TerminalDocument};

    use super::{
        ComparisonModelStage, ComparisonTaskBoard, ComparisonUpdateProgress, EmbeddingModelId,
        human_bytes, human_duration,
    };

    #[test]
    fn partition_measurements_are_compact_and_unambiguous() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(10 * 1024 * 1024), "10.0 MiB");
        assert_eq!(human_duration(850), "850 мс");
        assert_eq!(human_duration(78_000), "1 мин 18 с");
        assert_eq!(human_duration(7_260_000), "2 ч 1 мин");
    }

    #[test]
    fn comparison_board_tracks_every_task_in_one_three_phase_bar() {
        let model = EmbeddingModelId::MultilingualE5Small;
        let mut board = ComparisonTaskBoard::new();
        assert!(board.apply(ComparisonUpdateProgress::SharedCompleted));
        assert!(board.apply(ComparisonUpdateProgress::Model {
            model,
            stage: ComparisonModelStage::Downloading {
                asset: Some("model.onnx".to_owned()),
                completed_bytes: Some(50),
                total_bytes: Some(100),
            },
        }));

        let rendered = board
            .document()
            .to_dialogue_document(&LanguagePack::russian())
            .render(false);
        assert!(
            rendered.contains("✓ Общий корпус и лексический индекс"),
            "{rendered}"
        );
        assert!(
            rendered.contains("▶ E5 Small — загрузка весов: model.onnx"),
            "{rendered}"
        );
        assert!(rendered.contains("этап 1/3 · загрузка · 50%"), "{rendered}");
        assert!(!rendered.contains('░'), "{rendered}");

        assert!(board.apply(ComparisonUpdateProgress::Model {
            model,
            stage: ComparisonModelStage::Failed {
                message: "ошибка модели".to_owned(),
            },
        }));
        board.finish_aborted();
        assert_eq!(board.document().state, ProgressState::Failed);
    }

    #[test]
    fn model_bar_advances_through_download_vectorization_save_and_ready() {
        let model = EmbeddingModelId::MultilingualE5Small;
        let mut board = ComparisonTaskBoard::new();

        assert!(board.apply(ComparisonUpdateProgress::Model {
            model,
            stage: ComparisonModelStage::Downloading {
                asset: Some("model.onnx".to_owned()),
                completed_bytes: Some(50),
                total_bytes: Some(100),
            },
        }));
        assert_eq!(board.tasks[1].progress.as_ref().unwrap().current, 500);

        assert!(board.apply(ComparisonUpdateProgress::Model {
            model,
            stage: ComparisonModelStage::Indexing {
                completed_records: 5,
                total_records: 10,
            },
        }));
        assert_eq!(board.tasks[1].progress.as_ref().unwrap().current, 1_500);

        assert!(board.apply(ComparisonUpdateProgress::Model {
            model,
            stage: ComparisonModelStage::Saving,
        }));
        assert_eq!(board.tasks[1].progress.as_ref().unwrap().current, 2_000);

        assert!(board.apply(ComparisonUpdateProgress::Model {
            model,
            stage: ComparisonModelStage::Completed { reused: false },
        }));
        let task = &board.tasks[1];
        assert_eq!(task.state, terminal_dialogue::TaskState::Completed);
        assert_eq!(task.progress.as_ref().unwrap().current, 3_000);
        assert_eq!(
            task.progress.as_ref().unwrap().label.as_deref(),
            Some("готово")
        );
    }

    #[test]
    fn heartbeat_explains_zero_percent_and_time_without_new_bytes() {
        let model = EmbeddingModelId::MultilingualE5Large;
        let mut board = ComparisonTaskBoard::new();
        assert!(board.apply(ComparisonUpdateProgress::Model {
            model,
            stage: ComparisonModelStage::Downloading {
                asset: Some("config.json".to_owned()),
                completed_bytes: Some(0),
                total_bytes: Some(2_253_012_762),
            },
        }));
        let now = Instant::now();
        let task = board
            .tasks
            .iter_mut()
            .find(|task| task.model == Some(model))
            .unwrap();
        task.stage_started = Some(now - Duration::from_secs(10));
        task.last_progress_at = None;

        board.heartbeat_at(now);

        let task = board
            .tasks
            .iter()
            .find(|task| task.model == Some(model))
            .unwrap();
        assert!(task.detail.contains("прошло 10 сек"), "{}", task.detail);
        assert!(
            task.detail.contains("новых данных нет 10 сек"),
            "{}",
            task.detail
        );
    }
}
