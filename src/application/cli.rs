use crate::{
    application::RealRuntime,
    domain::{CapabilityState, SearchMode, SearchQuery, StableId},
    ports::AgentSurface,
};

const USAGE: &str = "usage:\n  fastsearch init <source> <service>\n  fastsearch index update <source> <service>\n  fastsearch index rebuild <source> <service>\n  fastsearch search <source> <service> <balanced|current|design> <query>\n  fastsearch get <source> <service> <stable-id>\n  fastsearch status <source> <service>";

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
    match arguments.as_slice() {
        [command, source, service] if command == "init" => {
            let runtime = open(source, service)?;
            Ok(render_status(&runtime))
        }
        [index, action, source, service] if index == "index" && action == "update" => {
            let mut runtime = open(source, service)?;
            runtime.index().map_err(runtime_error)?;
            Ok(render_status(&runtime))
        }
        [index, action, source, service, flag]
            if index == "index" && action == "update" && flag == "--test-fail-projection" =>
        {
            let mut runtime = open(source, service)?;
            runtime
                .index_with_test_projection_failure()
                .map_err(runtime_error)?;
            unreachable!("the controlled projection fault always fails")
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
