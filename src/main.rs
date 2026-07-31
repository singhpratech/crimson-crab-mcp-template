//! A minimal, production-ready Model Context Protocol (MCP) server backed by
//! Anthropic's Claude API. It exposes five tools: `ask_claude` (send a prompt,
//! get the reply), `chat` (multi-turn conversation), `count_tokens` (price a
//! prompt without running it), `list_models` (enumerate the models the API key
//! can use), and `get_model` (limits and metadata for one model).
//!
//! The server speaks MCP over stdio: **stdout is the protocol channel**, so all
//! human-readable logging is sent to stderr. Point any MCP client (for example
//! Claude Desktop) at the built binary and it can call the tools to have this
//! server forward requests to Claude.

use crimson_crab::api::ModelListParams;
use crimson_crab::model_ids::CLAUDE_OPUS_5;
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

/// Who spoke a turn of a `chat` conversation: `"user"` or `"assistant"`.
///
/// Modelling this as an enum (rather than a `String` checked at run time) makes
/// the constraint machine-readable: the derived JSON schema carries
/// `"enum": ["user", "assistant"]`, so MCP clients can see the allowed values
/// up front instead of discovering them from an error message.
///
/// Two schemars details are deliberate here, because they decide whether that
/// signal actually reaches the client:
///
/// - **No doc comments on the variants.** Documenting each variant would force
///   schemars to emit `oneOf: [{const: "user"}, {const: "assistant"}]` so it has
///   somewhere to hang the per-variant descriptions. That is valid JSON Schema
///   but far less widely understood than a flat `enum` array.
/// - **`#[schemars(inline)]`.** Without it the `role` property is just a
///   `$ref` into `$defs`, and clients that do not resolve refs see no
///   constraint at all. Inlining puts the `enum` directly on the property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
#[schemars(inline)]
pub enum ChatRole {
    User,
    Assistant,
}

/// One turn of a `chat` conversation.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ChatTurn {
    /// Who spoke this turn: "user" or "assistant".
    pub role: ChatRole,
    /// The text of the turn.
    pub content: String,
}

/// Arguments for the `chat` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ChatArgs {
    /// The conversation so far, oldest turn first. Must end with a "user" turn.
    pub messages: Vec<ChatTurn>,
    /// Optional system prompt that steers Claude's behavior.
    #[serde(default)]
    pub system: Option<String>,
    /// Model id to use. Defaults to the same model `ask_claude` uses.
    #[serde(default)]
    pub model: Option<String>,
    /// Maximum tokens Claude may generate (default 1024).
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

/// Arguments for the `get_model` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetModelArgs {
    /// The model id to look up, for example "claude-opus-5".
    pub model: String,
}

/// Arguments for the `count_tokens` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CountTokensArgs {
    /// The prompt whose token count you want.
    pub prompt: String,
    /// Optional system prompt to include in the count.
    #[serde(default)]
    pub system: Option<String>,
    /// Model id to count against (counts are model-specific). Defaults to the
    /// same model `ask_claude` uses.
    #[serde(default)]
    pub model: Option<String>,
}

