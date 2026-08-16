use std::io::{self, BufRead};

use terminal_dialogue::{
    ActionItem, ChatSession, ProgressDocument, ProgressState, ProgressValue, UserErrorDocument,
};

use crate::domain::EmbeddingModelId;

use super::super::{
    EmbeddingModelAvailability, embedding_model_cache_status, ensure_embedding_model_with_progress,
    model_descriptor,
};

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
    chat.show_typed(&preparation)?;
    let mut last_decile = 0;
    let mut progress_error = None;
    let provisioned = ensure_embedding_model_with_progress(selected, false, |event| {
        let percentage = event.completed_bytes().saturating_mul(100) / event.total_bytes();
        let decile = percentage / 10;
        if decile > last_decile || percentage == 100 {
            last_decile = decile;
            let total_mib = event.total_bytes().div_ceil(1024 * 1024).max(1);
            let completed_mib = event.completed_bytes().div_ceil(1024 * 1024).min(total_mib);
            if let Err(error) = chat.show_typed(
                &ProgressDocument::new(
                    "Загрузка векторной модели",
                    ProgressState::Running,
                    format!("Файл: {}", event.asset()),
                )
                .with_progress(ProgressValue::new(completed_mib, total_mib).with_unit("МБ")),
            ) {
                progress_error = Some(error);
            }
        }
    });
    if let Some(error) = progress_error {
        return Err(error);
    }
    match provisioned {
        Ok(model) => {
            chat.show_typed(
                &ProgressDocument::new(
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
            )?;
            Ok(Some(model))
        }
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
