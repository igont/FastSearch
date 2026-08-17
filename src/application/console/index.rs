use std::{
    io::{self, BufRead},
    time::Duration,
};

use terminal_dialogue::{
    ChatSession, ProgressDashboard, ProgressPhase, ProgressTaskSpec, ProgressUnit,
    run_progress_dashboard,
};

use super::super::{
    ProductionRuntime,
    production::{IndexingProgress, IndexingStage, IndexingWorkStage},
};
use super::{show_error, show_no_sources};

pub(super) fn run_index<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    runtime: Option<&mut ProductionRuntime>,
    rebuild: bool,
) -> io::Result<()> {
    let Some(runtime) = runtime else {
        return show_no_sources(chat);
    };
    let operation = if rebuild {
        "Перестроение индекса"
    } else {
        "Обновление индекса"
    };
    let dashboard = ProgressDashboard::new(
        operation,
        vec![ProgressTaskSpec::new(
            "Индекс рабочей области",
            vec![
                ProgressPhase::new("источники", ProgressUnit::count("этап", "этап/с")),
                ProgressPhase::new("корпус", ProgressUnit::count("этап", "этап/с")),
                ProgressPhase::new("лексика", ProgressUnit::count("этап", "этап/с")),
                ProgressPhase::new("векторизация", ProgressUnit::count("записей", "зап./с")),
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
        Ok(_) => Ok(()),
        Err(error) => show_error(
            chat,
            "INDEX_FAILED",
            error.message(),
            "Проверьте sources и повторите операцию.",
        ),
    }
}