/// Arguments for the `list_models` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListModelsArgs {
    /// Maximum number of models to return (the API defaults to 20).
    #[serde(default)]
    pub limit: Option<u32>,
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
            .model(CLAUDE_OPUS_5)
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

    /// Continue a multi-turn conversation with Claude and return the reply.
    #[tool(
        description = "Continue a multi-turn conversation with Claude: send the whole message history and get the next reply."
    )]
    async fn chat(
        &self,
        Parameters(ChatArgs {
            messages,
            system,
            model,
            max_tokens,
        }): Parameters<ChatArgs>,
    ) -> Result<CallToolResult, McpError> {
        // No invalid-role branch is needed: `ChatRole` is an enum, so a role
        // outside {user, assistant} fails deserialization and rmcp reports it to
        // the client as an invalid-params error before this body ever runs.
        let params: Vec<MessageParam> = messages
            .into_iter()
            .map(|turn| match turn.role {
                ChatRole::User => MessageParam::user(turn.content),
                ChatRole::Assistant => MessageParam::assistant(turn.content),
            })
            .collect();

        let mut builder = MessagesRequest::builder()
            .model(model.unwrap_or_else(|| CLAUDE_OPUS_5.to_string()))
            .max_tokens(max_tokens.unwrap_or(1024))
            .messages(params);
        if let Some(system) = system {
            builder = builder.system(system);
        }

        let request = match builder.build() {
            Ok(request) => request,
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

    /// Count how many input tokens a prompt would consume, without running it.
    #[tool(
        description = "Count the input tokens a prompt would consume for a given Claude model, without sending it."
    )]
    async fn count_tokens(
        &self,
        Parameters(CountTokensArgs {
            prompt,
            system,
            model,
        }): Parameters<CountTokensArgs>,
    ) -> Result<CallToolResult, McpError> {
        let model = model.unwrap_or_else(|| CLAUDE_OPUS_5.to_string());
        let mut request = CountTokensRequest::new(&model, vec![MessageParam::user(prompt)]);
        request.system = system.map(Into::into);

        match self.client.messages().count_tokens(&request).await {
            Ok(response) => Ok(CallToolResult::success(vec![ContentBlock::text(
                serde_json::json!({
                    "model": model,
                    "input_tokens": response.input_tokens,
                })
                .to_string(),
            )])),
            Err(err) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "token count failed: {err}"
            ))])),
        }
    }

    /// List the Claude models available to the configured API key.
    #[tool(description = "List the Claude models available to the configured Anthropic API key.")]
    async fn list_models(
        &self,
        Parameters(ListModelsArgs { limit }): Parameters<ListModelsArgs>,
    ) -> Result<CallToolResult, McpError> {
        let params = ModelListParams {
            limit,
            ..Default::default()
        };

        match self.client.models().list(&params).await {
            Ok(page) => {
                let models: Vec<serde_json::Value> = page
                    .data
                    .iter()
                    .map(|m| {
                        serde_json::json!({
                            "id": m.id,
                            "display_name": m.display_name,
                            "created_at": m.created_at,
                        })
                    })
                    .collect();
                match serde_json::to_string_pretty(&models) {
                    Ok(json) => Ok(CallToolResult::success(vec![ContentBlock::text(json)])),
                    Err(err) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                        "failed to serialize model list: {err}"
                    ))])),
                }
            }
            Err(err) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "model list failed: {err}"
            ))])),
        }
    }

    /// Look up one model's limits and metadata.
    #[tool(
        description = "Get a Claude model's metadata: display name, release date, context window, and max output tokens."
    )]
    async fn get_model(
        &self,
        Parameters(GetModelArgs { model }): Parameters<GetModelArgs>,
    ) -> Result<CallToolResult, McpError> {
        match self.client.models().get(&model).await {
            Ok(info) => Ok(CallToolResult::success(vec![ContentBlock::text(
                serde_json::json!({
                    "id": info.id,
                    "display_name": info.display_name,
                    "created_at": info.created_at,
                    "max_input_tokens": info.max_input_tokens,
                    "max_tokens": info.max_tokens,
                })
                .to_string(),
            )])),
            Err(err) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "model lookup failed: {err}"
            ))])),
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for ClaudeServer {
    fn get_info(&self) -> ServerInfo {
        // Build the identity from *this* crate's environment. `Implementation::
        // from_build_env()` looks like the right call but expands `env!` inside
        // rmcp, so it would report the transport crate ("rmcp") as the server
        // name rather than this server.
        let implementation = Implementation::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
            .with_title("Claude (crimson-crab)")
            .with_description(env!("CARGO_PKG_DESCRIPTION"))
            .with_website_url(env!("CARGO_PKG_REPOSITORY"));

        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(implementation)
            .with_instructions(
                "Tools backed by Anthropic's Claude API: `ask_claude` forwards a \
                 prompt to Claude and returns the reply, `chat` continues a \
                 multi-turn conversation, `count_tokens` prices a prompt without \
                 running it, `list_models` enumerates the models available to \
                 the configured API key, and `get_model` returns one model's \
                 limits and metadata."
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
