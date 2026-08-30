use crate::core::tools::mcp_auth::{parse_www_authenticate, McpOAuthState};
use crate::core::mcp::types::{McpError, TransportConfig};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

#[async_trait]
pub trait Transport: Send + Sync {
    async fn start(&self) -> Result<(), McpError>;
    async fn send(&self, message: Value) -> Result<(), McpError>;
    async fn receive(&self) -> Result<Option<Value>, McpError>;
    async fn close(&self) -> Result<(), McpError>;
}

struct StdioTransportState {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    reader: Option<BufReader<ChildStdout>>,
}

pub struct StdioTransport {
    config: TransportConfig,
    state: Arc<Mutex<StdioTransportState>>,
}

impl StdioTransport {
    pub fn new(config: &TransportConfig) -> Result<Self, McpError> {
        Ok(Self {
            config: config.clone(),
            state: Arc::new(Mutex::new(StdioTransportState {
                child: None,
                stdin: None,
                reader: None,
            })),
        })
    }
}

#[async_trait]
impl Transport for StdioTransport {
    async fn start(&self) -> Result<(), McpError> {
        let mut state = self.state.lock().await;
        if state.child.is_some() {
            return Ok(());
        }

        let cmd_str = self
            .config
            .command
            .clone()
            .ok_or("No command specified for stdio transport")?;
        let args = self.config.args.clone().unwrap_or_default();
        let env = self.config.env.clone().unwrap_or_default();

        let mut cmd = Command::new(cmd_str);
        cmd.args(args);
        cmd.envs(env);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // On Windows, we might need to set creation flags to avoid popping up windows
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Failed to spawn process: {}", e))?;
        let stdin = child.stdin.take().ok_or("Failed to open stdin")?;
        let stdout = child.stdout.take().ok_or("Failed to open stdout")?;
        let reader = BufReader::new(stdout);

        // Drain stderr into debug log so npm/npx errors don't leak to the terminal
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line).await {
                        Ok(0) => break,
                        Ok(_) => {
                            crate::utils::logging::append_debug_log_line(&format!(
                                "[MCP-stderr] {}",
                                line.trim()
                            ));
                        }
                        Err(_) => break,
                    }
                }
            });
        }

        state.child = Some(child);
        state.stdin = Some(stdin);
        state.reader = Some(reader);

        Ok(())
    }

    async fn send(&self, message: Value) -> Result<(), McpError> {
        let mut state = self.state.lock().await;
        let stdin = state.stdin.as_mut().ok_or("Not connected")?;

        let mut json_str = serde_json::to_string(&message)?;
        json_str.push('\n');

        stdin.write_all(json_str.as_bytes()).await?;
        stdin.flush().await?;

        Ok(())
    }

    async fn receive(&self) -> Result<Option<Value>, McpError> {
        let mut state = self.state.lock().await;
        let reader = state.reader.as_mut().ok_or("Not connected")?;

        let mut line = String::new();
        let bytes_read = reader.read_line(&mut line).await?;

        if bytes_read == 0 {
            return Ok(None); // EOF
        }

        let value: Value = serde_json::from_str(&line)
            .map_err(|e| format!("Failed to parse JSON: {} | Line: {}", e, line))?;
        Ok(Some(value))
    }

    async fn close(&self) -> Result<(), McpError> {
        let mut state = self.state.lock().await;
        if let Some(mut child) = state.child.take() {
            let _ = child.kill().await;
        }
        state.stdin = None;
        state.reader = None;
        Ok(())
    }
}

use futures::StreamExt;
use reqwest_eventsource::{Event, EventSource};

fn parse_sse_data(text: &str) -> Vec<Value> {
    let mut results = Vec::new();
    let mut data_parts: Vec<String> = Vec::new();
    for line in text.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            data_parts.push(data.to_string());
        } else if line.is_empty() && !data_parts.is_empty() {
            let joined = data_parts.join("");
            if let Ok(v) = serde_json::from_str::<Value>(&joined) {
                results.push(v);
            }
            data_parts.clear();
        }
    }
    if !data_parts.is_empty() {
        let joined = data_parts.join("");
        if let Ok(v) = serde_json::from_str::<Value>(&joined) {
            results.push(v);
        }
    }
    results
}

pub struct SseTransport {
    config: TransportConfig,
    client: reqwest::Client,
    event_source: Arc<Mutex<Option<EventSource>>>,
    post_endpoint: Arc<Mutex<Option<String>>>,
    oauth_state: Arc<Mutex<Option<McpOAuthState>>>,
}

impl SseTransport {
    pub fn new(config: &TransportConfig) -> Result<Self, McpError> {
        Ok(Self {
            config: config.clone(),
            client: reqwest::Client::new(),
            event_source: Arc::new(Mutex::new(None)),
            post_endpoint: Arc::new(Mutex::new(None)),
            oauth_state: Arc::new(Mutex::new(None)),
        })
    }

