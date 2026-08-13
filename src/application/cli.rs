use crate::{
    application::{ProductionConfig, ProductionRuntime, RealRuntime},
    domain::{
        BackendKind, Capability, CapabilityState, ErrorKind, FastSearchError, IndexFreshness,
        RecordKind, RelatedQuery, RetrievalChannel, SearchMode, SearchQuery, StableId,
    },
    ports::AgentSurface,
};
use serde_json::{Value, json};

const USAGE: &str = "usage:\n  fastsearch init <documents> <code> <service> [e5-root]\n  fastsearch index update <documents> <code> <service> [e5-root]\n  fastsearch index rebuild <documents> <code> <service> [e5-root]\n  fastsearch search <documents> <code> <service> <balanced|current|design> <query> [e5-root]\n  fastsearch get <documents> <code> <service> <stable-id> [e5-root]\n  fastsearch related <documents> <code> <service> <stable-id> [e5-root]\n  fastsearch status <documents> <code> <service> [e5-root]";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CliError {
    Usage,
    Runtime { code: &'static str, message: String },
}

impl CliError {
    #[must_use]
    pub const fn usage() -> &'static str {
        USAGE
    }

    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Usage => 2,
            Self::Runtime { .. } => 1,
        }
    }

    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Usage => "usage",
            Self::Runtime { code, .. } => code,
        }
    }

    #[must_use]
    pub fn message(&self) -> &str {
        match self {
            Self::Usage => "Команда или аргументы не распознаны",
            Self::Runtime { message, .. } => message,
        }
    }

    #[must_use]
    pub fn render_json(&self) -> String {
        serde_json::to_string(&json!({
            "schema_version": 1,
            "status": "error",
            "error": { "code": self.code(), "message": self.message() }
        }))
        .expect("CLI error JSON is serializable")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputFormat {
    Technical,
    Human,
    Json,
}

/// Runs the single real document CLI composition.
pub fn execute_cli(arguments: Vec<String>) -> Result<String, CliError> {
    execute_cli_formatted(arguments, OutputFormat::Technical)
}

/// Runs a direct command with an explicitly selected presentation format.
pub fn execute_cli_formatted(
    arguments: Vec<String>,
    format: OutputFormat,
) -> Result<String, CliError> {
    if let [index, action, source, service, flag] = arguments.as_slice()
        && index == "index"
        && action == "update"
        && flag == "--test-fail-projection"
    {
        let mut runtime = open(source, service)?;
        runtime
            .index_with_test_projection_failure()
            .map_err(runtime_error)?;
        unreachable!("the controlled projection fault always fails")
    }
    if arguments
        .iter()
        .any(|argument| argument == "--test-fail-projection")
    {
        return Err(CliError::Usage);
    }
    match arguments.as_slice() {
        [command, documents, code, service] if command == "init" => {
            let runtime = open_production(documents, code, service, None)?;
            Ok(render_status(&runtime, format))
        }
        [command, documents, code, service, e5] if command == "init" => {
            let runtime = open_production(documents, code, service, Some(e5))?;
            Ok(render_status(&runtime, format))
        }
        [index, action, documents, code, service]
            if index == "index" && matches!(action.as_str(), "update" | "rebuild") =>
        {
            execute_production_index(action, documents, code, service, None, format)
        }
        [index, action, documents, code, service, e5]
            if index == "index" && matches!(action.as_str(), "update" | "rebuild") =>
        {
            execute_production_index(action, documents, code, service, Some(e5), format)
        }
        [command, documents, code, service, mode, query] if command == "search" => {
            execute_production_search(documents, code, service, mode, query, None, format)
        }
        [command, documents, code, service, mode, query, e5] if command == "search" => {
            execute_production_search(documents, code, service, mode, query, Some(e5), format)
        }
        [command, documents, code, service, id] if command == "get" || command == "related" => {
            execute_production_record(command, documents, code, service, id, None, format)
        }
        [command, documents, code, service, id, e5] if command == "get" || command == "related" => {
            execute_production_record(command, documents, code, service, id, Some(e5), format)
        }
        [command, documents, code, service] if command == "status" => {
            let runtime = open_production(documents, code, service, None)?;
            Ok(render_status(&runtime, format))
        }
        [command, documents, code, service, e5] if command == "status" => {
            let runtime = open_production(documents, code, service, Some(e5))?;
            Ok(render_status(&runtime, format))
        }
        // Backward-compatible DT2 document-only commands remain accepted, but are
        // not advertised as the DT3 production surface.
        [command, source, service] if command == "init" => {
            let runtime = open(source, service)?;
            Ok(render_status(&runtime, format))
        }
        [index, action, source, service] if index == "index" && action == "update" => {
            let mut runtime = open(source, service)?;
            runtime.index().map_err(runtime_error)?;
            Ok(render_status(&runtime, format))
        }
        [index, action, source, service] if index == "index" && action == "rebuild" => {
            let mut runtime = open(source, service)?;
            runtime.rebuild().map_err(runtime_error)?;
            Ok(render_status(&runtime, format))
        }
        [command, source, service, mode, query] if command == "search" => {
            let runtime = open(source, service)?;
            let query = SearchQuery::new(query, parse_mode(mode)?).map_err(runtime_error)?;
            let response = runtime.search(&query).map_err(runtime_error)?;
            Ok(render_search(&response, format))
        }
        [command, source, service, id] if command == "get" => {
            let runtime = open(source, service)?;
            let id = StableId::parse(id).map_err(runtime_error)?;
            let record = runtime.get(&id).map_err(runtime_error)?;
            Ok(render_get(record.as_ref(), format))
        }
        [command, source, service] if command == "status" => {
            let runtime = open(source, service)?;
            Ok(render_status(&runtime, format))
        }
        _ => Err(CliError::Usage),
    }
}

fn open_production(
    documents: &str,
    code: &str,
    service: &str,
    e5: Option<&String>,
) -> Result<ProductionRuntime, CliError> {
    let config = ProductionConfig::new(documents, code, service);
    let config = match e5 {
        Some(root) => config.with_e5_root(root),
        None => config,
    };
    ProductionRuntime::open(config).map_err(runtime_error)
}

fn execute_production_index(
    action: &str,
    documents: &str,
    code: &str,
    service: &str,
    e5: Option<&String>,
    format: OutputFormat,
) -> Result<String, CliError> {
    let mut runtime = open_production(documents, code, service, e5)?;
    if action == "rebuild" {
        runtime.rebuild().map_err(runtime_error)?;
    } else {
        runtime.index().map_err(runtime_error)?;
    }
    Ok(render_status(&runtime, format))
}

fn execute_production_search(
    documents: &str,
    code: &str,
    service: &str,
    mode: &str,
    text: &str,
    e5: Option<&String>,
    format: OutputFormat,
) -> Result<String, CliError> {
    let mut runtime = open_production(documents, code, service, e5)?;
    // A CLI search is a process boundary. Reconcile before querying so the
    // in-memory local-E5 projection is rebuilt from the same committed authority
    // as lexical/maps/symbols instead of reporting a false cross-process Current.
    runtime.index().map_err(runtime_error)?;
    let query = SearchQuery::new(text, parse_mode(mode)?).map_err(runtime_error)?;
    Ok(render_search(
        &runtime.search(&query).map_err(runtime_error)?,
        format,
    ))
}

fn execute_production_record(
    command: &str,
    documents: &str,
    code: &str,
    service: &str,
    id: &str,
    e5: Option<&String>,
    format: OutputFormat,
) -> Result<String, CliError> {
    let runtime = open_production(documents, code, service, e5)?;
    let id = StableId::parse(id).map_err(runtime_error)?;
    if command == "related" {
        let records = runtime
            .related(&RelatedQuery::new(id))
            .map_err(runtime_error)?;
        Ok(render_records(&records, format))
    } else {
        Ok(render_get(
            runtime.get(&id).map_err(runtime_error)?.as_ref(),
            format,
        ))
    }
}

fn open(source: &str, service: &str) -> Result<RealRuntime, CliError> {
    RealRuntime::open(source, service).map_err(runtime_error)
}

fn runtime_error(error: FastSearchError) -> CliError {
    let code = match error.kind() {
        ErrorKind::InvalidIdentifier => "invalid_identifier",
        ErrorKind::InvalidLocator => "invalid_locator",
        ErrorKind::InvalidContent => "invalid_content",
        ErrorKind::InvalidQuery => "invalid_query",
        ErrorKind::UnsupportedSource { .. } => "unsupported_source",
        ErrorKind::CapabilityUnavailable { .. } => "capability_unavailable",
        ErrorKind::NotFound => "not_found",
        ErrorKind::StateFailure => "state_failure",
        ErrorKind::SourceFailure => "source_failure",
        ErrorKind::ProjectionFailure => "projection_failure",
        ErrorKind::DuplicateStableId => "duplicate_stable_id",
    };
    CliError::Runtime {
        code,
        message: error.to_string(),
    }
}

fn parse_mode(value: &str) -> Result<SearchMode, CliError> {
    match value {
        "balanced" => Ok(SearchMode::Balanced),
        "current" => Ok(SearchMode::Current),
        "design" => Ok(SearchMode::Design),
        _ => Err(CliError::Usage),
    }
}

fn render_status(runtime: &impl AgentSurface, format: OutputFormat) -> String {
    let status = runtime.index_status();
    if format == OutputFormat::Json {
        let capabilities = runtime
            .status()
            .into_iter()
            .map(|capability| {
                let (state, detail) = capability_state_json(capability.state());
                json!({
                    "name": capability_name(capability.capability()),
                    "state": state,
                    "detail": detail,
                })
            })
            .collect::<Vec<_>>();
        return pretty_json(json!({
            "schema_version": 1,
            "status": "ok",
            "kind": "index_status",
            "freshness": freshness_name(status.freshness()),
            "state_generation": status.state_generation(),
            "projection_generation": status.projection_generation(),
            "detail": status.detail(),
            "capabilities": capabilities,
        }));
    }
    if format == OutputFormat::Human {
        let mut lines = vec![
            "Состояние индекса".to_owned(),
            format!("  Актуальность: {}", human_freshness(status.freshness())),
            format!("  Поколение данных: {}", status.state_generation()),
            format!(
                "  Поколение проекции: {}",
                status
                    .projection_generation()
                    .map_or_else(|| "нет".to_owned(), |value| value.to_string())
            ),
            format!("  Подробности: {}", status.detail()),
            String::new(),
            "Возможности".to_owned(),
        ];
        lines.extend(runtime.status().into_iter().map(|capability| {
            format!(
                "  {} — {}",
                capability_name(capability.capability()),
                human_capability_state(capability.state())
            )
        }));
        return lines.join("\n");
    }
    let mut lines = vec![
        format!("freshness={:?}", status.freshness()),
        format!("state-generation={}", status.state_generation()),
        format!(
            "projection-generation={}",
            status
                .projection_generation()
                .map_or_else(|| "none".to_owned(), |generation| generation.to_string())
        ),
    ];
    lines.extend(
        runtime
            .status()
            .into_iter()
            .map(|capability| match capability.state() {
                CapabilityState::Available { backend } => {
                    format!("{:?}={backend:?}", capability.capability())
                }
                CapabilityState::Unavailable { .. } => {
                    format!("{:?}=Unavailable", capability.capability())
                }
                CapabilityState::Stale { .. } => format!("{:?}=Stale", capability.capability()),
                CapabilityState::Degraded { .. } => {
                    format!("{:?}=Degraded", capability.capability())
                }
            }),
    );
    lines.join("\n")
}

fn render_search(response: &crate::domain::SearchResponse, format: OutputFormat) -> String {
    if format == OutputFormat::Json {
        let hits = response
            .hits()
            .iter()
            .map(|hit| {
                json!({
                    "id": hit.record().id().as_str(),
                    "title": hit.record().title(),
                    "path": hit.record().locator().path(),
                    "record_kind": record_kind_name(hit.record().kind()),
                    "channel": channel_name(hit.channel()),
                    "score": hit.score(),
                })
            })
            .collect::<Vec<_>>();
        return pretty_json(json!({
            "schema_version": 1,
            "status": "ok",
            "kind": "search",
            "freshness": freshness_name(response.freshness()),
            "hit_count": hits.len(),
            "hits": hits,
        }));
    }
    if format == OutputFormat::Human {
        let mut lines = vec![
            "Результаты поиска".to_owned(),
            format!("  Актуальность: {}", human_freshness(response.freshness())),
            format!("  Найдено: {}", response.hits().len()),
        ];
        for (index, hit) in response.hits().iter().enumerate() {
            lines.extend([
                String::new(),
                format!("{}. {}", index + 1, hit.record().title()),
                format!("   ID: {}", hit.record().id().as_str()),
                format!("   Файл: {}", hit.record().locator().path()),
                format!("   Канал: {}", channel_name(hit.channel())),
                format!("   Оценка: {:.4}", hit.score()),
            ]);
        }
        return lines.join("\n");
    }
    let mut lines = vec![
        format!("freshness={:?}", response.freshness()),
        format!("hits={}", response.hits().len()),
    ];
    lines.extend(response.hits().iter().map(|hit| {
        format!(
            "record={}\tchannel={:?}\tscore={}",
            hit.record().id().as_str(),
            hit.channel(),
            hit.score()
        )
    }));
    lines.join("\n")
}

fn render_get(record: Option<&crate::domain::CanonicalRecord>, format: OutputFormat) -> String {
    if format == OutputFormat::Json {
        return pretty_json(json!({
            "schema_version": 1,
            "status": "ok",
            "kind": "record",
            "found": record.is_some(),
            "record": record.map(record_json),
        }));
    }
    if format == OutputFormat::Human {
        return record.map_or_else(
            || "Запись не найдена.".to_owned(),
            |record| {
                format!(
                    "Запись\n  Заголовок: {}\n  ID: {}\n  Тип: {}\n  Файл: {}\n\nСодержимое\n{}",
                    record.title(),
                    record.id().as_str(),
                    record_kind_name(record.kind()),
                    record.locator().path(),
                    record.searchable_content()
                )
            },
        );
    }
    record.map_or_else(
        || "found=false".to_owned(),
        |record| {
            format!(
                "found=true\nrecord={}\ntitle={}",
                record.id().as_str(),
                record.title()
            )
        },
    )
}

fn render_records(records: &[crate::domain::CanonicalRecord], format: OutputFormat) -> String {
    if format == OutputFormat::Json {
        return pretty_json(json!({
            "schema_version": 1,
            "status": "ok",
            "kind": "related",
            "record_count": records.len(),
            "records": records.iter().map(record_json).collect::<Vec<_>>(),
        }));
    }
    if format == OutputFormat::Human {
        let mut lines = vec![
            "Связанные записи".to_owned(),
            format!("  Найдено: {}", records.len()),
        ];
        for (index, record) in records.iter().enumerate() {
            lines.extend([
                String::new(),
                format!("{}. {}", index + 1, record.title()),
                format!("   ID: {}", record.id().as_str()),
                format!("   Файл: {}", record.locator().path()),
            ]);
        }
        return lines.join("\n");
    }
    let mut lines = vec![format!("records={}", records.len())];
    lines.extend(
        records
            .iter()
            .map(|record| format!("record={}\ttitle={}", record.id().as_str(), record.title())),
    );
    lines.join("\n")
}

fn record_json(record: &crate::domain::CanonicalRecord) -> Value {
    json!({
        "id": record.id().as_str(),
        "title": record.title(),
        "record_kind": record_kind_name(record.kind()),
        "path": record.locator().path(),
        "content": record.searchable_content(),
        "metadata": record.metadata(),
        "relations": record.relations().iter().map(StableId::as_str).collect::<Vec<_>>(),
    })
}

fn capability_state_json(state: &CapabilityState) -> (&'static str, Option<String>) {
    match state {
        CapabilityState::Available { backend } => {
            ("available", Some(backend_name(*backend).to_owned()))
        }
        CapabilityState::Unavailable { reason } => ("unavailable", Some(reason.clone())),
        CapabilityState::Stale { detail } => ("stale", Some(detail.clone())),
        CapabilityState::Degraded { detail } => ("degraded", Some(detail.clone())),
    }
}

fn human_capability_state(state: &CapabilityState) -> String {
    match state {
        CapabilityState::Available { backend } => format!("доступно ({})", backend_name(*backend)),
        CapabilityState::Unavailable { reason } => format!("недоступно: {reason}"),
        CapabilityState::Stale { detail } => format!("устарело: {detail}"),
        CapabilityState::Degraded { detail } => format!("ограничено: {detail}"),
    }
}

fn human_freshness(value: IndexFreshness) -> &'static str {
    match value {
        IndexFreshness::NotConfigured => "не настроен",
        IndexFreshness::Current => "актуален",
        IndexFreshness::Stale => "устарел",
        IndexFreshness::Degraded => "работает с ограничениями",
    }
}

