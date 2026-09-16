use std::{
    io::{self, BufRead},
    time::Duration,
};

use terminal_dialogue::{
    ChatSession, NoticeDocument, ProgressDashboard, ProgressPhase, ProgressTaskSpec, ProgressUnit,
    run_progress_dashboard,
};

use super::super::{
    ProductionRuntime,
    production::{IndexingProgress, IndexingStage},
};
use super::{show_error, show_no_sources, ui_guidance};

pub(super) fn run_index_inspect<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    runtime: Option<&ProductionRuntime>,
    arguments: &str,
) -> io::Result<()> {
    let Some(runtime) = runtime else {
        return show_no_sources(chat);
    };
    let output = if arguments.trim().is_empty() {
        None
    } else {
        let value = arguments.trim().trim_matches('"');
        if matches!(value, "current" | "preview") {
            return show_error(
                chat,
                "INDEX_INSPECT_ARGUMENT",
                "Режимы current и preview больше не используются.",
                "Запустите /index inspect без режима.",
            );
        }
        Some(std::path::PathBuf::from(value))
    };
    match runtime.inspect_chunks(output.as_deref()) {
        Ok(report) => chat.show_typed(
            &NoticeDocument::new(format!(
                "Выгрузка создана: {}. Файлов включено: {}, исключено: {}, чанков: {}.",
                report.display_inputs_path(),
                report.included_files(),
                report.excluded_files(),
                report.chunks()
            ))
            .with_next_step(ui_guidance::index_inspection()),
        ),
        Err(error) => show_error(
            chat,
            "INDEX_INSPECT_FAILED",
            error.message(),
            "Проверьте путь и наличие опубликованного индекса.",
        ),
    }
}

pub(super) fn run_index<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    runtime: Option<&mut ProductionRuntime>,
    rebuild: bool,
) -> io::Result<()> {
    let Some(runtime) = runtime else {
        return show_no_sources(chat);
    };
    if let Err(error) = super::super::search_ui::ensure_search_models_ready() {
        return show_error(
            chat,
            "MODELS_NOT_READY",
            error.message(),
            "Выполните fastsearch models prepare и повторите /index update.",
        );
    }
    let operation = if rebuild {
        "Перестроение индекса"
    } else {
        "Обновление индекса"
    };
    let mut tasks = vec![ProgressTaskSpec::new(
        "Корпус и полнотекстовый индекс",
        vec![
            ProgressPhase::new("источники", ProgressUnit::count("этап", "этап/с")),
            ProgressPhase::new("корпус", ProgressUnit::count("этап", "этап/с")),
            ProgressPhase::new("лексика", ProgressUnit::count("этап", "этап/с")),
        ],
    )];
    tasks.extend(super::super::thin_search::MODELS.map(|model| {
        ProgressTaskSpec::new(
            model.display_name(),
            vec![
                ProgressPhase::new("векторизация", ProgressUnit::count("фрагментов", "фр./с")),
                ProgressPhase::new("сохранение", ProgressUnit::count("этап", "этап/с")),
            ],
        )
    }));
    let dashboard =
        ProgressDashboard::new(operation, tasks).with_refresh_interval(Duration::from_secs(5));
    let runtime_for_index = &mut *runtime;
    let result = run_progress_dashboard(chat, dashboard, move |port| {
        let mut source_summary = "источники и текстовый индекс готовы".to_owned();
        let mut report = |progress: IndexingProgress| {
            let phase = match (progress.stage, progress.work_stage) {
                (IndexingStage::Sources, _) => 0,
                (IndexingStage::State, _) => 1,
                (IndexingStage::Lexical, _) => 2,
                (IndexingStage::Vector, _) => 2,
            };
            if let Some(counts) = progress.sources {
                source_summary = format!(
                    "Файлов: {} · новых: {} · изменено: {} · без изменений: {} · удалено: {}",
                    counts.total, counts.added, counts.changed, counts.unchanged, counts.deleted
                );
                port.stage(0, phase, &source_summary);
            } else {
                port.stage(0, phase, "выполняется");
            }
            if let (Some(completed), Some(total)) = (progress.work_completed, progress.work_total) {
                port.progress(0, phase, completed, total);
            }
        };
        let result = if rebuild {
            runtime_for_index.rebuild_with_progress(&mut report)
        } else {
            runtime_for_index.index_with_progress(&mut report)
        };
        if let Err(error) = result {
            port.fail(0, error.message());
            return Err(super::super::PublicSearchError::not_ready(error.message()));
        }
        port.complete(0, &source_summary);
        for (index, model) in super::super::thin_search::MODELS.into_iter().enumerate() {
            let task = index + 1;
            if runtime_for_index.model_partition_status(model).freshness()
                == crate::domain::IndexFreshness::Current
            {
                port.complete(task, "индекс актуален; повторная векторизация не нужна");
                continue;
            }
            let role = crate::domain::ProductionModelRole::ALL
                .into_iter()
                .find(|role| role.embedding_model() == Some(model))
                .ok_or_else(|| {
                    super::super::PublicSearchError::not_ready("missing production embedding role")
                })?;
            let root = super::super::model_provisioning::production_role_cache_root(role)
                .map_err(|error| super::super::PublicSearchError::not_ready(error.message()))?;
            let result =
                runtime_for_index.build_model_partition_with_progress(model, &root, |event| {
                    match event {
                        crate::adapters::vector::VectorBuildProgress::Embedding {
                            completed_records,
                            total_records,
                        } => {
                            port.stage(task, 0, "пересчёт фрагментов");
                            port.progress(task, 0, completed_records, total_records);
                        }
                        crate::adapters::vector::VectorBuildProgress::Saving => {
                            port.stage(task, 1, "сохранение")
                        }
                    }
                });
            match result {
                Ok(status) if status.freshness() == crate::domain::IndexFreshness::Current => {
                    port.complete(task, "готово")
                }
                Ok(_) => {
                    port.fail(task, "индекс не стал актуальным");
                    return Err(super::super::PublicSearchError::not_ready(
                        "projection did not become current",
                    ));
                }
                Err(error) => {
                    port.fail(task, error.message());
                    return Err(super::super::PublicSearchError::not_ready(error.message()));
                }
            }
        }
        Ok(())
    })?;
    match result {
        Ok(_) => super::show_index_status(chat, Some(runtime)),
        Err(error) => show_error(
            chat,
            "INDEX_FAILED",
            error.message(),
            "Проверьте sources и повторите операцию.",
        ),
    }
}
