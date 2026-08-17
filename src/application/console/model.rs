use std::{
    io::{self, BufRead},
    sync::mpsc::{self, RecvTimeoutError},
    thread,
    time::{Duration, Instant},
};

use terminal_dialogue::{
    ActionItem, ChatSession, ProgressDocument, ProgressState, ProgressValue, UserErrorDocument,
};

use crate::domain::EmbeddingModelId;

use super::super::{
    EmbeddingModelAvailability, embedding_model_cache_status, ensure_embedding_model_with_progress,
    model_descriptor,
};
use super::progress::{ProgressEstimator, ProgressForecast, human_duration};

pub(super) fn provision_model_with_ui<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    selected: EmbeddingModelId,
) -> io::Result<Option<EmbeddingModelAvailability>> {
    let cache_was_ready = embedding_model_cache_status(selected).is_ok_and(|status| status.ready());
    let mut preparation = ProgressDocument::new(
        "Подготовка векторной модели",
        ProgressState::Running,
        format!(
            "FastSearch проверяет {} и при необходимости загружает её…",
            selected.display_name()
        ),
    );
    if !cache_was_ready {
        let total_mib = model_descriptor(selected)
            .approximate_download_bytes
            .div_ceil(1024 * 1024);
        preparation =
            preparation.with_progress(ProgressValue::new(0, total_mib.max(1)).with_unit("МБ"));
    }
    let mut progress_error = None;
    let provisioned = {
        let mut region = chat.live_region();
        region.update_typed(&preparation)?;
        let (sender, receiver) = mpsc::channel();
        let started = Instant::now();
        let mut last_progress_at = None;
        let mut completed = 0;
        let mut total = model_descriptor(selected).approximate_download_bytes.max(1);
        let mut asset = "установка соединения".to_owned();
        let mut estimator = ProgressEstimator::default();
        let mut forecast = None;
        let provisioned = thread::scope(|scope| {
            let worker = scope.spawn(move || {
                ensure_embedding_model_with_progress(selected, false, |event| {
                    let _ = sender.send(event);
                })
            });
            let mut next_refresh = Instant::now() + Duration::from_secs(5);
            loop {
                let wait = next_refresh.saturating_duration_since(Instant::now());
                match receiver.recv_timeout(wait) {
                    Ok(event) => {
                        if event.completed_bytes() > completed {
                            last_progress_at = Some(Instant::now());
                        }
                        completed = event.completed_bytes();
                        total = event.total_bytes().max(1);
                        asset = event.asset().to_owned();
                        forecast = estimator.observe(completed, total, started.elapsed());
                        let now = Instant::now();
                        if progress_error.is_none() && (now >= next_refresh || completed == total) {
                            progress_error = region
                                .update_typed(&download_document(
                                    &asset,
                                    completed,
                                    total,
                                    started,
                                    last_progress_at,
                                    forecast,
                                ))
                                .err();
                        }
                        if now >= next_refresh {
                            next_refresh = now + Duration::from_secs(5);
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        if progress_error.is_none() {
                            progress_error = region
                                .update_typed(&download_document(
                                    &asset,
                                    completed,
                                    total,
                                    started,
                                    last_progress_at,
                                    forecast,
                                ))
                                .err();
                        }
                        next_refresh = Instant::now() + Duration::from_secs(5);
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
            worker.join().unwrap_or_else(|_| {
                Err(crate::domain::FastSearchError::new(
                    crate::domain::ErrorKind::ProjectionFailure,
                    "model preparation worker terminated unexpectedly",
                ))
            })
        });
        if progress_error.is_none() {
            let final_document = match &provisioned {
                Ok(model) => ProgressDocument::new(
                    "Подготовка векторной модели",
                    ProgressState::Completed,
                    if model.downloaded() {
                        "Модель загружена, проверена и готова к использованию."
                    } else {
                        "Проверенная модель уже находится в локальном кеше."
                    },
                )
                .with_detail(format!(
                    "Источник: {} · ревизия: {}.",
                    model_descriptor(selected).repository,
                    model_descriptor(selected).revision
                )),
                Err(error) => ProgressDocument::new(
                    "Подготовка векторной модели",
                    ProgressState::Failed,
                    error.message(),
                ),
            };
            progress_error = region.update_typed(&final_document).err();
        }
        provisioned
    };
    if let Some(error) = progress_error {
        return Err(error);
    }
    match provisioned {
        Ok(model) => Ok(Some(model)),
        Err(error) => {
            chat.show_typed(
                &UserErrorDocument::new(error.message())
                    .with_code("MODEL_PROVISION")
                    .with_hint(
                        "FastSearch повторит загрузку автоматически; прежняя активная модель остаётся выбранной.",
                    )
                    .with_action(ActionItem::new("/model", "вернуться к каталогу")),
            )?;
            Ok(None)
        }
    }
}

fn download_document(
    asset: &str,
    completed: u64,
    total: u64,
    started: Instant,
    last_progress_at: Option<Instant>,
    forecast: Option<ProgressForecast>,
) -> ProgressDocument {
    let now = Instant::now();
    let elapsed = now.saturating_duration_since(started);
    let silence = last_progress_at.map_or(elapsed, |last| now.saturating_duration_since(last));
    let total_mib = total.div_ceil(1024 * 1024).max(1);
    let completed_mib = completed.div_ceil(1024 * 1024).min(total_mib);
    let detail = forecast.map_or_else(
        || {
            format!(
                "Данных пока недостаточно для ETA · прошло {}.",
                human_duration(elapsed)
            )
        },
        ProgressForecast::download_detail,
    );
    let silence = if silence >= Duration::from_secs(5) {
        format!(" Новых данных нет {}.", human_duration(silence))
    } else {
        String::new()
    };
    ProgressDocument::new(
        "Загрузка векторной модели",
        ProgressState::Running,
        format!("Файл: {asset}"),
    )
    .with_progress(ProgressValue::new(completed_mib, total_mib).with_unit("МБ"))
    .with_detail(format!("{detail}{silence}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_heartbeat_reports_elapsed_time_before_first_bytes() {
        let now = Instant::now();
        let document = download_document(
            "config.json",
            0,
            2_253_012_762,
            now - Duration::from_secs(10),
            None,
            None,
        );
        let detail = document.detail.unwrap();
        assert!(detail.contains("прошло "), "{detail}");
        assert!(detail.contains("Новых данных нет "), "{detail}");
        assert!(detail.contains("сек"), "{detail}");
    }
}
