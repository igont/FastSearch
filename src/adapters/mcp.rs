//! Official rmcp stdio adapter for the single DT4 public search tool.

use std::{
    borrow::Cow,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, SyncSender, TrySendError},
    },
    thread,
    time::{Duration, Instant},
};

use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, JsonObject, ListToolsResult,
        ProtocolVersion, ServerCapabilities, ServerInfo, Tool, ToolAnnotations,
    },
    service::RequestContext,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::oneshot;

use crate::application::{
    PublicSearchError, PublicSearchRequest, SEARCH_DEADLINE, ThinSearchCoordinator,
};

const SUPPORTED: &[ProtocolVersion] = &[ProtocolVersion::V_2025_11_25];

struct SearchJob {
    request: PublicSearchRequest,
    cancelled: Arc<AtomicBool>,
    admitted_at: Instant,
    response: oneshot::Sender<Result<Value, PublicSearchError>>,
}

enum SearchJobOutcome {
    Completed(Result<Value, PublicSearchError>),
    Cancelled,
    TimedOut,
    WorkerStopped,
}

#[derive(Clone)]
pub struct ThinMcpServer {
    jobs: SyncSender<SearchJob>,
    outstanding: Arc<AtomicUsize>,
}

impl ThinMcpServer {
    fn start(workspace: &Path) -> Result<(Self, thread::JoinHandle<()>), PublicSearchError> {
        let (jobs, receiver) = mpsc::sync_channel::<SearchJob>(2);
        let outstanding = Arc::new(AtomicUsize::new(0));
        let worker_outstanding = Arc::clone(&outstanding);
        let (admitted, admission) = mpsc::sync_channel(0);
        let workspace = workspace.to_path_buf();
        let worker = thread::Builder::new()
            .name("fastsearch-mcp-runtime".to_owned())
            .spawn(move || {
                let mut coordinator = match ThinSearchCoordinator::open(&workspace) {
                    Ok(value) => {
                        let _ = admitted.send(Ok(()));
                        value
                    }
                    Err(error) => {
                        let _ = admitted.send(Err(error));
                        return;
                    }
                };
                while let Ok(job) = receiver.recv() {
                    let result = coordinator
                        .search(&job.request, &job.cancelled, job.admitted_at)
                        .and_then(|(response, _)| {
                            serde_json::to_value(response).map_err(|error| {
                                PublicSearchError::search_failed(error.to_string())
                            })
                        });
                    if !job.cancelled.load(Ordering::Acquire) {
                        let _ = job.response.send(result);
                    }
                    worker_outstanding.fetch_sub(1, Ordering::AcqRel);
                }
            })
            .map_err(|error| PublicSearchError::not_ready(error.to_string()))?;
        admission.recv().map_err(|_| {
            PublicSearchError::not_ready("runtime worker stopped during admission")
        })??;
        Ok((Self { jobs, outstanding }, worker))
    }
}

impl ServerHandler for ThinMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(SUPPORTED)
    }

    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let schema = JsonObject::from_iter([
            ("type".to_owned(), json!("object")),
            (
                "properties".to_owned(),
                json!({
                    "query": {"type": "string", "minLength": 1, "maxLength": 1024},
                    "project_scope": {"type": "string", "enum": ["current", "future", "general"]},
                    "document_status": {"type": "string", "enum": ["draft", "actual"]}
                }),
            ),
            ("required".to_owned(), json!(["query"])),
            ("additionalProperties".to_owned(), json!(false)),
        ]);
        Ok(ListToolsResult {
            tools: vec![
                Tool::new("search", "Search the admitted FastSearch workspace", schema)
                    .with_output_schema::<crate::application::PublicSearchResponse>()
                    .with_annotations(
                        ToolAnnotations::new()
                            .read_only(true)
                            .destructive(false)
                            .idempotent(true)
                            .open_world(false),
                    ),
            ],
            ..Default::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        if request.name != "search" {
            return Err(McpError::invalid_params("unknown tool", None));
        }
        let arguments = request
            .arguments
            .ok_or_else(|| McpError::invalid_params("search arguments are required", None))?;
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct WireSearchArgs {
            query: String,
            project_scope: Option<String>,
            document_status: Option<String>,
        }
        let wire: WireSearchArgs = serde_json::from_value(Value::Object(arguments))
            .map_err(|_| McpError::invalid_params("invalid search arguments", None))?;
        let request = match PublicSearchRequest::from_wire_values(
            wire.query,
            wire.project_scope.as_deref(),
            wire.document_status.as_deref(),
        ) {
            Ok(value) => value,
            Err(error) => return Ok(tool_error(error).into()),
        };
        let admitted_at = Instant::now();
        if self
            .outstanding
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < 2).then_some(count + 1)
            })
            .is_err()
        {
            return Ok(tool_error(PublicSearchError::busy(
                "one active and one waiting request already occupy the runtime",
            ))
            .into());
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        let (response, mut awaiting) = oneshot::channel();
        let job = SearchJob {
            request,
            cancelled: Arc::clone(&cancelled),
            admitted_at,
            response,
        };
        match self.jobs.try_send(job) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                self.outstanding.fetch_sub(1, Ordering::AcqRel);
                return Ok(tool_error(PublicSearchError::busy(
                    "one active and one waiting request already occupy the runtime",
                ))
                .into());
            }
            Err(TrySendError::Disconnected(_)) => {
                self.outstanding.fetch_sub(1, Ordering::AcqRel);
                return Err(McpError::internal_error("runtime worker stopped", None));
            }
        }
        match await_search_job(
            &mut awaiting,
            Arc::clone(&cancelled),
            admitted_at,
            SEARCH_DEADLINE,
            context.ct.cancelled(),
        )
        .await
        {
            SearchJobOutcome::Completed(Ok(value)) => Ok(CallToolResult::structured(value).into()),
            SearchJobOutcome::Completed(Err(error)) => Ok(tool_error(error).into()),
            SearchJobOutcome::TimedOut => Ok(tool_error(PublicSearchError::timeout(
                "the 30 second MCP call deadline elapsed",
            ))
            .into()),
            SearchJobOutcome::Cancelled => Ok(CallToolResult::structured_error(json!({
                "error":{"code":"SEARCH_FAILED","message":"The search failed.","retryable":true}
            }))
            .into()),
            SearchJobOutcome::WorkerStopped => {
                Err(McpError::internal_error("runtime worker stopped", None))
            }
        }
    }
}

