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

pub(crate) struct McpFixture {
    pub(crate) config: MCPServerConfig,
    requests: Arc<Mutex<Vec<Value>>>,
    side_effects: Arc<Mutex<Vec<Value>>>,
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

impl McpFixture {
    pub(crate) async fn start(reply: ToolReply) -> Self {
        Self::start_with_side_effects(reply, Arc::new(Mutex::new(Vec::new()))).await
    }

    pub(crate) async fn start_with_side_effects(
        reply: ToolReply,
        side_effects: Arc<Mutex<Vec<Value>>>,
    ) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind MCP loopback fixture");
        let address = listener.local_addr().expect("fixture address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let cancel = CancellationToken::new();
        let shutdown = cancel.clone();
        let observed_side_effects = side_effects.clone();
        let task_side_effects = observed_side_effects.clone();
        let task = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    connection = listener.accept() => {
                        let (socket, _) = connection.expect("accept MCP fixture connection");
                        let requests = captured.clone();
                        let side_effects = task_side_effects.clone();
                        let reply = reply.clone();
                        connections.spawn(async move {
                            let service = hyper::service::service_fn(
                                move |request: Request<hyper::body::Incoming>| {
                                    let requests = requests.clone();
                                    let side_effects = side_effects.clone();
                                    let reply = reply.clone();
                                    async move {
                                        let body = request.into_body().collect().await
                                            .map_err(std::io::Error::other)?.to_bytes();
                                        let message: Value = serde_json::from_slice(&body)
                                            .expect("fixture request is JSON");
                                        requests.lock().await.push(message.clone());
                                        let id = message.get("id").cloned();
                                        let response = match message["method"].as_str() {
                                            Some("initialize") => json!({
                                                "protocolVersion": "2024-11-05",
                                                "capabilities": {},
                                                "serverInfo": {"name": "fixture", "version": "1"}
                                            }),
                                            Some("tools/list") => json!({"tools": [{
                                                "name": "inspect",
                                                "description": "Local regression fixture",
                                                "inputSchema": {"type": "object"}
                                            }]}),
                                            Some("tools/call") => {
                                                side_effects.lock().await.push(message["params"]["arguments"].clone());
                                                match reply {
                                                ToolReply::Result(value) => value,
                                                ToolReply::ProtocolError(error) => {
                                                    return Ok::<_, std::io::Error>(
                                                        Response::builder()
                                                            .header("Content-Type", "application/json")
                                                            .body(Full::new(Bytes::from(json!({
                                                                "jsonrpc": "2.0", "id": id,
                                                                "error": error
                                                            }).to_string())))
                                                            .expect("fixture protocol response"),
                                                    );
                                                }
                                                ToolReply::ResponseLost => {
                                                    // 请求已经记录，随后断开响应，模拟副作用已完成的未知结果。
                                                    return Err(std::io::Error::new(
                                                        std::io::ErrorKind::ConnectionReset,
                                                        "fixture dropped tool response",
                                                    ));
                                                }
                                                }
                                            },
                                            Some("notifications/initialized") => {
                                                return Ok(Response::builder()
                                                    .status(202)
                                                    .body(Full::new(Bytes::new()))
                                                    .expect("fixture notification response"));
                                            }
                                            method => panic!("unexpected fixture method: {method:?}"),
                                        };
                                        Ok(Response::builder()
                                            .header("Content-Type", "application/json")
                                            .body(Full::new(Bytes::from(json!({
                                                "jsonrpc": "2.0", "id": id, "result": response
                                            }).to_string())))
                                            .expect("fixture result response"))
                                    }
                                },
                            );
                            // ResponseLost 刻意关闭连接；客户端断开也不构成夹具失败。
                            let _ = hyper::server::conn::http1::Builder::new()
                                .serve_connection(hyper_util::rt::TokioIo::new(socket), service)
                                .await;
                        });
                    }
                    Some(result) = connections.join_next(), if !connections.is_empty() => {
                        result.expect("MCP fixture connection task panicked");
                    }
                }
            }
            connections.abort_all();
            while connections.join_next().await.is_some() {}
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
            requests,
            side_effects: observed_side_effects,
            cancel,
            task,
        }
    }

    pub(crate) async fn requests(&self) -> Vec<Value> {
        self.requests.lock().await.clone()
    }

    pub(crate) async fn side_effects(&self) -> Vec<Value> {
        self.side_effects.lock().await.clone()
    }

    pub(crate) async fn shutdown(self) {
        self.cancel.cancel();
        self.task.await.expect("MCP fixture task panicked");
    }
}
