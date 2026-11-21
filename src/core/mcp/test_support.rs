//! 仅监听回环地址的 MCP 回归夹具，记录实际 JSON-RPC 请求，不接触真实服务。

use super::types::{MCPServerConfig, TransportConfig};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Request, Response};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub(crate) enum ToolReply {
    Result(Value),
    ProtocolError(Value),
    ResponseLost,
}

#[derive(Default)]
struct Observed {
    requests: Mutex<Vec<Value>>,
    side_effects: Mutex<Vec<Value>>,
    connection_errors: Mutex<Vec<String>>,
}

pub(crate) struct McpFixture {
    pub(crate) config: MCPServerConfig,
    observed: Arc<Observed>,
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

fn json_response(value: Value) -> Response<Full<Bytes>> {
    Response::builder()
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(value.to_string())))
        .expect("fixture JSON response")
}

async fn respond(
    request: Request<hyper::body::Incoming>,
    observed: Arc<Observed>,
    reply: ToolReply,
) -> Result<Response<Full<Bytes>>, std::io::Error> {
    let body = request
        .into_body()
        .collect()
        .await
        .map_err(std::io::Error::other)?
        .to_bytes();
    let message: Value = serde_json::from_slice(&body).expect("fixture request is JSON");
    observed.requests.lock().await.push(message.clone());
    let id = message.get("id").cloned();
    let result = match message["method"].as_str() {
        Some("initialize") => json!({
            "protocolVersion": "2024-11-05", "capabilities": {},
            "serverInfo": {"name": "fixture", "version": "1"}
        }),
        Some("tools/list") => json!({"tools": [{
            "name": "inspect", "description": "Local regression fixture",
            "inputSchema": {"type": "object"}
        }]}),
        Some("notifications/initialized") => {
            return Ok(Response::builder()
                .status(202)
                .body(Full::new(Bytes::new()))
                .expect("fixture notification response"));
        }
        Some("tools/call") => match reply {
            ToolReply::Result(value) => value,
            ToolReply::ProtocolError(error) => {
                return Ok(json_response(
                    json!({"jsonrpc": "2.0", "id": id, "error": error}),
                ));
            }
            ToolReply::ResponseLost => {
                // 先记下已执行的模拟副作用，再丢弃响应，不能靠重发恢复未知结果。
                observed
                    .side_effects
                    .lock()
                    .await
                    .push(message["params"]["arguments"].clone());
                return Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    "fixture dropped tool response",
                ));
            }
        },
        method => panic!("unexpected fixture method: {method:?}"),
    };
    Ok(json_response(
        json!({"jsonrpc": "2.0", "id": id, "result": result}),
    ))
}

impl McpFixture {
    pub(crate) async fn start(reply: ToolReply) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind MCP loopback fixture");
        let address = listener.local_addr().expect("fixture address");
        let observed = Arc::new(Observed::default());
        let captured = observed.clone();
        let cancel = CancellationToken::new();
        let shutdown = cancel.clone();
        let task = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    connection = listener.accept() => {
                        let (socket, _) = connection.expect("accept MCP fixture connection");
                        let observed = captured.clone();
                        let reply = reply.clone();
                        connections.spawn(async move {
                            let requests = observed.clone();
                            let service = hyper::service::service_fn(move |request| {
                                respond(request, requests.clone(), reply.clone())
                            });
                            if let Err(error) = hyper::server::conn::http1::Builder::new()
                                .serve_connection(hyper_util::rt::TokioIo::new(socket), service)
                                .await
                            {
                                observed.connection_errors.lock().await.push(error.to_string());
                            }
                        });
                    }
                    Some(result) = connections.join_next(), if !connections.is_empty() => {
                        result.expect("MCP fixture connection task panicked");
                    }
                }
            }
            connections.abort_all();
            while let Some(result) = connections.join_next().await {
                if let Err(error) = result {
                    assert!(
                        error.is_cancelled(),
                        "MCP fixture connection task panicked: {error}"
                    );
                }
            }
        });
        let config = MCPServerConfig {
            name: "local-regression".to_string(),
            transport: TransportConfig {
                transport_type: "http".to_string(),
                command: None,
                args: None,
                env: None,
                url: Some(format!("http://{address}/mcp")),
                headers: None,
            },
            command: None,
            args: None,
            env: None,
            disabled: None,
        };
        Self {
            config,
            observed,
            cancel,
            task,
        }
    }

    pub(crate) async fn requests(&self) -> Vec<Value> {
        self.observed.requests.lock().await.clone()
    }

    pub(crate) async fn side_effects(&self) -> Vec<Value> {
        self.observed.side_effects.lock().await.clone()
    }

    pub(crate) async fn diagnostics(&self) -> String {
        format!(
            "requests={:?}; connection_errors={:?}",
            self.requests().await,
            self.observed.connection_errors.lock().await
        )
    }

    pub(crate) async fn shutdown(self) {
        self.cancel.cancel();
        self.task.await.expect("MCP fixture task panicked");
    }
}