async fn await_search_job<C>(
    awaiting: &mut oneshot::Receiver<Result<Value, PublicSearchError>>,
    cancelled: Arc<AtomicBool>,
    admitted_at: Instant,
    budget: Duration,
    cancellation: C,
) -> SearchJobOutcome
where
    C: std::future::Future<Output = ()>,
{
    let remaining = budget.saturating_sub(admitted_at.elapsed());
    tokio::pin!(cancellation);
    tokio::select! {
        biased;
        () = &mut cancellation => {
            cancelled.store(true, Ordering::Release);
            SearchJobOutcome::Cancelled
        }
        () = tokio::time::sleep(remaining) => {
            cancelled.store(true, Ordering::Release);
            SearchJobOutcome::TimedOut
        }
        result = awaiting => match result {
            Ok(value) => SearchJobOutcome::Completed(value),
            Err(_) => SearchJobOutcome::WorkerStopped,
        }
    }
}

fn tool_error(error: PublicSearchError) -> CallToolResult {
    let value = serde_json::to_value(&error).unwrap_or_else(
        |_| json!({"code":"SEARCH_FAILED","message":"The search failed.","retryable":true}),
    );
    CallToolResult::structured_error(json!({"error": value}))
}

pub fn run_stdio(workspace: PathBuf) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    runtime.block_on(async move {
        let (server, worker) =
            ThinMcpServer::start(&workspace).map_err(|error| error.message().to_owned())?;
        let service = match server.serve(rmcp::transport::stdio()).await {
            Ok(service) => service,
            Err(error) if error.to_string().contains("initialize request") => {
                drop(error);
                return worker
                    .join()
                    .map_err(|_| "runtime worker panicked".to_owned());
            }
            Err(error) => return Err(error.to_string()),
        };
        service.waiting().await.map_err(|error| error.to_string())?;
        worker
            .join()
            .map_err(|_| "runtime worker panicked".to_owned())
    })
}

#[cfg(test)]
mod tests {
    use std::{future::pending, sync::atomic::Ordering, time::Duration};

    use serde_json::json;
    use tokio::sync::oneshot;

    use super::{SearchJobOutcome, await_search_job};

    #[tokio::test]
    async fn queued_wait_consumes_the_single_admission_budget() {
        let budget = Duration::from_millis(200);
        let admitted_at = std::time::Instant::now() - Duration::from_millis(150);
        let obsolete = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_obsolete = std::sync::Arc::clone(&obsolete);
        let (sender, mut receiver) = oneshot::channel();
        let worker = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if !worker_obsolete.load(Ordering::Acquire) {
                let _ = sender.send(Ok(json!({"late": true})));
                false
            } else {
                true
            }
        });
        let observed_at = std::time::Instant::now();

        let outcome = await_search_job(
            &mut receiver,
            std::sync::Arc::clone(&obsolete),
            admitted_at,
            budget,
            pending(),
        )
        .await;

        assert!(matches!(outcome, SearchJobOutcome::TimedOut));
        assert!(observed_at.elapsed() < Duration::from_millis(100));
        assert!(obsolete.load(Ordering::Acquire));
        assert!(worker.await.expect("controlled worker completes"));
    }

    #[tokio::test]
    async fn late_blocking_result_is_suppressed_after_timeout() {
        let budget = Duration::from_millis(50);
        let admitted_at = std::time::Instant::now();
        let obsolete = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_obsolete = std::sync::Arc::clone(&obsolete);
        let (sender, mut receiver) = oneshot::channel();
        let worker = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            if worker_obsolete.load(Ordering::Acquire) {
                true
            } else {
                sender.send(Ok(json!({"late": true}))).is_err()
            }
        });

        let outcome = await_search_job(
            &mut receiver,
            std::sync::Arc::clone(&obsolete),
            admitted_at,
            budget,
            pending(),
        )
        .await;

        assert!(matches!(outcome, SearchJobOutcome::TimedOut));
        assert!(obsolete.load(Ordering::Acquire));
        assert!(worker.await.expect("controlled blocking job completes"));
    }
}
