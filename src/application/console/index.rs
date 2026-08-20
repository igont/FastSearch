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
    production::{IndexingProgress, IndexingStage, IndexingWorkStage},
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
    let model = runtime.embedding_model();
    let device = runtime.execution_device();
    let operation = if rebuild {
        "Перестроение индекса"
    } else {
        "Обновление индекса"
    };
    let dashboard = ProgressDashboard::new(
        operation,
        vec![ProgressTaskSpec::new(
            format!(
                "Индекс рабочей области · {} · {}",
                model.display_name(),
                device.label()
            ),
            vec![
                ProgressPhase::new("источники", ProgressUnit::count("этап", "этап/с")),
                ProgressPhase::new("корпус", ProgressUnit::count("этап", "этап/с")),
                ProgressPhase::new("лексика", ProgressUnit::count("этап", "этап/с")),
                ProgressPhase::new(
                    format!(
                        "{} · {} · векторизация",
                        model.display_name(),
                        device.label()
                    ),
                    ProgressUnit::count("записей", "зап./с"),
                ),
                ProgressPhase::new("сохранение", ProgressUnit::count("этап", "этап/с")),
            ],
        )],
    )
    .with_refresh_interval(Duration::from_secs(5));
    let runtime_for_index = &mut *runtime;
    let result = run_progress_dashboard(chat, dashboard, move |port| {
        let mut report = |progress: IndexingProgress| {
            let phase = match (progress.stage, progress.work_stage) {
                (IndexingStage::Sources, _) => 0,
                (IndexingStage::State, _) => 1,
                (IndexingStage::Lexical, _) => 2,
                (IndexingStage::Vector, Some(IndexingWorkStage::Saving)) => 4,
                (IndexingStage::Vector, _) => 3,
            };
            port.stage(0, phase, "выполняется");
            if let (Some(completed), Some(total)) = (progress.work_completed, progress.work_total) {
                port.progress(0, phase, completed, total);
            }
        };
        let result = if rebuild {
            runtime_for_index.rebuild_with_progress(&mut report)
        } else {
            runtime_for_index.index_with_progress(&mut report)
        };
        match &result {
            Ok(_) => port.complete(0, "готово"),
            Err(error) => port.fail(0, error.message()),
        }
        result
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
