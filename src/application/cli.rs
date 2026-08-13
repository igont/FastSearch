use crate::{
    application::{ProductionConfig, ProductionRuntime, RealRuntime},
    domain::{
        BackendKind, CanonicalRecord, Capability, CapabilityState, CapabilityStatus, ErrorKind,
        FastSearchError, IndexFreshness, LifecycleStatus, RecordKind, RelatedQuery,
        RetrievalChannel, SearchMode, SearchQuery, SearchResponse, StableId,
    },
    ports::AgentSurface,
};
use serde_json::{json, Value};

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

/// Runs the production semantic/code CLI and the retained DT2 compatibility commands.
pub fn execute_cli(arguments: Vec<String>) -> Result<String, CliError> {
    execute_cli_formatted(arguments, OutputFormat::Technical)
}

/// Runs a direct command with an explicitly selected presentation format.
pub fn execute_cli_formatted(
    arguments: Vec<String>,
    format: OutputFormat,
) -> Result<String, CliError> {
    Ok(render_outcome(
        execute_command(parse_command(arguments)?)?,
        format,
    ))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Command {
    Production {
        config: ProductionCommandConfig,
        action: CommandAction,
    },
    Compatibility {
        source: String,
        service: String,
        action: CommandAction,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ProductionCommandConfig {
    documents: String,
    code: String,
    service: String,
    e5: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CommandAction {
    Init,
    Index { rebuild: bool },
    Search { mode: SearchMode, text: String },
    Get { id: String },
    Related { id: String },
    Status,
    TestProjectionFailure,
}

#[derive(Clone, Debug)]
pub(super) enum CommandOutcome {
    Status {
        status: LifecycleStatus,
        capabilities: Vec<CapabilityStatus>,
    },
    Search(SearchResponse),
    Record(Option<CanonicalRecord>),
    Related(Vec<CanonicalRecord>),
}

impl CommandOutcome {
    #[cfg(test)]
    fn status_for_test() -> Self {
        Self::Status {
            status: LifecycleStatus::not_configured("test presenter outcome"),
            capabilities: Vec::new(),
        }
    }
}

impl Command {
    #[cfg(test)]
    fn name(&self) -> &'static str {
        match self {
            Self::Production { action, .. } | Self::Compatibility { action, .. } => action.name(),
        }
    }
}

impl CommandAction {
    #[cfg(test)]
    fn name(&self) -> &'static str {
        match self {
            Self::Init => "init",
            Self::Index { .. } | Self::TestProjectionFailure => "index",
            Self::Search { .. } => "search",
            Self::Get { .. } => "get",
            Self::Related { .. } => "related",
            Self::Status => "status",
        }
    }
}

/// Parses only the private direct-CLI grammar. This is not an application or MCP DTO.
fn parse_command(arguments: Vec<String>) -> Result<Command, CliError> {
    if arguments
        .iter()
        .any(|value| value == "--test-fail-projection")
    {
        return match arguments.as_slice() {
            [index, action, source, service, flag]
                if index == "index" && action == "update" && flag == "--test-fail-projection" =>
            {
                Ok(Command::Compatibility {
                    source: source.clone(),
                    service: service.clone(),
                    action: CommandAction::TestProjectionFailure,
                })
            }
            _ => Err(CliError::Usage),
        };
    }
    match arguments.as_slice() {
        [command, documents, code, service] if command == "init" || command == "status" => Ok(
            production_command(documents, code, service, None, command_action(command)?),
        ),
        [command, documents, code, service, e5] if command == "init" || command == "status" => Ok(
            production_command(documents, code, service, Some(e5), command_action(command)?),
        ),
        [index, action, documents, code, service]
            if index == "index" && matches!(action.as_str(), "update" | "rebuild") =>
        {
            Ok(production_command(
                documents,
                code,
                service,
                None,
                index_action(action),
            ))
        }
        [index, action, documents, code, service, e5]
            if index == "index" && matches!(action.as_str(), "update" | "rebuild") =>
        {
            Ok(production_command(
                documents,
                code,
                service,
                Some(e5),
                index_action(action),
            ))
        }
        [command, documents, code, service, mode, query] if command == "search" => {
            Ok(production_command(
                documents,
                code,
                service,
                None,
                CommandAction::Search {
                    mode: parse_mode(mode)?,
                    text: query.clone(),
                },
            ))
        }
        [command, documents, code, service, mode, query, e5] if command == "search" => {
            Ok(production_command(
                documents,
                code,
                service,
                Some(e5),
                CommandAction::Search {
                    mode: parse_mode(mode)?,
                    text: query.clone(),
                },
            ))
        }
        [command, documents, code, service, id] if command == "get" || command == "related" => Ok(
            production_command(documents, code, service, None, record_action(command, id)),
        ),
        [command, documents, code, service, id, e5] if command == "get" || command == "related" => {
            Ok(production_command(
                documents,
                code,
                service,
                Some(e5),
                record_action(command, id),
            ))
        }
        [command, source, service] if command == "init" || command == "status" => Ok(
            compatibility_command(source, service, command_action(command)?),
        ),
        [index, action, source, service]
            if index == "index" && matches!(action.as_str(), "update" | "rebuild") =>
        {
            Ok(compatibility_command(source, service, index_action(action)))
        }
        [command, source, service, mode, query] if command == "search" => {
            Ok(compatibility_command(
                source,
                service,
                CommandAction::Search {
                    mode: parse_mode(mode)?,
                    text: query.clone(),
                },
            ))
        }
        [command, source, service, id] if command == "get" => Ok(compatibility_command(
            source,
            service,
            record_action(command, id),
        )),
        _ => Err(CliError::Usage),
    }
}

pub(super) fn production_command(
    documents: &str,
    code: &str,
    service: &str,
    e5: Option<&String>,
    action: CommandAction,
) -> Command {
    Command::Production {
        config: ProductionCommandConfig {
            documents: documents.to_owned(),
            code: code.to_owned(),
            service: service.to_owned(),
            e5: e5.cloned(),
        },
        action,
    }
}

fn compatibility_command(source: &str, service: &str, action: CommandAction) -> Command {
    Command::Compatibility {
        source: source.to_owned(),
        service: service.to_owned(),
        action,
    }
}

fn command_action(command: &str) -> Result<CommandAction, CliError> {
    match command {
        "init" => Ok(CommandAction::Init),
        "status" => Ok(CommandAction::Status),
        _ => Err(CliError::Usage),
    }
}

fn index_action(action: &str) -> CommandAction {
    CommandAction::Index {
        rebuild: action == "rebuild",
    }
}

fn record_action(command: &str, id: &str) -> CommandAction {
    if command == "related" {
        CommandAction::Related { id: id.to_owned() }
    } else {
        CommandAction::Get { id: id.to_owned() }
    }
}

/// Executes one private CLI command. It deliberately stays below the public application surface.
pub(super) fn execute_command(command: Command) -> Result<CommandOutcome, CliError> {
    match command {
        Command::Production { config, action } => execute_production_command(config, action),
        Command::Compatibility {
            source,
            service,
            action,
        } => execute_compatibility_command(&source, &service, action),
    }
}

fn execute_production_command(
    config: ProductionCommandConfig,
    action: CommandAction,
) -> Result<CommandOutcome, CliError> {
    let mut runtime = open_production(
        &config.documents,
        &config.code,
        &config.service,
        config.e5.as_ref(),
    )?;
    match action {
        CommandAction::Init | CommandAction::Status => Ok(status_outcome(&runtime)),
        CommandAction::Index { rebuild } => {
            if rebuild {
                runtime.rebuild().map_err(runtime_error)?;
            } else {
                runtime.index().map_err(runtime_error)?;
            }
            Ok(status_outcome(&runtime))
        }
        CommandAction::Search { mode, text } => {
            // A CLI search is a process boundary. Reconcile the in-memory local-E5
            // projection from the committed authority before querying.
            runtime.index().map_err(runtime_error)?;
            let query = SearchQuery::new(&text, mode).map_err(runtime_error)?;
            Ok(CommandOutcome::Search(
                runtime.search(&query).map_err(runtime_error)?,
            ))
        }
        CommandAction::Get { id } => record_outcome(&runtime, &id, false),
        CommandAction::Related { id } => record_outcome(&runtime, &id, true),
        CommandAction::TestProjectionFailure => Err(CliError::Usage),
    }
}

fn execute_compatibility_command(
    source: &str,
    service: &str,
    action: CommandAction,
) -> Result<CommandOutcome, CliError> {
    let mut runtime = open(source, service)?;
    match action {
        CommandAction::Init | CommandAction::Status => Ok(status_outcome(&runtime)),
        CommandAction::Index { rebuild } => {
            if rebuild {
                runtime.rebuild().map_err(runtime_error)?;
            } else {
                runtime.index().map_err(runtime_error)?;
            }
            Ok(status_outcome(&runtime))
        }
        CommandAction::Search { mode, text } => {
            let query = SearchQuery::new(&text, mode).map_err(runtime_error)?;
            Ok(CommandOutcome::Search(
                runtime.search(&query).map_err(runtime_error)?,
            ))
        }
        CommandAction::Get { id } => record_outcome(&runtime, &id, false),
        CommandAction::TestProjectionFailure => {
            runtime
                .index_with_test_projection_failure()
                .map_err(runtime_error)?;
            unreachable!("the controlled projection fault always fails")
        }
        CommandAction::Related { .. } => Err(CliError::Usage),
    }
}

fn status_outcome(runtime: &impl AgentSurface) -> CommandOutcome {
    CommandOutcome::Status {
        status: runtime.index_status(),
        capabilities: runtime.status(),
    }
}

fn record_outcome(
    runtime: &impl AgentSurface,
    raw_id: &str,
    related: bool,
) -> Result<CommandOutcome, CliError> {
    let id = StableId::parse(raw_id).map_err(runtime_error)?;
    if related {
        Ok(CommandOutcome::Related(
            runtime
                .related(&RelatedQuery::new(id))
                .map_err(runtime_error)?,
        ))
    } else {
        Ok(CommandOutcome::Record(
            runtime.get(&id).map_err(runtime_error)?,
        ))
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

pub(in crate::application) use presenters::render_outcome;

/// Pure CLI presentation: typed outcomes in, text out. It owns no runtime or filesystem work.
mod presenters {
    use super::*;

    pub(in crate::application) fn render_outcome(
        outcome: CommandOutcome,
        format: OutputFormat,
    ) -> String {
        match outcome {
            CommandOutcome::Status {
                status,
                capabilities,
            } => render_status(&status, &capabilities, format),
            CommandOutcome::Search(response) => render_search(&response, format),
            CommandOutcome::Record(record) => render_get(record.as_ref(), format),
            CommandOutcome::Related(records) => render_records(&records, format),
        }
    }

    fn render_status(
        status: &LifecycleStatus,
        capabilities: &[CapabilityStatus],
        format: OutputFormat,
    ) -> String {
        if format == OutputFormat::Json {
            let capabilities = capabilities
                .iter()
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
            lines.extend(capabilities.iter().map(|capability| {
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
            capabilities
                .iter()
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
            CapabilityState::Available { backend } => {
                format!("доступно ({})", backend_name(*backend))
            }
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
}

#[cfg(test)]
mod command_contract_tests {
    use super::{parse_command, render_outcome, CommandOutcome, OutputFormat};

    #[test]
    fn parses_a_production_status_command_once_before_dispatch() {
        let command = parse_command(vec![
            "status".to_owned(),
            "documents".to_owned(),
            "code".to_owned(),
            "service".to_owned(),
        ])
        .expect("typed production command");

        assert_eq!(command.name(), "status");
    }

    #[test]
    fn presenter_renders_a_typed_status_outcome_without_runtime_access() {
        let output = render_outcome(CommandOutcome::status_for_test(), OutputFormat::Technical);

        assert!(output.contains("freshness="));
    }
}
