//! End-to-end protocol tests: spawn the real compiled binary and talk MCP to it
//! over stdin/stdout, exactly the way an MCP client would.
//!
//! These tests never reach Anthropic. `initialize` and `tools/list` are pure
//! introspection, and the one `tools/call` we make is rejected during argument
//! deserialization — before any HTTP request could happen. A dummy API key is
//! therefore enough to boot the server, and the suite stays hermetic and fast.
//!
//! Dependency-light on purpose: `std::process` plus `serde_json` (already a
//! dependency of the binary, so it is available to test targets too).

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

/// Protocol revision we negotiate. rmcp 2.2 supports this one.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// The five tools this template is contracted to expose.
const EXPECTED_TOOLS: [&str; 5] = [
    "ask_claude",
    "chat",
    "count_tokens",
    "get_model",
    "list_models",
];

/// Hard upper bound on a single test. A protocol bug can leave us blocked in
/// `read_line` forever, which would hang CI rather than fail it; the watchdog
/// turns that into a killed process and a failed read.
const WATCHDOG: Duration = Duration::from_secs(60);

/// A running server process plus its protocol pipes.
struct Server {
    child: Arc<Mutex<Child>>,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    finished: Arc<AtomicBool>,
    next_id: i64,
}

impl Server {
    /// Spawn the compiled binary. Cargo hands integration tests the path to it
    /// via `CARGO_BIN_EXE_<name>`, so this always tests the artifact that was
    /// just built rather than a stale one on `$PATH`.
    fn spawn() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_crimson-crab-mcp-template"))
            .env("ANTHROPIC_API_KEY", "sk-ant-dummy")
            // Keep the server quiet; stdout is the protocol channel and stderr
            // is only logging, which would otherwise interleave with test output.
            .env("RUST_LOG", "error")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn MCP server binary");

        let stdin = child.stdin.take().expect("stdin was piped");
        let stdout = child.stdout.take().expect("stdout was piped");

        let child = Arc::new(Mutex::new(child));
        let finished = Arc::new(AtomicBool::new(false));

