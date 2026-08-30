//! Network proxy for sandbox isolation
//!
//! Implements HTTP/SOCKS proxy with domain allowlist filtering,
//! similar to StarCode's sandbox runtime.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;

use super::config::NetworkConfig;

/// Domain matcher supporting wildcards
#[derive(Debug, Clone)]
pub struct DomainMatcher {
    /// Exact matches
    exact: HashSet<String>,
    /// Wildcard patterns (*.example.com)
    wildcards: Vec<String>,
}

impl DomainMatcher {
    pub fn new(domains: Vec<String>) -> Self {
        let mut exact = HashSet::new();
        let mut wildcards = Vec::new();

        for domain in domains {
            let domain = domain.to_lowercase();
            if let Some(suffix) = domain.strip_prefix("*.") {
                // Store without the *. prefix for suffix matching
                wildcards.push(suffix.to_string());
            } else {
                exact.insert(domain);
            }
        }

        Self { exact, wildcards }
    }

    /// Check if a domain matches any rule
    pub fn matches(&self, domain: &str) -> bool {
        let domain = domain.to_lowercase();

        // Check exact match
        if self.exact.contains(&domain) {
            return true;
        }

        // Check wildcard patterns
        for suffix in &self.wildcards {
            if domain.ends_with(&format!(".{}", suffix)) || domain == *suffix {
                return true;
            }
        }

        false
    }
}

/// Network proxy for sandbox
pub struct NetworkProxy {
    /// Proxy listen address
    listen_addr: SocketAddr,
    /// Allowed domains
    allowed: DomainMatcher,
    /// Denied domains
    denied: DomainMatcher,
    /// Default action (true = allow)
    default_action: bool,
    /// Running state
    running: Arc<RwLock<bool>>,
}

impl NetworkProxy {
    /// Create a new network proxy
    pub fn new(config: &NetworkConfig, port: u16) -> Self {
        Self {
            listen_addr: SocketAddr::from(([127, 0, 0, 1], port)),
            allowed: DomainMatcher::new(config.allowed_domains.clone()),
            denied: DomainMatcher::new(config.denied_domains.clone()),
            default_action: config.default_action,
            running: Arc::new(RwLock::new(false)),
        }
    }

    /// Start the proxy server
    pub async fn start(&self) -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind(self.listen_addr).await?;

        {
            let mut running = self.running.write().await;
            *running = true;
        }

        let running = self.running.clone();
        let allowed = self.allowed.clone();
        let denied = self.denied.clone();
        let default_action = self.default_action;

        tokio::spawn(async move {
            loop {
                // Check if we should stop
                {
                    let running = running.read().await;
                    if !*running {
                        break;
                    }
                }

                // Accept new connections
                let accept_result = listener.accept().await;
                if let Ok((stream, _)) = accept_result {
                    let allowed = allowed.clone();
                    let denied = denied.clone();
                    let default_action = default_action;

                    tokio::spawn(async move {
                        if let Err(e) =
                            handle_connection(stream, allowed, denied, default_action).await
                        {
                            tracing::debug!("Connection error: {}", e);
                        }
                    });
                }
            }
        });

        Ok(())
    }

    /// Stop the proxy server
    pub async fn stop(&self) {
        let mut running = self.running.write().await;
        *running = false;
    }

    /// Get the proxy address
    pub fn address(&self) -> SocketAddr {
        self.listen_addr
    }

    /// Get HTTP proxy URL
    pub fn http_proxy_url(&self) -> String {
        format!("http://{}", self.listen_addr)
    }

    /// Get SOCKS proxy URL
    pub fn socks_proxy_url(&self) -> String {
        format!("socks5://{}", self.listen_addr)
    }
}

/// Handle a single proxy connection
async fn handle_connection(
    mut stream: TcpStream,
    allowed: DomainMatcher,
    denied: DomainMatcher,
    default_action: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Read HTTP CONNECT request
    let mut reader = BufReader::new(&mut stream);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).await?;

    // Parse CONNECT request
    let parts: Vec<&str> = request_line.trim().split_whitespace().collect();
    if parts.len() < 2 || parts[0] != "CONNECT" {
        stream
            .write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n")
            .await?;
        return Ok(());
    }

    let target = parts[1];
    let (host, port) = parse_host_port(target)?;

    // Check domain rules
    let blocked = denied.matches(&host);
    let allowed_match = allowed.matches(&host);

    let should_allow = if blocked {
        false
    } else if allowed_match {
        true
    } else {
        default_action
    };

    if !should_allow {
        let response = "HTTP/1.1 403 Forbidden\r\nX-Proxy-Error: blocked-by-allowlist\r\n\r\n";
        stream.write_all(response.as_bytes()).await?;
        tracing::info!("Blocked connection to {}", target);
        return Ok(());
    }

    // Connect to target
    let target_addr = format!("{}:{}", host, port);
    let mut target_stream = match TcpStream::connect(&target_addr).await {
        Ok(s) => s,
        Err(e) => {
            let response = format!(
                "HTTP/1.1 502 Bad Gateway\r\n\r\nConnection failed: {}\r\n",
                e
            );
            stream.write_all(response.as_bytes()).await?;
            return Ok(());
        }
    };

    // Send 200 Connection Established
    stream
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await?;

    // Tunnel data bidirectionally
    let (mut ri, mut wi) = stream.split();
    let (mut ro, mut wo) = target_stream.split();

    let client_to_target = tokio::io::copy(&mut ri, &mut wo);
    let target_to_client = tokio::io::copy(&mut ro, &mut wi);

    tokio::try_join!(client_to_target, target_to_client)?;

    Ok(())
}

/// Parse host:port from CONNECT request
fn parse_host_port(
    target: &str,
) -> Result<(String, u16), Box<dyn std::error::Error + Send + Sync>> {
    if let Some(colon_pos) = target.rfind(':') {
        let host = target[..colon_pos].to_string();
        let port: u16 = target[colon_pos + 1..].parse()?;
        Ok((host, port))
    } else {
        Err("Invalid host:port format".into())
    }
}

