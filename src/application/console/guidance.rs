use terminal_dialogue::{ActionItem, NextStep, UserErrorDocument};

use crate::domain::{EmbeddingModelId, IndexFreshness};

pub(super) fn discovery_create() -> NextStep {
    NextStep::instruction("Проверьте найденные источники и выберите действие:")
        .with_action(ActionItem::new("Enter", "создать рабочую область"))
        .with_action(ActionItem::new("E", "изменить roots"))
        .with_action(ActionItem::new("/exit", "отменить создание"))
}

pub(super) fn discovery_apply() -> NextStep {
    NextStep::instruction("Проверьте найденные источники и выберите действие:")
        .with_action(ActionItem::new("Enter", "применить найденные roots"))
        .with_action(ActionItem::new("/exit", "отменить изменение"))
}

pub(super) fn model_catalog() -> NextStep {
    NextStep::instruction("Введите номер модели или выберите действие:")
        .with_action(ActionItem::new(
            format!("<номер 1–{}>", EmbeddingModelId::ALL.len()),
            "скачать при отсутствии и выбрать как основную",
        ))
        .with_action(ActionItem::new(
            "/model <номер|slug>",
            "выбрать модель сразу",
        ))
        .with_action(ActionItem::new(
            "/model info <номер|slug>",
            "открыть подробности",
        ))
        .with_action(ActionItem::new(
            "/model device <номер|slug> [cpu|gpu]",
            "назначить или переключить устройство",
        ))
}

pub(super) fn model_detail() -> NextStep {
    NextStep::instruction("Доступные действия:")
        .with_action(ActionItem::new("/model", "вернуться к каталогу"))
        .with_action(ActionItem::new(
            "/model <номер|slug>",
            "выбрать модель сразу",
        ))
        .with_action(ActionItem::new(
            "/model set <номер|slug>",
            "скачать при отсутствии и выбрать как основную",
        ))
        .with_action(ActionItem::new(
            "/model device <номер|slug> [cpu|gpu]",
            "назначить или переключить устройство",
        ))
}

pub(super) fn result_detail() -> NextStep {
    NextStep::instruction("Доступные действия:")
        .with_action(ActionItem::new("/related <номер>", "показать связи"))
        .with_action(ActionItem::new("/repeat", "вернуться к выдаче"))
}

pub(super) fn workspace(freshness: Option<IndexFreshness>) -> NextStep {
    match freshness {
        Some(IndexFreshness::Current) => {
            NextStep::instruction("Индекс готов. Основной следующий шаг — ввести поисковый запрос:")
                .with_action(ActionItem::new("<текст запроса>", "выполнить поиск"))
                .with_action(ActionItem::new("/model", "открыть каталог моделей"))
                .with_action(ActionItem::new(
                    "/model <номер|slug>",
                    "выбрать модель сразу",
                ))
                .with_action(ActionItem::new(
                    "/compare",
                    "сравнить результаты разных моделей",
                ))
                .with_action(ActionItem::new("/index", "проверить состояние индекса"))
                .with_action(ActionItem::new("/sources", "проверить источники"))
                .with_action(ActionItem::new("/help", "показать все команды"))
                .with_action(ActionItem::new("/exit", "закрыть FastSearch"))
        }
        Some(IndexFreshness::Stale) => NextStep::instruction("Поиск пока недоступен.")
            .with_action(ActionItem::new("/index update", "актуализировать индекс"))
            .with_action(ActionItem::new(
                "/compare",
                "сравнить готовность и выдачу разных моделей",
            ))
            .with_action(ActionItem::new("/sources", "проверить источники"))
            .with_action(ActionItem::new("/model", "открыть каталог моделей"))
            .with_action(ActionItem::new("/help", "показать все команды"))
            .with_action(ActionItem::new("/exit", "закрыть FastSearch")),
        Some(IndexFreshness::Degraded) => NextStep::instruction("Индекс повреждён или недоступен.")
            .with_action(ActionItem::new("/index", "посмотреть подробности"))
            .with_action(ActionItem::new("/index rebuild", "восстановить индекс"))
            .with_action(ActionItem::new("/sources", "проверить источники"))
            .with_action(ActionItem::new("/help", "показать все команды"))
            .with_action(ActionItem::new("/exit", "закрыть FastSearch")),
        Some(IndexFreshness::NotConfigured) | None => {
            NextStep::instruction("Подключите источники, чтобы подготовить рабочую область.")
                .with_action(ActionItem::new("/sources set", "подключить источники"))
                .with_action(ActionItem::new(
                    "/sources discover",
                    "найти источники автоматически",
                ))
                .with_action(ActionItem::new("/help", "показать все команды"))
                .with_action(ActionItem::new("/exit", "закрыть FastSearch"))
        }
    }
}

pub(super) fn search_unavailable(freshness: IndexFreshness) -> Option<UserErrorDocument> {
    match freshness {
        IndexFreshness::Stale => Some(
            UserErrorDocument::new("Поиск пока недоступен: индекс требует обновления.")
                .with_code("SEARCH_NOT_READY")
                .with_hint("Исходные документы и код при актуализации не изменяются.")
                .with_action(ActionItem::new("/index update", "актуализировать индекс")),
        ),
        IndexFreshness::Degraded => Some(
            UserErrorDocument::new("Поиск недоступен: индекс находится в состоянии ошибки.")
                .with_code("SEARCH_NOT_READY")
                .with_hint("Сначала проверьте состояние, затем восстановите индекс.")
                .with_action(ActionItem::new("/status", "посмотреть подробности"))
                .with_action(ActionItem::new("/index rebuild", "восстановить индекс")),
        ),
        IndexFreshness::NotConfigured => Some(
            UserErrorDocument::new("Поиск недоступен: индекс ещё не настроен.")
                .with_code("SEARCH_NOT_READY")
                .with_hint("Сначала подключите источники, затем подготовьте индекс.")
                .with_action(ActionItem::new("/sources set", "подключить источники"))
                .with_action(ActionItem::new("/index update", "подготовить индекс")),
        ),
        IndexFreshness::Current => None,
    }
}

pub(super) fn sources() -> NextStep {
    NextStep::instruction("Это справочный экран; отдельный режим не открыт.")
        .with_action(ActionItem::new(
            "/sources discover",
            "повторно найти папки документации и кода",
        ))
        .with_action(ActionItem::new(
            "/sources set",
            "изменить источники вручную",
        ))
        .with_action(ActionItem::new(
            "/status",
            "вернуться к сводке рабочей области",
        ))
        .with_action(ActionItem::new("/help", "показать все команды"))
        .with_action(ActionItem::new("/exit", "закрыть FastSearch"))
}

pub(super) fn search_results() -> NextStep {
    NextStep::instruction("Введите новый запрос обычным текстом или выберите действие:")
        .with_action(ActionItem::new("/open <номер>", "открыть результат"))
        .with_action(ActionItem::new("/related <номер>", "показать связи"))
        .with_action(ActionItem::new("/next", "следующая страница"))
        .with_action(ActionItem::new("/prev", "предыдущая страница"))
}
