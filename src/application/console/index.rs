use std::{
    io::{self, BufRead},
    sync::mpsc::{self, RecvTimeoutError},
    thread,
    time::{Duration, Instant},
};

use terminal_dialogue::{ChatSession, ProgressDocument, ProgressState, ProgressValue};

use super::super::{
    ProductionRuntime,
    production::{IndexingProgress, IndexingStage, IndexingWorkStage},
};
use super::progress::{ProgressEstimator, ProgressForecast, human_duration as progress_duration};
use super::{human_freshness, show_error, show_no_sources};

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
    let mut output_error = None;
    let mut vector_started = None;
    let mut vector_estimator = ProgressEstimator::default();
    let mut vector_forecast = None;
    let mut current_progress = None;
    let mut current_stage = None;
    let mut stage_started = Instant::now();
    let mut last_event_at = Instant::now();
    let result = {
        let mut region = chat.live_region();
        let (sender, receiver) = mpsc::channel();
        let runtime_for_index = &mut *runtime;
        let result = thread::scope(|scope| {
            let worker = scope.spawn(move || {
                let mut report = |progress| {
                    let _ = sender.send(progress);
                };
                if rebuild {
                    runtime_for_index.rebuild_with_progress(&mut report)
                } else {
                    runtime_for_index.index_with_progress(&mut report)
                }
            });
            loop {
                match receiver.recv_timeout(Duration::from_secs(5)) {
                    Ok(progress) => {
                        let now = Instant::now();
                        if current_stage != Some((progress.stage, progress.work_stage)) {
                            current_stage = Some((progress.stage, progress.work_stage));
                            stage_started = now;
                        }
                        if let (
                            Some(IndexingWorkStage::Vectorizing),
                            Some(completed),
                            Some(total),
                        ) = (
                            progress.work_stage,
                            progress.work_completed,
                            progress.work_total,
                        ) {
                            let started = vector_started.get_or_insert(now);
                            vector_forecast =
                                vector_estimator.observe(completed, total, started.elapsed());
                        }
                        current_progress = Some(progress);
                        last_event_at = now;
                        if output_error.is_none() {
                            output_error = region
                                .update_typed(&indexing_progress_document(
                                    operation,
                                    progress,
                                    vector_forecast,
                                    now.saturating_duration_since(stage_started),
                                    Duration::ZERO,
                                ))
                                .err();
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        if let Some(progress) = current_progress {
                            let now = Instant::now();
                            if output_error.is_none() {
                                output_error = region
                                    .update_typed(&indexing_progress_document(
                                        operation,
                                        progress,
                                        vector_forecast,
                                        now.saturating_duration_since(stage_started),
                                        now.saturating_duration_since(last_event_at),
                                    ))
                                    .err();
                            }
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
            worker.join().unwrap_or_else(|_| {
                Err(crate::domain::FastSearchError::new(
                    crate::domain::ErrorKind::ProjectionFailure,
                    "indexing worker terminated unexpectedly",
                ))
            })
        });
        if output_error.is_none() {
            let final_document = match &result {
                Ok(status) => ProgressDocument::new(
                    operation,
                    ProgressState::Completed,
                    "Операция завершена.",
                )
                .with_detail(format!("Индекс: {}.", human_freshness(status.freshness()))),
                Err(error) => {
                    ProgressDocument::new(operation, ProgressState::Failed, error.message())
                }
            };
            output_error = region.update_typed(&final_document).err();
        }
        result
    };
    if let Some(error) = output_error {
        return Err(error);
    }
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

pub(super) fn indexing_progress_document(
    operation: &str,
    progress: IndexingProgress,
    forecast: Option<ProgressForecast>,
    stage_elapsed: Duration,
    silence: Duration,
) -> ProgressDocument {
    let stage = match progress.stage {
        IndexingStage::Sources => "FastSearch читает исходные документы и код…",
        IndexingStage::State => "FastSearch применяет изменения корпуса…",
        IndexingStage::Lexical => "FastSearch строит полнотекстовый индекс…",
        IndexingStage::Vector if progress.work_stage == Some(IndexingWorkStage::Saving) => {
            "FastSearch сохраняет векторный индекс…"
        }
        IndexingStage::Vector => "FastSearch векторизует записи…",
    };
    let silence_note = if silence >= Duration::from_secs(5) {
        format!(" Статус не менялся {}.", progress_duration(silence))
    } else {
        String::new()
    };
    let mut document = ProgressDocument::new(operation, ProgressState::Running, stage);
    if let (Some(IndexingWorkStage::Vectorizing), Some(completed), Some(total)) = (
        progress.work_stage,
        progress.work_completed,
        progress.work_total,
    ) {
        if total > 0 {
            document = document
                .with_progress(ProgressValue::new(completed, total).with_unit("записей"))
                .with_detail(format!(
                    "{}{}",
                    forecast.map_or_else(
                        || format!(
                            "Оценка времени появится после первых записей · прошло {}.",
                            progress_duration(stage_elapsed)
                        ),
                        ProgressForecast::detail,
                    ),
                    silence_note
                ));
        } else {
            document = document.with_detail("В корпусе нет записей для векторизации.");
        }
    } else {
        document = document.with_detail(format!(
            "Этап выполняется {}.{}",
            progress_duration(stage_elapsed),
            silence_note
        ));
    }
    document
}