fn freshness_name(value: IndexFreshness) -> &'static str {
    match value {
        IndexFreshness::NotConfigured => "not_configured",
        IndexFreshness::Current => "current",
        IndexFreshness::Stale => "stale",
        IndexFreshness::Degraded => "degraded",
    }
}

fn capability_name(value: Capability) -> &'static str {
    match value {
        Capability::Source => "source",
        Capability::State => "state",
        Capability::LexicalRetrieval => "lexical_retrieval",
        Capability::VectorRetrieval => "vector_retrieval",
        Capability::CodeMaps => "code_maps",
        Capability::Symbols => "symbols",
        Capability::AgentSurface => "agent_surface",
    }
}

fn backend_name(value: BackendKind) -> &'static str {
    match value {
        BackendKind::Mock => "mock",
        BackendKind::Real => "real",
    }
}

fn record_kind_name(value: RecordKind) -> &'static str {
    match value {
        RecordKind::MarkdownSection => "markdown_section",
        RecordKind::RegistryRow => "registry_row",
        RecordKind::CodeMap => "code_map",
        RecordKind::CodeSymbol => "code_symbol",
    }
}

fn channel_name(value: RetrievalChannel) -> &'static str {
    match value {
        RetrievalChannel::Exact => "exact",
        RetrievalChannel::Lexical => "lexical",
        RetrievalChannel::Vector => "vector",
        RetrievalChannel::CodeMap => "code_map",
        RetrievalChannel::Symbol => "symbol",
    }
}

fn pretty_json(value: Value) -> String {
    serde_json::to_string_pretty(&value).expect("CLI success JSON is serializable")
}
