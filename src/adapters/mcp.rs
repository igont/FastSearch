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

use crate::application::{PublicSearchError, PublicSearchRequest, ThinSearchCoordinator};

const SUPPORTED: &[ProtocolVersion] = &[ProtocolVersion::V_2025_11_25];

struct SearchJob {
    request: PublicSearchRequest,
    cancelled: Arc<AtomicBool>,
    response: oneshot::Sender<Result<Value, PublicSearchError>>,
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
                    let result = coordinator.search(&job.request, &job.cancelled).and_then(
                        |(response, _)| {
                            serde_json::to_value(response).map_err(|error| {
                                PublicSearchError::search_failed(error.to_string())
                            })
                        },
                    );
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
        tokio::select! {
            result = &mut awaiting => match result {
                Ok(Ok(value)) => Ok(CallToolResult::structured(value).into()),
                Ok(Err(error)) => Ok(tool_error(error).into()),
                Err(_) => Err(McpError::internal_error("runtime worker stopped", None)),
            },
            () = context.ct.cancelled() => {
                cancelled.store(true, Ordering::Release);
                let _ = awaiting.await;
                Ok(CallToolResult::structured_error(json!({"error":{"code":"SEARCH_FAILED","message":"The search failed.","retryable":true}})).into())
            }
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
