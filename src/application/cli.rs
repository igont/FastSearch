use crate::{
    application::{ProductionConfig, ProductionRuntime, RealRuntime},
    domain::{CapabilityState, RelatedQuery, SearchMode, SearchQuery, StableId},
    ports::AgentSurface,
};

const USAGE: &str = "usage:\n  fastsearch init <documents> <code> <service> [e5-root]\n  fastsearch index update <documents> <code> <service> [e5-root]\n  fastsearch index rebuild <documents> <code> <service> [e5-root]\n  fastsearch search <documents> <code> <service> <balanced|current|design> <query> [e5-root]\n  fastsearch get <documents> <code> <service> <stable-id> [e5-root]\n  fastsearch related <documents> <code> <service> <stable-id> [e5-root]\n  fastsearch status <documents> <code> <service> [e5-root]";

pub enum CliError {
    Usage,
    Runtime(String),
}

impl CliError {
    #[must_use]
    pub const fn usage() -> &'static str {
        USAGE
    }
}

/// Runs the single real document CLI composition.
pub fn execute_cli(arguments: Vec<String>) -> Result<String, CliError> {
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
            Ok(render_status(&runtime))
        }
        [command, documents, code, service, e5] if command == "init" => {
            let runtime = open_production(documents, code, service, Some(e5))?;
            Ok(render_status(&runtime))
        }
        [index, action, documents, code, service]
            if index == "index" && matches!(action.as_str(), "update" | "rebuild") =>
        {
            execute_production_index(action, documents, code, service, None)
        }
        [index, action, documents, code, service, e5]
            if index == "index" && matches!(action.as_str(), "update" | "rebuild") =>
        {
            execute_production_index(action, documents, code, service, Some(e5))
        }
        [command, documents, code, service, mode, query] if command == "search" => {
            execute_production_search(documents, code, service, mode, query, None)
        }
        [command, documents, code, service, mode, query, e5] if command == "search" => {
            execute_production_search(documents, code, service, mode, query, Some(e5))
        }
        [command, documents, code, service, id] if command == "get" || command == "related" => {
            execute_production_record(command, documents, code, service, id, None)
        }
        [command, documents, code, service, id, e5] if command == "get" || command == "related" => {
            execute_production_record(command, documents, code, service, id, Some(e5))
        }
        [command, documents, code, service] if command == "status" => {
            let runtime = open_production(documents, code, service, None)?;
            Ok(render_status(&runtime))
        }
        [command, documents, code, service, e5] if command == "status" => {
            let runtime = open_production(documents, code, service, Some(e5))?;
            Ok(render_status(&runtime))
        }
        // Backward-compatible DT2 document-only commands remain accepted, but are
        // not advertised as the DT3 production surface.
        [command, source, service] if command == "init" => {
            let runtime = open(source, service)?;
            Ok(render_status(&runtime))
        }
        [index, action, source, service] if index == "index" && action == "update" => {
            let mut runtime = open(source, service)?;
            runtime.index().map_err(runtime_error)?;
            Ok(render_status(&runtime))
        }
        [index, action, source, service] if index == "index" && action == "rebuild" => {
            let mut runtime = open(source, service)?;
            runtime.rebuild().map_err(runtime_error)?;
            Ok(render_status(&runtime))
        }
        [command, source, service, mode, query] if command == "search" => {
            let runtime = open(source, service)?;
            let query = SearchQuery::new(query, parse_mode(mode)?).map_err(runtime_error)?;
            let response = runtime.search(&query).map_err(runtime_error)?;
            Ok(render_search(&response))
        }
        [command, source, service, id] if command == "get" => {
            let runtime = open(source, service)?;
            let id = StableId::parse(id).map_err(runtime_error)?;
            let record = runtime.get(&id).map_err(runtime_error)?;
            Ok(render_get(record.as_ref()))
        }
        [command, source, service] if command == "status" => {
            let runtime = open(source, service)?;
            Ok(render_status(&runtime))
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
) -> Result<String, CliError> {
    let mut runtime = open_production(documents, code, service, e5)?;
    if action == "rebuild" {
        runtime.rebuild().map_err(runtime_error)?;
    } else {
        runtime.index().map_err(runtime_error)?;
    }
    Ok(render_status(&runtime))
}

fn execute_production_search(
    documents: &str,
    code: &str,
    service: &str,
    mode: &str,
    text: &str,
    e5: Option<&String>,
) -> Result<String, CliError> {
    let mut runtime = open_production(documents, code, service, e5)?;
    // A CLI search is a process boundary. Reconcile before querying so the
    // in-memory local-E5 projection is rebuilt from the same committed authority
    // as lexical/maps/symbols instead of reporting a false cross-process Current.
    runtime.index().map_err(runtime_error)?;
    let query = SearchQuery::new(text, parse_mode(mode)?).map_err(runtime_error)?;
    Ok(render_search(
        &runtime.search(&query).map_err(runtime_error)?,
    ))
}

fn execute_production_record(
    command: &str,
    documents: &str,
    code: &str,
    service: &str,
    id: &str,
    e5: Option<&String>,
) -> Result<String, CliError> {
    let runtime = open_production(documents, code, service, e5)?;
    let id = StableId::parse(id).map_err(runtime_error)?;
    if command == "related" {
        let records = runtime
            .related(&RelatedQuery::new(id))
            .map_err(runtime_error)?;
        Ok(render_records(&records))
    } else {
        Ok(render_get(
            runtime.get(&id).map_err(runtime_error)?.as_ref(),
        ))
    }
}

fn open(source: &str, service: &str) -> Result<RealRuntime, CliError> {
    RealRuntime::open(source, service).map_err(runtime_error)
}

fn runtime_error(error: crate::domain::FastSearchError) -> CliError {
    CliError::Runtime(error.to_string())
}

fn parse_mode(value: &str) -> Result<SearchMode, CliError> {
    match value {
        "balanced" => Ok(SearchMode::Balanced),
        "current" => Ok(SearchMode::Current),
        "design" => Ok(SearchMode::Design),
        _ => Err(CliError::Usage),
    }
}

fn render_status(runtime: &impl AgentSurface) -> String {
    let status = runtime.index_status();
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

fn render_search(response: &crate::domain::SearchResponse) -> String {
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

fn render_get(record: Option<&crate::domain::CanonicalRecord>) -> String {
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

fn render_records(records: &[crate::domain::CanonicalRecord]) -> String {
    let mut lines = vec![format!("records={}", records.len())];
    lines.extend(
        records
            .iter()
            .map(|record| format!("record={}\ttitle={}", record.id().as_str(), record.title())),
    );
    lines.join("\n")
}