    pub async fn set_oauth_state(&self, state: McpOAuthState) {
        *self.oauth_state.lock().await = Some(state);
    }

    pub async fn get_oauth_state(&self) -> Option<McpOAuthState> {
        self.oauth_state.lock().await.clone()
    }

    fn build_auth_header(oauth_state: &Option<McpOAuthState>) -> Option<String> {
        oauth_state.as_ref().and_then(|s| {
            if s.has_valid_token() {
                s.access_token
                    .as_ref()
                    .map(|t| format!("Bearer {}", t))
            } else {
                None
            }
        })
    }

    async fn handle_401_response(&self, url: &str, resp: &reqwest::Response) -> McpError {
        let www_auth = resp
            .headers()
            .get("www-authenticate")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let params = parse_www_authenticate(&www_auth);

        let mut state = McpOAuthState::new(self.config.url.clone().unwrap_or_default());
        state.auth_url = params.get("authorization_uri").cloned().or_else(|| {
            if !www_auth.is_empty() {
                Some(format!(
                    "{}{}",
                    url.trim_end_matches('/'),
                    "/.well-known/oauth-authorization-server"
                ))
            } else {
                None
            }
        });
        state.token_url = params.get("token_uri").cloned();
        state.client_id = params.get("client_id").cloned();
        state.scopes = params
            .get("scope")
            .map(|s| s.split(' ').map(String::from).collect())
            .unwrap_or_default();

        *self.oauth_state.lock().await = Some(state.clone());

        format!(
            "MCP OAuth required for '{}'. Auth URL: {}",
            self.config.url.clone().unwrap_or_default(),
            state.auth_url.unwrap_or_else(|| "unknown".to_string())
        )
        .into()
    }
}

#[async_trait]
impl Transport for SseTransport {
    async fn start(&self) -> Result<(), McpError> {
        let url = self
            .config
            .url
            .as_ref()
            .ok_or("No URL specified for SSE transport")?;

        // Note: reqwest-eventsource 0.5 uses reqwest 0.11 internally.
        // Auth headers for SSE connection are passed via URL query params or
        // handled at the HTTP transport level. The POST endpoint (send) uses
        // the main reqwest 0.12 client with proper auth headers.
        let es = EventSource::get(url);

        let mut event_source_guard = self.event_source.lock().await;
        *event_source_guard = Some(es);

        Ok(())
    }

    async fn send(&self, message: Value) -> Result<(), McpError> {
        let endpoint = {
            let guard = self.post_endpoint.lock().await;
            guard.clone()
        };

        let post_url = if let Some(ep) = endpoint {
            if ep.starts_with("http") {
                ep
            } else {
                let base_url = self.config.url.as_ref().ok_or("Base URL missing")?;
                let base = reqwest::Url::parse(base_url)
                    .map_err(|e| format!("Invalid base URL: {}", e))?;
                base.join(&ep)
                    .map_err(|e| format!("Invalid endpoint URL: {}", e))?
                    .to_string()
            }
        } else {
            return Err("SSE endpoint not yet received from server".into());
        };

        let mut req = self.client.post(&post_url).json(&message);
        if let Some(auth) = Self::build_auth_header(&self.oauth_state.lock().await.clone()) {
            req = req.header(reqwest::header::AUTHORIZATION, auth);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("Failed to send POST request: {}", e))?;

        if resp.status().as_u16() == 401 {
            return Err(self.handle_401_response(&post_url, &resp).await);
        }

        if !resp.status().is_success() {
            return Err(format!("POST request failed with status: {}", resp.status()).into());
        }

        Ok(())
    }

    async fn receive(&self) -> Result<Option<Value>, McpError> {
        let mut es_guard = self.event_source.lock().await;
        let es = es_guard.as_mut().ok_or("EventSource not initialized")?;

        loop {
            let event = match es.next().await {
                Some(Ok(event)) => event,
                Some(Err(e)) => return Err(format!("SSE error: {}", e).into()),
                None => return Ok(None),
            };

            match event {
                Event::Open => continue,
                Event::Message(message) => {
                    if message.event == "endpoint" {
                        let mut ep_guard = self.post_endpoint.lock().await;
                        *ep_guard = Some(message.data);
                        continue;
                    } else if message.event == "message" {
                        let value: Value = serde_json::from_str(&message.data)
                            .map_err(|e| format!("Failed to parse SSE JSON: {}", e))?;
                        return Ok(Some(value));
                    }
                }
            }
        }
    }

    async fn close(&self) -> Result<(), McpError> {
        let mut es_guard = self.event_source.lock().await;
        if let Some(mut es) = es_guard.take() {
            es.close();
        }
        Ok(())
    }
}

