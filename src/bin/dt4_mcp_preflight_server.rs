use std::borrow::Cow;

use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, JsonObject,
        ListToolsResult, ProtocolVersion, ServerCapabilities, ServerInfo, Tool,
    },
    service::RequestContext,
};
use serde_json::{Value, json};

const MODE_ENV: &str = "FASTSEARCH_DT4_MCP_PREFLIGHT_HELPER";
const SUPPORTED: &[ProtocolVersion] = &[ProtocolVersion::V_2025_11_25];

#[derive(Clone, Default)]
struct PreflightServer;

impl ServerHandler for PreflightServer {
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
                json!({"query": {"type": "string", "minLength": 1}}),
            ),
            ("required".to_owned(), json!(["query"])),
            ("additionalProperties".to_owned(), json!(false)),
        ]);
        Ok(ListToolsResult {
            tools: vec![Tool::new("search", "DT4 protocol-only search", schema)],
            ..Default::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        if request.name != "search" {
            return Err(McpError::invalid_params("unknown preflight tool", None));
        }
        if std::env::var(MODE_ENV).as_deref() == Ok("cancel") {
            context.ct.cancelled().await;
            return Ok(CallToolResult::success(vec![ContentBlock::text("late")]).into());
        }
        let query = request
            .arguments
            .as_ref()
            .and_then(|arguments| arguments.get("query"))
            .and_then(Value::as_str)
            .ok_or_else(|| McpError::invalid_params("query is required", None))?;
        Ok(CallToolResult::structured(json!({
            "preflight": true,
            "query": query,
            "working_search_invoked": false
        }))
        .into())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let service = match PreflightServer.serve(rmcp::transport::stdio()).await {
        Ok(service) => service,
        Err(error) if error.to_string().contains("initialize request") => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    service.waiting().await?;
    Ok(())
}
