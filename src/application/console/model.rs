use std::{
    io::{self, BufRead},
    time::Duration,
};

use terminal_dialogue::{
    ActionItem, ChatSession, ProgressDashboard, ProgressPhase, ProgressTaskSpec, ProgressUnit,
    UserErrorDocument, run_progress_dashboard,
};

use crate::domain::EmbeddingModelId;

use super::super::{EmbeddingModelAvailability, ensure_embedding_model_with_progress};

pub(super) fn provision_model_with_ui<R: BufRead>(
    chat: &mut ChatSession<'_, R>,
    selected: EmbeddingModelId,
) -> io::Result<Option<EmbeddingModelAvailability>> {
    let dashboard = ProgressDashboard::new(
        "Подготовка векторной модели",
        vec![ProgressTaskSpec::new(
            selected.display_name(),
            vec![ProgressPhase::new("загрузка", ProgressUnit::bytes())],
        )],
    )
    .with_refresh_interval(Duration::from_secs(5));
    let provisioned = run_progress_dashboard(chat, dashboard, move |port| {
        port.stage(0, 0, "проверка и загрузка");
        let result = ensure_embedding_model_with_progress(selected, false, |event| {
            port.stage(0, 0, event.asset());
            port.progress(0, 0, event.completed_bytes(), event.total_bytes());
        });
        match &result {
            Ok(model) => port.complete(
                0,
                if model.downloaded() {
                    "загружена и проверена"
                } else {
                    "уже в кеше"
                },
            ),
            Err(error) => port.fail(0, error.message()),
        }
        result
    })?;
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
