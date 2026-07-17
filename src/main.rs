//! A minimal, production-ready Model Context Protocol (MCP) server that
//! exposes a single tool, `ask_claude`, backed by Anthropic's Claude API.
//!
//! The server speaks MCP over stdio: **stdout is the protocol channel**, so all
//! human-readable logging is sent to stderr. Point any MCP client (for example
//! Claude Desktop) at the built binary and it can call `ask_claude` to have this
//! server forward a prompt to Claude and return the reply.

use crimson_crab::model_ids::CLAUDE_OPUS_4_8;
use crimson_crab::prelude::*;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo};
use rmcp::{schemars, tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler};
use rmcp::{transport::stdio, ServiceExt};

/// Arguments for the `ask_claude` tool.
///
/// The `JsonSchema` derive drives the tool's input schema that MCP clients see.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AskClaudeArgs {
    /// The prompt to send to Claude.
    pub prompt: String,
    /// Optional system prompt that steers Claude's behavior.
    #[serde(default)]
    pub system: Option<String>,
}

/// The MCP server. Holds a single, reusable Claude client that is built once at
/// startup and cloned cheaply per request (the client is internally reference
/// counted).
#[derive(Clone)]
pub struct ClaudeServer {
    client: Client,
    tool_router: ToolRouter<ClaudeServer>,
}

#[tool_router]
impl ClaudeServer {
    /// Build the server, constructing the Claude client from the environment
    /// (`ANTHROPIC_API_KEY`).
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            client: Client::from_env()?,
            tool_router: Self::tool_router(),
        })
    }

    /// Send `prompt` (with an optional `system` prompt) to Claude and return the
    /// concatenated text of the reply.
    #[tool(description = "Ask Anthropic's Claude a question and return its answer.")]
    async fn ask_claude(
        &self,
        Parameters(AskClaudeArgs { prompt, system }): Parameters<AskClaudeArgs>,
    ) -> Result<CallToolResult, McpError> {
        let mut builder = MessagesRequest::builder()
            .model(CLAUDE_OPUS_4_8)
            .max_tokens(1024)
            .messages(vec![MessageParam::user(prompt)]);
        if let Some(system) = system {
            builder = builder.system(system);
        }

        let request = match builder.build() {
            Ok(request) => request,
            // Return a tool-level error (visible to the caller) rather than panicking.
            Err(err) => {
                return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                    "failed to build request: {err}"
                ))]));
            }
        };

        match self.client.messages().create(&request).await {
            Ok(message) => Ok(CallToolResult::success(vec![ContentBlock::text(
                message.text(),
            )])),
            Err(err) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "Claude request failed: {err}"
            ))])),
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for ClaudeServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::from_build_env())
            .with_instructions(
                "Exposes an `ask_claude` tool that forwards a prompt to Anthropic's \
                 Claude and returns the reply."
                    .to_string(),
            )
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // MCP uses stdout for the protocol, so logging must go to stderr only.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    tracing::info!("starting crimson-crab MCP server");

    let service = ClaudeServer::new()?
        .serve(stdio())
        .await
        .inspect_err(|err| tracing::error!(?err, "failed to start MCP server"))?;

    service.waiting().await?;
    Ok(())
}
