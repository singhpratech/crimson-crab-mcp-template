# Claude-powered MCP server in Rust (starter template)

A minimal, production-ready [Model Context Protocol](https://modelcontextprotocol.io)
(MCP) server, written in Rust, that exposes a single tool — `ask_claude` — backed
by Anthropic's Claude API through the [`crimson-crab`](https://crates.io/crates/crimson-crab)
SDK. When an MCP client (such as Claude Desktop) calls the tool, the server sends
the prompt to Claude and returns Claude's text answer.

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

Restart Claude Desktop; the `ask_claude` tool will then be available.

## What `ask_claude` does

| Parameter | Type              | Required | Description                        |
| --------- | ----------------- | -------- | ---------------------------------- |
| `prompt`  | `string`          | yes      | The prompt to send to Claude.      |
| `system`  | `string`          | no       | Optional system prompt.            |

The tool builds a Claude Messages request (defaulting to the `claude-opus-4-8`
model), calls the API, and returns the concatenated text of Claude's reply. Errors
are returned as MCP tool errors rather than panicking. The Claude client is built
once at startup and reused across calls.

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