pub struct StreamableHttpTransport {
    config: TransportConfig,
    client: reqwest::Client,
    session_id: Arc<Mutex<Option<String>>>,
    buffer: Arc<Mutex<VecDeque<Value>>>,
    oauth_state: Arc<Mutex<Option<McpOAuthState>>>,
}

impl StreamableHttpTransport {
    pub fn new(config: &TransportConfig) -> Result<Self, McpError> {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(hdrs) = &config.headers {
            for (k, v) in hdrs {
                let name = reqwest::header::HeaderName::from_bytes(k.as_bytes())
                    .map_err(|e| format!("Invalid header name {}: {}", k, e))?;
                let val = reqwest::header::HeaderValue::from_str(v)
                    .map_err(|e| format!("Invalid header value for {}: {}", k, e))?;
                headers.insert(name, val);
            }
        }
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .map_err(|e| format!("Failed to build HTTP client: {}", e))?;
        Ok(Self {
            config: config.clone(),
            client,
            session_id: Arc::new(Mutex::new(None)),
            buffer: Arc::new(Mutex::new(VecDeque::new())),
            oauth_state: Arc::new(Mutex::new(None)),
        })
    }

    pub async fn set_oauth_state(&self, state: McpOAuthState) {
        *self.oauth_state.lock().await = Some(state);
    }

    pub async fn get_oauth_state(&self) -> Option<McpOAuthState> {
        self.oauth_state.lock().await.clone()
    }

    fn build_auth_header(oauth_state: &Option<McpOAuthState>) -> Option<String> {
        oauth_state.as_ref().and_then(|s| {
            if s.has_valid_token() {
                s.access_token
                    .as_ref()
                    .map(|t| format!("Bearer {}", t))
            } else {
                None
            }
        })
    }

    async fn handle_401_response(&self, resp: &reqwest::Response) -> McpError {
        let url = self.config.url.clone().unwrap_or_default();
        let www_auth = resp
            .headers()
            .get("www-authenticate")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let params = parse_www_authenticate(&www_auth);

        let mut state = McpOAuthState::new(url.clone());
        state.auth_url = params.get("authorization_uri").cloned().or_else(|| {
            if !www_auth.is_empty() {
                Some(format!(
                    "{}{}",
                    url.trim_end_matches('/'),
                    "/.well-known/oauth-authorization-server"
                ))
            } else {
                None
            }
        });
        state.token_url = params.get("token_uri").cloned();
        state.client_id = params.get("client_id").cloned();
        state.scopes = params
            .get("scope")
            .map(|s| s.split(' ').map(String::from).collect())
            .unwrap_or_default();

        *self.oauth_state.lock().await = Some(state.clone());

        format!(
            "MCP OAuth required for '{}'. Auth URL: {}",
            url,
            state.auth_url.unwrap_or_else(|| "unknown".to_string())
        )
        .into()
    }
}

#[async_trait]
impl Transport for StreamableHttpTransport {
    async fn start(&self) -> Result<(), McpError> {
        Ok(())
    }

    async fn send(&self, message: Value) -> Result<(), McpError> {
        let url = self
            .config
            .url
            .as_ref()
            .ok_or("No URL specified for streamable_http transport")?;

        let session_id = self.session_id.lock().await.clone();

        let mut req = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .json(&message);

        if let Some(sid) = session_id {
            req = req.header("Mcp-Session-Id", sid);
        }

        if let Some(auth) = Self::build_auth_header(&self.oauth_state.lock().await.clone()) {
            req = req.header(reqwest::header::AUTHORIZATION, auth);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("HTTP POST failed: {}", e))?;

        if resp.status().as_u16() == 401 {
            return Err(self.handle_401_response(&resp).await);
        }

        if !resp.status().is_success() && resp.status().as_u16() != 202 {
            return Err(format!("HTTP POST failed with status: {}", resp.status()).into());
        }

        if let Some(sid_val) = resp.headers().get("Mcp-Session-Id") {
            if let Ok(sid_str) = sid_val.to_str() {
                *self.session_id.lock().await = Some(sid_str.to_string());
            }
        }

        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let mut buf = self.buffer.lock().await;
        if content_type.contains("text/event-stream") {
            let text = resp
                .text()
                .await
                .map_err(|e| format!("Failed to read SSE body: {}", e))?;
            for v in parse_sse_data(&text) {
                buf.push_back(v);
            }
        } else {
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| format!("Failed to read response body: {}", e))?;
            if !bytes.is_empty() {
                let v: Value = serde_json::from_slice(&bytes)
                    .map_err(|e| format!("Failed to parse JSON response: {}", e))?;
                buf.push_back(v);
            }
        }

        Ok(())
    }

    async fn receive(&self) -> Result<Option<Value>, McpError> {
        let mut buf = self.buffer.lock().await;
        Ok(buf.pop_front())
    }

    async fn close(&self) -> Result<(), McpError> {
        *self.session_id.lock().await = None;
        self.buffer.lock().await.clear();
        Ok(())
    }
}
