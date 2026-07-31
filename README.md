# Claude-powered MCP server in Rust (starter template)

[![crimson-crab-mcp-template MCP server](https://glama.ai/mcp/servers/singhpratech/crimson-crab-mcp-template/badges/score.svg)](https://glama.ai/mcp/servers/singhpratech/crimson-crab-mcp-template)
[![crates.io](https://img.shields.io/crates/v/crimson-crab.svg?label=crimson-crab)](https://crates.io/crates/crimson-crab)
[![license](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](#license)

A minimal, production-ready [Model Context Protocol](https://modelcontextprotocol.io)
(MCP) server, written in Rust, backed by Anthropic's Claude API through the
[`crimson-crab`](https://crates.io/crates/crimson-crab) SDK. It exposes five
tools: `ask_claude` (send a prompt, get Claude's reply), `chat` (multi-turn
conversation), `count_tokens` (price a prompt without running it), `list_models`
(enumerate the models your API key can use), and `get_model` (limits and
metadata for one model).

Use it as the reference starting point for building your own Claude-powered MCP
tools in Rust.

## Use this template

Click **Use this template** on GitHub, or generate a fresh project with
[`cargo generate`](https://github.com/cargo-generate/cargo-generate):

```sh
cargo generate --git https://github.com/singhpratech/crimson-crab-mcp-template
```

You can also just clone it:

```sh
git clone https://github.com/singhpratech/crimson-crab-mcp-template
```

## Quickstart

```sh
export ANTHROPIC_API_KEY=sk-ant-...
cargo run
```

The server communicates over stdio, so running it directly just waits for an MCP
client to connect. All logging goes to **stderr** — stdout is reserved for the MCP
protocol. Set `RUST_LOG=debug` for more verbose logs.

To build an optimized binary you can point a client at:

```sh
cargo build --release
# -> target/release/crimson-crab-mcp-template
```

## Wire it into Claude Desktop

Add an entry to your Claude Desktop MCP config
(`claude_desktop_config.json`), pointing at the built binary and passing your API
key through the environment:

```json
{
  "mcpServers": {
    "claude-via-crimson-crab": {
      "command": "/absolute/path/to/target/release/crimson-crab-mcp-template",
      "env": {
        "ANTHROPIC_API_KEY": "sk-ant-..."
      }
    }
  }
}
```

Restart Claude Desktop; the `ask_claude`, `chat`, `count_tokens`,
`list_models`, and `get_model` tools will then be available.

## The tools

### `ask_claude`

| Parameter | Type              | Required | Description                        |
| --------- | ----------------- | -------- | ---------------------------------- |
| `prompt`  | `string`          | yes      | The prompt to send to Claude.      |
| `system`  | `string`          | no       | Optional system prompt.            |

Builds a Claude Messages request (defaulting to the `claude-opus-5` model),
calls the API, and returns the concatenated text of Claude's reply.

### `chat`

| Parameter    | Type       | Required | Description                                              |
| ------------ | ---------- | -------- | -------------------------------------------------------- |
| `messages`   | `array`    | yes      | The conversation so far, oldest first: `{role, content}` where `role` is an enum constrained to `"user"` or `"assistant"`. Must end with a user turn. |
| `system`     | `string`   | no       | Optional system prompt.                                  |
| `model`      | `string`   | no       | Model id (defaults to the `ask_claude` model).           |
| `max_tokens` | `integer`  | no       | Maximum tokens to generate (default 1024).               |

Sends the whole message history to Claude and returns the next reply — use it
instead of `ask_claude` when the caller needs to keep context across turns.

### `count_tokens`

| Parameter | Type              | Required | Description                                        |
| --------- | ----------------- | -------- | -------------------------------------------------- |
| `prompt`  | `string`          | yes      | The prompt whose token count you want.             |
| `system`  | `string`          | no       | Optional system prompt to include in the count.    |
| `model`   | `string`          | no       | Model id to count against (counts are model-specific). |

Returns the number of input tokens the prompt would consume, without running
it — useful for estimating cost before an expensive call.

### `list_models`

| Parameter | Type              | Required | Description                                    |
| --------- | ----------------- | -------- | ---------------------------------------------- |
| `limit`   | `integer`         | no       | Maximum number of models to return.            |

Returns the id, display name, and release date of each Claude model available
to the configured API key.

### `get_model`

| Parameter | Type              | Required | Description                                    |
| --------- | ----------------- | -------- | ---------------------------------------------- |
| `model`   | `string`          | yes      | The model id to look up, e.g. `claude-opus-5`. |

Returns the model's display name, release date, context window
(`max_input_tokens`), and maximum output tokens.

Errors from all tools are returned as MCP tool errors rather than panicking.
The Claude client is built once at startup and reused across calls.

## Built with crimson-crab

This template is built with [`crimson-crab`](https://crates.io/crates/crimson-crab)
— a production-grade Rust SDK for Anthropic's Claude API. Source:
<https://github.com/singhpratech/crimson-crab>.

## License

Licensed under either of

- MIT license ([LICENSE-MIT](LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

at your option.

---

crimson-crab is an independent open-source project and is not affiliated with Anthropic.
