use super::*;
use terminal_dialogue::{CommandCatalog, CommandSpec};

use std::time::Duration;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ComparisonTransition {
    Back,
    Exit,
}

fn comparison_progress_dashboard() -> ProgressDashboard {
    let shared = ProgressTaskSpec::new(
        "Общий корпус и лексический индекс",
        vec![ProgressPhase::new(
            "подготовка",
            ProgressUnit::count("этапов", "этапов/с"),
        )],
    )
    .without_bar();
    let model_tasks = EmbeddingModelId::ALL.into_iter().map(|model| {
        ProgressTaskSpec::new(
            model.display_name(),
            vec![
                ProgressPhase::new("загрузка", ProgressUnit::bytes()),
                ProgressPhase::new("индекс", ProgressUnit::count("записей", "зап./с")),
                ProgressPhase::new("сохранение", ProgressUnit::count("этап", "этап/с")),
            ],
        )
    });
    ProgressDashboard::new(
        "Подготовка сравнения",
        std::iter::once(shared).chain(model_tasks).collect(),
    )
    .with_refresh_interval(Duration::from_secs(5))
}

fn report_comparison_progress(port: &ProgressPort, event: ComparisonUpdateProgress) {
    match event {
        ComparisonUpdateProgress::Shared { stage, .. } => {
            let activity = match stage {
                ComparisonSharedStage::Sources => "чтение исходных файлов",
                ComparisonSharedStage::State => "обновление корпуса",
                ComparisonSharedStage::Lexical => "лексический индекс",
            };
            port.stage(0, 0, activity);
        }
        ComparisonUpdateProgress::SharedCompleted => port.complete(0, "готово"),
        ComparisonUpdateProgress::SharedFailed { message } => port.fail(0, message),
        ComparisonUpdateProgress::Model { model, stage } => {
            let task = EmbeddingModelId::ALL
                .iter()
                .position(|candidate| *candidate == model)
                .expect("comparison progress uses the static model catalog")
                + 1;
            match stage {
                ComparisonModelStage::Checking => port.stage(task, 0, "проверка"),
                ComparisonModelStage::Downloading {
                    asset,
                    completed_bytes,
                    total_bytes,
                } => {
                    port.stage(task, 0, asset.unwrap_or_else(|| "соединение".to_owned()));
                    if let Some((completed, total)) = completed_bytes.zip(total_bytes) {
                        port.progress(task, 0, completed, total);
                    }
                }
                ComparisonModelStage::Validating => {
                    port.stage(task, 0, "проверка runtime");
                    port.progress(task, 0, 1, 1);
                }
                ComparisonModelStage::Indexing {
                    completed_records,
                    total_records,
                } => {
                    port.stage(task, 1, "векторизация");
                    port.progress(task, 1, completed_records, total_records);
                }
                ComparisonModelStage::Saving => port.stage(task, 2, "сохранение"),
                ComparisonModelStage::Completed { reused } => port.complete(
                    task,
                    if reused {
                        "уже готово"
                    } else {
                        "готово"
                    },
                ),
                ComparisonModelStage::Failed { message } => port.fail(task, message),
            }
        }
    }
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
            CommandResolution::Unknown { .. } if !line.trim_start().starts_with('/') => {
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
                let runtime_for_update = &mut *runtime;
                let result =
                    run_progress_dashboard(chat, comparison_progress_dashboard(), move |port| {
                        ComparisonCoordinator::new(runtime_for_update)
                            .update_required_with_progress(false, |event| {
                                report_comparison_progress(&port, event);
                            })
                    })?;
                match result {
                    Ok(outcomes) => {
                        let readiness = ComparisonCoordinator::new(runtime).readiness();
                        show_comparison_readiness(chat, &readiness)?;
                        for outcome in outcomes.iter().filter(|item| item.error().is_some()) {
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
                        "Используйте /update для подготовки индексов.",
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
    let lexical_best_score = run.lexical_hits().first().map_or(0.0, SearchHit::score);
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
            document.with_item(comparison_result_item(&code, hit, lexical_best_score))
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
        let best_score = model.hits().first().map_or(0.0, SearchHit::score);
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
                document.with_item(comparison_result_item(&code, hit, best_score))
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

fn comparison_result_item(code: &str, hit: &SearchHit, best_score: f64) -> ResultItem {
    ResultItem::new(
        format!(
            "[{code}] [{}] {}",
            record_label(hit.record().kind()),
            hit.record().title()
        ),
        hit.record().locator().path(),
    )
    .with_excerpt(full_trigger(hit.record().searchable_content()))
    .with_match_percent(relative_match_percent(hit.score(), best_score))
    .with_result_code(code)
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
    use std::io::Cursor;

    use terminal_dialogue::{ChatSession, SessionConfig, run_progress_dashboard};

    use super::{
        ComparisonModelStage, ComparisonUpdateProgress, EmbeddingModelId,
        comparison_progress_dashboard, human_bytes, human_duration, report_comparison_progress,
    };

    fn separator() -> String {
        String::new()
    }

    #[test]
    fn partition_measurements_are_compact_and_unambiguous() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(10 * 1024 * 1024), "10.0 MiB");
        assert_eq!(human_duration(850), "850 мс");
        assert_eq!(human_duration(78_000), "1 мин 18 с");
        assert_eq!(human_duration(7_260_000), "2 ч 1 мин");
    }

    #[test]
    fn comparison_events_flow_through_the_library_progress_port() {
        let mut input = Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();
        let config = SessionConfig::for_terminal(false, false);
        let mut chat = ChatSession::configured(&mut input, &mut output, config, separator);
        let model = EmbeddingModelId::MultilingualE5Small;

        let result = run_progress_dashboard(&mut chat, comparison_progress_dashboard(), |port| {
            report_comparison_progress(&port, ComparisonUpdateProgress::SharedCompleted);
            report_comparison_progress(
                &port,
                ComparisonUpdateProgress::Model {
                    model,
                    stage: ComparisonModelStage::Downloading {
                        asset: Some("model.onnx".to_owned()),
                        completed_bytes: Some(50),
                        total_bytes: Some(100),
                    },
                },
            );
            Ok::<_, ()>(())
        })
        .unwrap();

        assert_eq!(result, Ok(()));
        let transcript = String::from_utf8(output).unwrap();
        assert!(transcript.contains("▶ E5 Small"), "{transcript}");
        assert!(transcript.contains("1/3  загрузка  50%"), "{transcript}");
        assert!(transcript.contains("50 Б / 100 Б"), "{transcript}");
        assert!(transcript.contains("\n\n  ○ E5 Base"), "{transcript}");
    }

    #[test]
    fn downloaded_model_enters_the_single_indexing_lane_immediately() {
        let mut input = Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();
        let config = SessionConfig::for_terminal(false, false);
        let mut chat = ChatSession::configured(&mut input, &mut output, config, separator);
        let model = EmbeddingModelId::Qwen3Embedding06B;

        let result = run_progress_dashboard(&mut chat, comparison_progress_dashboard(), |port| {
            report_comparison_progress(
                &port,
                ComparisonUpdateProgress::Model {
                    model,
                    stage: ComparisonModelStage::Downloading {
                        asset: Some("model.safetensors".to_owned()),
                        completed_bytes: Some(100),
                        total_bytes: Some(100),
                    },
                },
            );
            report_comparison_progress(
                &port,
                ComparisonUpdateProgress::Model {
                    model,
                    stage: ComparisonModelStage::Indexing {
                        completed_records: 0,
                        total_records: 0,
                    },
                },
            );
            Ok::<_, ()>(())
        })
        .unwrap();

        assert_eq!(result, Ok(()));
        let transcript = String::from_utf8(output).unwrap();
        assert!(transcript.contains("2/3"), "{transcript}");
        assert!(transcript.contains("векторизация"), "{transcript}");
        assert!(transcript.contains("0%"), "{transcript}");
    }
}
