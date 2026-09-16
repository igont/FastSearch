//! Workspace entry points and presentation for the shared public search.
use std::{path::Path, sync::atomic::AtomicBool, time::Instant};

use terminal_dialogue::{LanguagePack, ReportDocument, ReportSection, TerminalDocument};

use super::{
    ProductionRuntime, PublicSearchError, PublicSearchRequest, PublicSearchResponse,
    ThinSearchCoordinator, WorkspaceStore, production_model_set_status,
};
use crate::domain::IndexFreshness;
use crate::ports::AgentSurface;

pub fn describe_search_error(error: &PublicSearchError) -> (&'static str, &'static str) {
    use super::PublicSearchErrorCode::*;
    match error.code() {
        InvalidRequest => (
            "Некорректный поисковый запрос.",
            "Проверьте текст запроса и значения фильтров.",
        ),
        NotReady => (
            "Поиск ещё не готов.",
            "Выполните fastsearch models prepare, затем обновите индекс рабочей области.",
        ),
        StaleIndex => (
            "Поисковый индекс устарел.",
            "Выполните /index update или fastsearch index update --workspace <каталог>.",
        ),
        Busy => (
            "Поиск занят другим запросом.",
            "Повторите запрос после его завершения.",
        ),
        Timeout => (
            "Поиск не уложился в отведённое время.",
            "Уточните запрос или повторите его после завершения других тяжёлых задач.",
        ),
        ResponseTooLarge => (
            "Найденные фрагменты превышают допустимый размер ответа.",
            "Уточните запрос или ограничьте область фильтрами.",
        ),
        SearchFailed => (
            "Не удалось завершить поиск.",
            "Проверьте состояние моделей и индекса, затем повторите запрос.",
        ),
    }
}

pub fn execute_workspace_search(
    root: &Path,
    request: &PublicSearchRequest,
    json: bool,
) -> Result<String, PublicSearchError> {
    let started = Instant::now();
    let (response, _) =
        ThinSearchCoordinator::open(root)?.search(request, &AtomicBool::new(false), started)?;
    if json {
        String::from_utf8(response.serialize_bounded()?)
            .map_err(|error| PublicSearchError::search_failed(error.to_string()))
    } else {
        Ok(search_document(&response, true)
            .to_dialogue_document(&LanguagePack::russian())
            .render(false))
    }
}

/// Explicit preparation, kept outside every read-only search entry point.
pub fn prepare_workspace_search_index(root: &Path, rebuild: bool) -> Result<(), PublicSearchError> {
    ensure_search_models_ready()?;
    let store = WorkspaceStore::open(root).map_err(not_ready)?;
    let mut runtime = ProductionRuntime::open(store.production_config()).map_err(not_ready)?;
    if rebuild {
        runtime.rebuild().map_err(not_ready)?;
    } else {
        runtime.index().map_err(not_ready)?;
    }
    prepare_search_partitions(&runtime)
}

pub(super) fn ensure_search_models_ready() -> Result<(), PublicSearchError> {
    production_model_set_status().map_err(not_ready)?;
    Ok(())
}

pub(super) fn prepare_search_partitions(
    runtime: &ProductionRuntime,
) -> Result<(), PublicSearchError> {
    for model in super::thin_search::MODELS {
        if runtime.model_partition_status(model).freshness() == IndexFreshness::Current {
            continue;
        }
        let role = crate::domain::ProductionModelRole::ALL
            .into_iter()
            .find(|role| role.embedding_model() == Some(model))
            .ok_or_else(|| PublicSearchError::not_ready("missing production embedding role"))?;
        let root =
            super::model_provisioning::production_role_cache_root(role).map_err(not_ready)?;
        let status = runtime
            .build_model_partition(model, &root)
            .map_err(not_ready)?;
        if status.freshness() != IndexFreshness::Current {
            return Err(PublicSearchError::not_ready(format!(
                "{} projection did not become current",
                model.slug()
            )));
        }
    }
    Ok(())
}

pub(super) fn search_index_freshness(runtime: &ProductionRuntime) -> IndexFreshness {
    let shared = runtime.index_status().freshness();
    if shared != IndexFreshness::Current {
        return shared;
    }
    for model in super::thin_search::MODELS {
        let state = runtime.model_partition_status(model).freshness();
        if state != IndexFreshness::Current {
            return IndexFreshness::Stale;
        }
    }
    IndexFreshness::Current
}

fn not_ready(error: crate::domain::FastSearchError) -> PublicSearchError {
    PublicSearchError::not_ready(error.message())
}

pub(super) fn search_document(response: &PublicSearchResponse, expand: bool) -> ReportDocument {
    let summary = if response.count() == 0 {
        "Убедительных совпадений не найдено. Уточните вопрос или проверьте источники.".to_owned()
    } else {
        format!(
            "Показаны лучшие фрагменты: {}. Порядок не гарантирует правильность ответа.",
            response.count()
        )
    };
    let mut document = ReportDocument::new().with_section(
        ReportSection::new("Результаты поиска")
            .with_line(format!("Запрос: {}", response.query()))
            .with_line(summary),
    );
    for result in response.results() {
        document = document.with_section(
            ReportSection::new(format!("№{} · {}", result.rank(), result.title()))
                .with_line(result.path())
                .with_line(format!(
                    "Область: {} · Статус: {}",
                    match result.project_scope() {
                        super::ProjectScope::Current => "Настоящее состояние проекта",
                        super::ProjectScope::Future => "Будущее состояние проекта",
                        super::ProjectScope::General => "Общее описание",
                    },
                    match result.document_status() {
                        super::DocumentStatus::Actual => "Актуальный",
                        super::DocumentStatus::Draft => "Черновик",
                    }
                ))
                .with_line(if expand {
                    result.content().to_owned()
                } else {
                    preview(result.content())
                }),
        );
    }
    document
}

/// Preview only. The response and /open retain the complete original passage.
pub(super) fn preview(content: &str) -> String {
    const MAX_CHARS: usize = 480;
    const MAX_LINES: usize = 6;
    let mut result = String::new();
    let mut lines = 1;
    for (count, ch) in content.trim().chars().enumerate() {
        if count == MAX_CHARS || (ch == '\n' && lines == MAX_LINES) {
            result.push_str("\n… [полный фрагмент: /open <номер>]");
            break;
        }
        if ch == '\n' {
            lines += 1;
        }
        result.push(ch);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_preserves_structure_and_bounds_unicode_without_changing_source() {
        let text = "Первая строка.\n\n| A | Б |\n| 1 | 2 |";
        assert_eq!(preview(text), text);
        let long = "я".repeat(600);
        assert!(preview(&long).starts_with(&"я".repeat(480)));
        assert!(preview(&long).contains("/open"));
        assert_eq!(long.chars().count(), 600);
        assert!(preview(&"строка\n".repeat(10)).contains("/open"));
    }

    #[test]
    fn empty_search_is_an_explicit_abstention() {
        let request = PublicSearchRequest::new("неизвестное", None, None).unwrap();
        let response = PublicSearchResponse::from_ranked_results(&request, []).unwrap();
        let rendered = search_document(&response, false)
            .to_dialogue_document(&LanguagePack::russian())
            .render(false);
        assert!(rendered.contains("Убедительных совпадений не найдено"));
        assert!(!rendered.contains("100%"));
    }
}