        // Watchdog: poll rather than sleep the whole duration, so a passing test
        // does not linger for a minute waiting on this thread.
        {
            let child = Arc::clone(&child);
            let finished = Arc::clone(&finished);
            std::thread::spawn(move || {
                let deadline = std::time::Instant::now() + WATCHDOG;
                while std::time::Instant::now() < deadline {
                    if finished.load(Ordering::SeqCst) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                let _ = child.lock().expect("watchdog lock").kill();
            });
        }

        Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            finished,
            next_id: 0,
        }
    }

    /// Write one newline-delimited JSON-RPC message.
    fn send(&mut self, message: &Value) {
        let line = serde_json::to_string(message).expect("serialize request");
        writeln!(self.stdin, "{line}").expect("write to server stdin");
        self.stdin.flush().expect("flush server stdin");
    }

    /// Read one newline-delimited JSON-RPC message.
    fn recv(&mut self) -> Value {
        let mut line = String::new();
        let read = self
            .stdout
            .read_line(&mut line)
            .expect("read from server stdout");
        assert!(
            read > 0,
            "server closed stdout without replying — it most likely crashed"
        );
        serde_json::from_str(&line).unwrap_or_else(|err| panic!("invalid JSON {line:?}: {err}"))
    }

    /// Send a request and return the matching response.
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }));

        let response = self.recv();
        assert_eq!(
            response["id"],
            json!(id),
            "response id did not match request id for {method}"
        );
        response
    }

    /// Send a notification (no response expected).
    fn notify(&mut self, method: &str) {
        self.send(&json!({ "jsonrpc": "2.0", "method": method }));
    }

    /// Perform the MCP opening handshake and return the `initialize` result.
    fn handshake(&mut self) -> Value {
        let response = self.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "protocol-test", "version": "0.0.0" },
            }),
        );
        let result = response["result"].clone();
        assert!(!result.is_null(), "initialize failed: {response}");

        self.notify("notifications/initialized");
        result
    }

    /// `tools/list`, returned as a name -> tool map.
    fn list_tools(&mut self) -> serde_json::Map<String, Value> {
        let response = self.request("tools/list", json!({}));
        let tools = response["result"]["tools"]
            .as_array()
            .unwrap_or_else(|| panic!("tools/list returned no tool array: {response}"))
            .clone();

        tools
            .into_iter()
            .map(|tool| {
                let name = tool["name"]
                    .as_str()
                    .expect("every tool has a name")
                    .to_string();
                (name, tool)
            })
            .collect()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.finished.store(true, Ordering::SeqCst);
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Follow a `$ref` like `#/$defs/ChatTurn` within the same document.
fn resolve<'a>(root: &'a Value, schema: &'a Value) -> &'a Value {
    let Some(reference) = schema.get("$ref").and_then(Value::as_str) else {
        return schema;
    };
    let mut current = root;
    for segment in reference.trim_start_matches("#/").split('/') {
        current = current
            .get(segment)
            .unwrap_or_else(|| panic!("dangling $ref {reference:?} in schema"));
    }
    current
}

#[test]
fn server_advertises_its_identity_and_exactly_five_tools() {
    let mut server = Server::spawn();
    let init = server.handshake();

    assert_eq!(
        init["serverInfo"]["name"],
        json!("crimson-crab-mcp-template"),
        "serverInfo.name must identify this crate, not the rmcp transport crate"
    );

    let tools = server.list_tools();
    let mut names: Vec<&str> = tools.keys().map(String::as_str).collect();
    names.sort_unstable();

    assert_eq!(names, EXPECTED_TOOLS, "unexpected tool set");
    assert_eq!(tools.len(), 5, "expected exactly five tools");
}

#[test]
fn chat_role_schema_carries_a_machine_readable_enum() {
    let mut server = Server::spawn();
    server.handshake();
    let tools = server.list_tools();

    let schema = &tools["chat"]["inputSchema"];
    let messages = &schema["properties"]["messages"];
    assert_eq!(
        messages["type"],
        json!("array"),
        "messages must be an array"
    );

    let turn = resolve(schema, &messages["items"]);
    let role = resolve(schema, &turn["properties"]["role"]);

    assert_eq!(
        role["enum"],
        json!(["user", "assistant"]),
        "role must expose its allowed values as a JSON Schema enum, got: {role}"
    );

    // The other half of fix #3: the wire schema says these are integers, and the
    // README's tool tables have to agree with it.
    assert!(
        schema["properties"]["max_tokens"]["type"]
            .as_array()
            .expect("max_tokens is a nullable type union")
            .contains(&json!("integer")),
        "chat.max_tokens should be an integer, got: {}",
        schema["properties"]["max_tokens"]
    );
    let limit = &tools["list_models"]["inputSchema"]["properties"]["limit"];
    assert!(
        limit["type"]
            .as_array()
            .expect("limit is a nullable type union")
            .contains(&json!("integer")),
        "list_models.limit should be an integer, got: {limit}"
    );
}

#[test]
fn invalid_chat_role_is_an_error_not_a_crash() {
    let mut server = Server::spawn();
    server.handshake();

    let response = server.request(
        "tools/call",
        json!({
            "name": "chat",
            "arguments": {
                "messages": [{ "role": "system", "content": "you are a bird" }],
            },
        }),
    );

    // rmcp rejects the bad role while deserializing arguments, which surfaces as
    // a JSON-RPC error. Accept a tool-level `isError` result too, so the test
    // pins the observable contract ("the caller is told it was wrong") rather
    // than rmcp's current choice of channel.
    let error = &response["error"];
    let is_tool_error = response["result"]["isError"] == json!(true);
    assert!(
        !error.is_null() || is_tool_error,
        "an invalid role must be reported as an error, got: {response}"
    );

    // Whichever channel it arrives on, the message has to tell a client what the
    // valid roles are — that is what the old hand-written check used to do.
    let message = if error.is_null() {
        response["result"]["content"].to_string()
    } else {
        error["message"].as_str().unwrap_or_default().to_string()
    };
    assert!(
        message.contains("user") && message.contains("assistant"),
        "error should name the allowed roles, got: {message}"
    );

    // The whole point: the server survived the bad input and still serves.
    let tools = server.list_tools();
    assert_eq!(
        tools.len(),
        5,
        "server must still answer tools/list after a rejected call"
    );
}
