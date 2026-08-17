use terminal_dialogue::{CommandCatalog, CommandSpec};

#[must_use]
pub fn help_text() -> &'static str {
    "FastSearch — локальный поиск по документации и исходному коду.\n\nЗапуск:\n  fastsearch                       рабочие области и интерактивный поиск\n  fastsearch chat                  то же самое явно\n  fastsearch [--json] <команда>    compatibility CLI для scripts/CI\n  fastsearch --help                эта справка\n  fastsearch --version             версия программы\n\nПоиск:\n  обычный текст                    выполнить поиск\n  /search <запрос>                 выполнить поиск явно\n  /related N                       связанные материалы результата\n\nИсточники и индекс:\n  /workspace                       выбрать или создать область\n  /sources [discover|set]          показать, найти или изменить roots\n  /status                          состояние области и providers\n  /index [status|update|rebuild]   обслуживание индекса\n\nМодели и сравнение:\n  /model                           каталог embedding-моделей\n  /model set <N|slug>              выбрать модель поиска\n  /model info <N|slug>             сведения о модели\n  /model device <N|slug> [cpu|gpu] назначить либо переключить устройство\n  /compare                         сравнить выдачу готовых моделей\n  /experiment record <оценка>      записать оценку поиска\n\nНавигация:\n  /open N                          открыть результат\n  /next | /prev | /page N         навигация по выдаче\n  /repeat                          повторить текущую выдачу\n\nПриложение:\n  /help                            контекстная справка\n  /version                         версия программы\n  /exit                            выход\n\nCompatibility-команды сохраняют прежние arguments documents/code/service до отдельного machine-CLI cutover."
}

pub(super) fn workspace_catalog() -> CommandCatalog {
    CommandCatalog::new(vec![
        grouped(
            "search",
            "найти документы или код",
            "/search <запрос>",
            "ПОИСК",
        ),
        grouped(
            "related",
            "показать связанные записи",
            "/related N",
            "ПОИСК",
        ),
        grouped(
            "sources set",
            "изменить documentation/code roots",
            "/sources set",
            "ИСТОЧНИКИ И ИНДЕКС",
        ),
        grouped(
            "sources discover",
            "заново обнаружить roots внутри области",
            "/sources discover",
            "ИСТОЧНИКИ И ИНДЕКС",
        ),
        grouped(
            "sources",
            "показать два source contours",
            "/sources",
            "ИСТОЧНИКИ И ИНДЕКС",
        ),
        grouped(
            "model device",
            "назначить CPU/GPU или переключить устройство",
            "/model device <N|slug> [cpu|gpu]",
            "МОДЕЛИ И СРАВНЕНИЕ",
        ),
        grouped(
            "model set",
            "выбрать embedding-модель",
            "/model set <N|slug>",
            "МОДЕЛИ И СРАВНЕНИЕ",
        ),
        grouped(
            "model info",
            "показать источник и технические сведения модели",
            "/model info <N|slug>",
            "МОДЕЛИ И СРАВНЕНИЕ",
        ),
        grouped(
            "model",
            "показать каталог embedding-моделей",
            "/model",
            "МОДЕЛИ И СРАВНЕНИЕ",
        ),
        grouped(
            "compare",
            "сравнить выдачу всех embedding-моделей",
            "/compare",
            "МОДЕЛИ И СРАВНЕНИЕ",
        ),
        grouped(
            "experiment record",
            "записать оценку последнего поиска",
            "/experiment record <оценка>",
            "ПОИСК",
        ),
        grouped(
            "index rebuild",
            "полностью перестроить local index",
            "/index rebuild",
            "ИСТОЧНИКИ И ИНДЕКС",
        ),
        grouped(
            "index update",
            "применить изменения sources",
            "/index update",
            "ИСТОЧНИКИ И ИНДЕКС",
        ),
        grouped(
            "index",
            "показать состояние индекса",
            "/index",
            "ИСТОЧНИКИ И ИНДЕКС",
        )
        .with_alias("index status"),
        grouped(
            "status",
            "показать область, freshness и capabilities",
            "/status",
            "ИСТОЧНИКИ И ИНДЕКС",
        ),
        grouped(
            "workspace",
            "сменить рабочую область",
            "/workspace",
            "ИСТОЧНИКИ И ИНДЕКС",
        ),
        grouped(
            "open",
            "открыть результат по номеру",
            "/open <N>",
            "НАВИГАЦИЯ",
        ),
        grouped(
            "next",
            "следующая страница результатов",
            "/next",
            "НАВИГАЦИЯ",
        ),
        grouped(
            "prev",
            "предыдущая страница результатов",
            "/prev",
            "НАВИГАЦИЯ",
        )
        .with_alias("previous"),
        grouped("page", "перейти к странице", "/page <N>", "НАВИГАЦИЯ"),
        grouped("repeat", "повторить текущую выдачу", "/repeat", "НАВИГАЦИЯ"),
        grouped("version", "показать версию", "/version", "ПРИЛОЖЕНИЕ")
            .with_alias("--version")
            .with_alias("-V"),
        grouped("help", "показать команды", "/help", "ПРИЛОЖЕНИЕ")
            .with_alias("--help")
            .with_alias("-h")
            .with_alias("помощь"),
        grouped("exit", "закрыть FastSearch", "/exit", "ПРИЛОЖЕНИЕ")
            .with_alias("quit")
            .with_alias("выход"),
    ])
    .expect("FastSearch command catalog is static and valid")
}

fn grouped(name: &str, summary: &str, usage: &str, group: &str) -> CommandSpec {
    CommandSpec::new(name, summary, usage).with_group(group)
}
