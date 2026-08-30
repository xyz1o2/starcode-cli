// ── MCP Context Engine Server ──────────────────────────────────────────────────
//
// P4 improvement (docs/context-engine-ace-benchmark.zh-CN.md):
// Exposes StarCode CLI's context engine as an MCP server so external agents
// (Claude Code, Cursor, Codex, etc.) can use semantic search and call chain
// tracing via the MCP protocol.
//
// Architecture:
//   stdin/stdout  ←→  JSON-RPC 2.0  ←→  MCP protocol  ←→  Context Engine
//
// Supported MCP methods:
//   initialize          — protocol handshake
//   tools/list          — discover available tools
//   tools/call          — invoke a tool
//   ping                — health check

use crate::core::tools::semantic_search;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::Arc;

/// Server info exposed during initialization.
const SERVER_NAME: &str = "starcode-context-engine";
const SERVER_VERSION: &str = "0.1.0";
const PROTOCOL_VERSION: &str = "2024-11-05";

/// Run the MCP context engine server using stdio transport.
///
/// Reads JSON-RPC requests line-by-line from stdin and writes responses
/// to stdout. Log messages go to stderr so they don't interfere with
/// the JSON-RPC stream.
pub async fn run_stdio_server() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut stdout = std::io::stdout();

    eprintln!("[starcode-mcp] Context engine MCP server starting (stdio)");

    let mut initialized = false;

    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(32);
    let (err_tx, mut err_rx) = tokio::sync::mpsc::channel::<String>(4);

    // Spawn a dedicated thread with a large stack to read stdin.
    // Tokio's built-in stdin uses blocking reads internally which can cause
    // stack overflow on tokio worker threads. We use our own thread instead.
    std::thread::Builder::new()
        .name("mcp-stdin-reader".into())
        .stack_size(16 * 1024 * 1024) // 16 MiB stack to prevent overflow
        .spawn(move || {
            let stdin = std::io::stdin();
            let reader = BufReader::new(stdin.lock());
            for line in reader.lines() {
                match line {
                    Ok(l) => {
                        if tx.blocking_send(l).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = err_tx.blocking_send(format!("stdin read error: {}", e));
                        break;
                    }
                }
            }
        })
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;

    // Process messages on the async Tokio runtime
    loop {
        tokio::select! {
            line = rx.recv() => {
                match line {
                    Some(line) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }

                        let request: serde_json::Value = match serde_json::from_str(trimmed) {
                            Ok(v) => v,
                            Err(e) => {
                                eprintln!("[starcode-mcp] JSON parse error: {}", e);
                                continue;
                            }
                        };

                        let method = request
                            .get("method")
                            .and_then(|m| m.as_str())
                            .unwrap_or("");

                        let id = request.get("id").cloned();

                        // Only respond if the request has an id (requests without id are notifications)
                        let response = match method {
                            "initialize" => {
                                if !initialized {
                                    initialized = true;
                                    Some(build_response(id, &serde_json::json!({
                                        "protocolVersion": PROTOCOL_VERSION,
                                        "serverInfo": {
                                            "name": SERVER_NAME,
                                            "version": SERVER_VERSION
                                        },
                                        "capabilities": {
                                            "tools": {}
                                        }
                                    })))
                                } else {
                                    Some(build_error(
                                        id,
                                        -32000,
                                        "Server already initialized",
                                    ))
                                }
                            }
                            "ping" => Some(build_response(id, &serde_json::json!({}))),
                            "tools/list" => {
                                if !initialized {
                                    Some(build_error(
                                        id,
                                        -32002,
                                        "Not initialized",
                                    ))
                                } else {
                                    Some(build_response(
                                        id,
                                        &serde_json::json!({
                                            "tools": [
                                                {
                                                    "name": "search_code",
                                                    "description": "Search the codebase semantically using hybrid keyword search with RRF fusion and heuristic reranking. Returns the most relevant code snippets for a natural-language query. Supports 10+ languages (Rust, Python, JS/TS, Go, C/C++, Java).",
                                                    "inputSchema": {
                                                        "type": "object",
                                                        "properties": {
                                                            "query": {
                                                                "type": "string",
                                                                "description": "Natural language query describing what code to find"
                                                            },
                                                            "root_path": {
                                                                "type": "string",
                                                                "description": "Root directory of the project to search (default: current working directory)"
                                                            }
                                                        },
                                                        "required": ["query"]
                                                    }
                                                },
                                                {
                                                    "name": "trace_call_chain",
                                                    "description": "Trace the call chain for a function/method/class. Resolves cross-file call relationships using Tree-sitter AST analysis. Shows who calls a symbol (callers) and what it calls (callees), up to 3 levels deep.",
                                                    "inputSchema": {
                                                        "type": "object",
                                                        "properties": {
                                                            "name_hint": {
                                                                "type": "string",
                                                                "description": "Partial or full name of the function/method to trace (case-insensitive substring match)"
                                                            },
                                                            "root_path": {
                                                                "type": "string",
                                                                "description": "Root directory of the project (default: current working directory)"
                                                            }
                                                        },
                                                        "required": ["name_hint", "root_path"]
                                                    }
                                                }
                                            ]
                                        }),
                                    ))
                                }
                            }
                            "tools/call" => {
                                if !initialized {
                                    Some(build_error(id, -32002, "Not initialized"))
                                } else {
                                    let params = request.get("params");
                                    let tool_name = params
                                        .and_then(|p| p.get("name"))
                                        .and_then(|n| n.as_str())
                                        .unwrap_or("");
                                    let arguments = params
                                        .and_then(|p| p.get("arguments"))
                                        .cloned()
                                        .unwrap_or(serde_json::json!({}));

                                    Some(handle_tool_call(id, tool_name, &arguments).await)
                                }
                            }
                            "notifications/initialized" => {
                                // Client notification, no response needed
                                None
                            }
                            _ => Some(build_error(
                                id,
                                -32601,
                                &format!("Method not found: {}", method),
                            )),
                        };

                        if let Some(resp) = response {
                            let resp_str = serde_json::to_string(&resp).unwrap_or_default();
                            writeln!(stdout, "{}", resp_str)?;
                            stdout.flush()?;
                        }
                    }
                    None => break, // Channel closed, stdin EOF
                }
            }
            err = err_rx.recv() => {
                if let Some(msg) = err {
                    eprintln!("[starcode-mcp] {}", msg);
                }
                break;
            }
        }
    }

    eprintln!("[starcode-mcp] Server shutting down");
    Ok(())
}

/// Handle a tools/call request and return the JSON-RPC response.
async fn handle_tool_call(
    id: Option<serde_json::Value>,
    tool_name: &str,
    arguments: &serde_json::Value,
) -> serde_json::Value {
    match tool_name {
        "search_code" => {
            let query = arguments
                .get("query")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if query.is_empty() {
                return build_error(id, -32602, "Missing required parameter: query");
            }

            let root = get_root_path(arguments);
            let query = query.to_string();
            match tokio::task::spawn_blocking(move || {
                let no_progress: Option<Arc<dyn Fn(String) + Send + Sync>> = None;
                semantic_search::search_codebase(&root, &query, no_progress)
            })
            .await
            {
                Ok(Ok(output)) => build_response(
                    id,
                    &serde_json::json!({
                        "content": [
                            {
                                "type": "text",
                                "text": output
                            }
                        ]
                    }),
                ),
                Ok(Err(e)) => build_error(id, -32000, &format!("Search failed: {}", e)),
                Err(e) => build_error(id, -32000, &format!("Task join error: {}", e)),
            }
        }
        "trace_call_chain" => {
            let name_hint = arguments
                .get("name_hint")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if name_hint.is_empty() {
                return build_error(id, -32602, "Missing required parameter: name_hint");
            }

            let root = get_root_path(arguments);
            let name_hint = name_hint.to_string();
            match tokio::task::spawn_blocking(move || {
                semantic_search::trace_call_chain(&root, &name_hint)
            })
            .await
            {
                Ok(Ok(output)) => build_response(
                    id,
                    &serde_json::json!({
                        "content": [
                            {
                                "type": "text",
                                "text": output
                            }
                        ]
                    }),
                ),
                Ok(Err(e)) => build_error(id, -32000, &format!("Call chain trace failed: {}", e)),
                Err(e) => build_error(id, -32000, &format!("Task join error: {}", e)),
            }
        }
        _ => build_error(id, -32601, &format!("Unknown tool: {}", tool_name)),
    }
}

/// Extract the root path from tool arguments, defaulting to cwd.
fn get_root_path(arguments: &serde_json::Value) -> PathBuf {
    arguments
        .get("root_path")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

/// Build a successful JSON-RPC response.
fn build_response(id: Option<serde_json::Value>, result: &serde_json::Value) -> serde_json::Value {
    let mut resp = serde_json::json!({
        "jsonrpc": "2.0",
        "result": result
    });
    if let Some(id) = id {
        resp["id"] = id;
    }
    resp
}

/// Build a JSON-RPC error response.
fn build_error(
    id: Option<serde_json::Value>,
    code: i64,
    message: &str,
) -> serde_json::Value {
    let mut resp = serde_json::json!({
        "jsonrpc": "2.0",
        "error": {
            "code": code,
            "message": message
        }
    });
    if let Some(id) = id {
        resp["id"] = id;
    }
    resp
}
