//! AgentMail RemoteBridge —— 局域网/单机多进程互通（Phase 5，设计稿 §7 的落地）。
//!
//! 架构原则（`docs/agent-messaging-design.zh-CN.md` §7.2）：
//! - **进程内 [`AgentRouter`] 是唯一语义层**：远端消息落进本地 mailbox 后，
//!   走现成的「turn 开头 drain → user-role XML」消费路径；
//! - **无死信**：入站消息投给未注册身份 → 显式 404；出站对端不可达 /
//!   拒收 → 显式上报，绝不假装成功；
//! - **安全默认**：listener 默认绑定 `127.0.0.1`（单机多进程互通免配置）；
//!   暴露到局域网必须同时满足 显式 host 覆盖 + 共享密钥，否则拒绝启动。
//!
//! 协议：
//! - `GET /.well-known/star-agent.json` → 发现卡片（identity / inbox URL）；
//! - `POST /agent-mail/inbox`，body = [`AgentMessage`] JSON，Bearer 共享密钥
//!   （密钥未设且绑定回环时可省略）；成功 = `202 {"status":"queued"}`。
//!
//! 环境变量（对齐仓库 STAR_* 惯例）：
//! - `STAR_AGENT_PEERS`：逗号分隔的对端 base URL（如 `http://10.0.0.5:7901`）；
//! - `STAR_AGENT_MAIL_KEY`：共享密钥（入站校验 + 出站附带）；
//! - `STAR_AGENT_MAIL_HOST` / `STAR_AGENT_MAIL_PORT`（默认 `127.0.0.1:7901`）；
//! - `STAR_AGENT_MAIL_ENABLED`：入站 listener 开关（默认关）。

use super::{AgentMessage, SharedAgentRouter};
use http_body_util::BodyExt;
use hyper::body::Bytes;
use hyper::{Request, Response, StatusCode};
use std::sync::Arc;

/// 消息体上限（设计 §7.2 第 5 条：64 KB，对齐 mailbox 节流思路）。
const MAX_BODY_BYTES: usize = 64 * 1024;

/// 入站 listener 配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailServerConfig {
    pub host: String,
    pub port: u16,
    pub shared_key: Option<String>,
    pub enabled: bool,
}

impl MailServerConfig {
    /// 从环境变量读配置。默认：回环 + 7901 + 关闭。
    pub fn from_env() -> Self {
        let enabled = std::env::var("STAR_AGENT_MAIL_ENABLED")
            .ok()
            .map(|v| {
                let v = v.to_lowercase();
                !(v == "0" || v == "false" || v == "off")
            })
            .unwrap_or(false);
        Self {
            host: std::env::var("STAR_AGENT_MAIL_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
            port: std::env::var("STAR_AGENT_MAIL_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(7901),
            shared_key: std::env::var("STAR_AGENT_MAIL_KEY")
                .ok()
                .filter(|k| !k.is_empty()),
            enabled,
        }
    }

    /// 安全闸门：非回环绑定必须带共享密钥，否则不能启动。
    /// （§7.2 第 5 条：暴露到局域网需要显式 host + 共享密钥两个条件。）
    pub fn is_safe_to_start(&self) -> Result<(), String> {
        if self.host != "127.0.0.1" && self.host != "localhost" && self.shared_key.is_none() {
            return Err(format!(
                "STAR_AGENT_MAIL_HOST={} exposes the mail listener beyond loopback; \
                 set STAR_AGENT_MAIL_KEY to require a shared secret, or keep the default 127.0.0.1",
                self.host
            ));
        }
        Ok(())
    }
}

// ── 出站：PeerMailClient ─────────────────────────────────────────────────

/// 出站转发报告（每对端一条结果；不是静默成功）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerForwardReport {
    pub accepted: Vec<String>,
    pub failed: Vec<(String, String)>,
}

impl PeerForwardReport {
    pub fn is_all_failed(&self) -> bool {
        self.accepted.is_empty()
    }
}

/// 出站投递客户端（reqwest 复用总线上的运行时）。
pub struct PeerMailClient {
    peers: Vec<String>,
    shared_key: Option<String>,
    http: reqwest::Client,
}

impl PeerMailClient {
    /// `STAR_AGENT_PEERS` = 逗号分隔的 base URL；去空白、去尾斜杠。
    pub fn from_env() -> Self {
        let peers = std::env::var("STAR_AGENT_PEERS")
            .map(|v| {
                v.split(',')
                    .map(|p| p.trim().trim_end_matches('/').to_string())
                    .filter(|p| !p.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        Self::new(
            peers,
            std::env::var("STAR_AGENT_MAIL_KEY")
                .ok()
                .filter(|k| !k.is_empty()),
        )
    }

    pub fn new(peers: Vec<String>, shared_key: Option<String>) -> Self {
        Self {
            peers,
            shared_key,
            http: reqwest::Client::new(),
        }
    }

    /// 配置的 peers 原始 URL 列表（`/agent-mail` 状态展示用）。
    pub fn peer_base_urls(&self) -> Vec<String> {
        self.peers.clone()
    }

    pub fn has_peers(&self) -> bool {
        !self.peers.is_empty()
    }

    /// 转发一条消息到全部对端（单播消息按对端自行解析 identity；
    /// `to == "*all"` 是跨机广播，普通 `"*"` 只在本进程广播，见 §7.2 第 3 条）。
    pub async fn forward(&self, mut msg: AgentMessage, to: &str) -> PeerForwardReport {
        let mut accepted = Vec::new();
        let mut failed = Vec::new();
        for peer in &self.peers {
            let url = format!("{}/agent-mail/inbox", peer);
            let mut req = self.http.post(&url).json(&msg);
            if let Some(ref key) = self.shared_key {
                req = req.bearer_auth(key);
            }
            match req.send().await {
                Ok(resp) if resp.status() == StatusCode::ACCEPTED => {
                    accepted.push(peer.clone());
                }
                Ok(resp) => failed.push((
                    peer.clone(),
                    format!("peer replied {} (not 202 queued)", resp.status()),
                )),
                Err(e) => failed.push((peer.clone(), format!("unreachable: {}", e))),
            }
        }
        let _ = msg; // 消息已按对端序列化发送；单条入参不区分已送达副本
        PeerForwardReport { accepted, failed }
    }
}

// ── 发现（Phase 5.3 后半）：well-known 卡片探测 ──────────────────────────

/// 一个对端的发现结果（卡片字段 + 探测状态）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerCard {
    /// 探针请求用的 base URL。
    pub base_url: String,
    /// 探测结果：Ok = 对端是 starcode agent-mail 服务。
    pub status: Result<(), String>,
    /// 卡片内容（探测成功时有）。
    pub kind: Option<String>,
    pub version: Option<i64>,
    pub inbox_path: Option<String>,
    /// 卡片宣称的鉴权方式（`bearer` / `none-loopback`）。
    pub auth: Option<String>,
}

/// 逐对端探测 `GET /.well-known/star-agent.json`。
///
/// 这是 semiautomatic 发现的内核：`STAR_AGENT_PEERS` 给候选列表，探测卡片
/// 确认对端真是 agent-mail 服务并读它的鉴权策略。**不做 mDNS 自动扫描**
/// （设计 §7.2 第 2 条：v1 保持可审计）——列表外的地址永远不会被主动探测。
pub async fn discover_peers(peers: &[String]) -> Vec<PeerCard> {
    let http = reqwest::Client::new();
    let mut cards = Vec::new();
    for peer in peers {
        let url = format!("{}/.well-known/star-agent.json", peer);
        match http
            .get(&url)
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .await
        {
            Ok(resp) if resp.status() == StatusCode::OK => {
                match resp.json::<serde_json::Value>().await {
                    Ok(card)
                        if card.get("kind").and_then(|k| k.as_str())
                            == Some("starcode-agent-mail") =>
                    {
                        cards.push(PeerCard {
                            base_url: peer.clone(),
                            status: Ok(()),
                            kind: card.get("kind").and_then(|k| k.as_str()).map(String::from),
                            version: card.get("version").and_then(|v| v.as_i64()),
                            inbox_path: card
                                .get("inbox_path")
                                .and_then(|p| p.as_str())
                                .map(String::from),
                            auth: card.get("auth").and_then(|a| a.as_str()).map(String::from),
                        });
                    }
                    Ok(_) => cards.push(PeerCard {
                        base_url: peer.clone(),
                        status: Err("not an agent-mail service (unknown card)".into()),
                        kind: None,
                        version: None,
                        inbox_path: None,
                        auth: None,
                    }),
                    Err(e) => cards.push(PeerCard {
                        base_url: peer.clone(),
                        status: Err(format!("card decode failed: {}", e)),
                        kind: None,
                        version: None,
                        inbox_path: None,
                        auth: None,
                    }),
                }
            }
            Ok(resp) => cards.push(PeerCard {
                base_url: peer.clone(),
                status: Err(format!("probe replied {}", resp.status())),
                kind: None,
                version: None,
                inbox_path: None,
                auth: None,
            }),
            Err(e) => cards.push(PeerCard {
                base_url: peer.clone(),
                status: Err(format!("unreachable: {}", e)),
                kind: None,
                version: None,
                inbox_path: None,
                auth: None,
            }),
        }
    }
    cards
}

// ── 入站：HTTP listener ──────────────────────────────────────────────────

/// 启动入站邮箱 listener。返回 `(bound_addr, JoinHandle)` 便于测试断言与
/// bootstrap 记日志。配置禁用 / 不安全时返回错误说明，不启动。
pub async fn spawn_inbound_listener(
    hub: SharedAgentRouter,
    cfg: MailServerConfig,
) -> Result<(std::net::SocketAddr, tokio::task::JoinHandle<()>), String> {
    spawn_inbound_listener_impl(hub, None, cfg).await
}

/// 同 [`spawn_inbound_listener`]，但携带到信唤醒信号（IdleNotification 联动，
/// Phase 6）：远端来信落进本地 mailbox 后顺手 notify，UI 空闲等待被唤醒后
/// 注入一条 user 消息让 agent 立刻消费（而不是干等下一次用户输入）。
pub async fn spawn_inbound_listener_with_wakeup(
    hub: SharedAgentRouter,
    wakeup: std::sync::Arc<super::MailWakeupSignal>,
    cfg: MailServerConfig,
) -> Result<(std::net::SocketAddr, tokio::task::JoinHandle<()>), String> {
    spawn_inbound_listener_impl(hub, Some(wakeup), cfg).await
}

async fn spawn_inbound_listener_impl(
    hub: SharedAgentRouter,
    wakeup: Option<std::sync::Arc<super::MailWakeupSignal>>,
    cfg: MailServerConfig,
) -> Result<(std::net::SocketAddr, tokio::task::JoinHandle<()>), String> {
    if !cfg.enabled {
        return Err("mail listener disabled (STAR_AGENT_MAIL_ENABLED unset)".into());
    }
    cfg.is_safe_to_start()?;

    let addr = format!("{}:{}", cfg.host, cfg.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("bind {}: {}", addr, e))?;
    let bound: std::net::SocketAddr = listener.local_addr().map_err(|e| e.to_string())?;

    let key_for_auth = cfg.shared_key.clone();
    let wakeup_for_conn = wakeup.clone();
    let handle = tokio::spawn(async move {
        loop {
            let Ok((stream, _peer)) = listener.accept().await else {
                break;
            };
            let io = hyper_util::rt::TokioIo::new(stream);
            let hub = hub.clone();
            let shared_key = key_for_auth.clone();
            let wakeup_sig = wakeup_for_conn.clone();
            // hyper/http-body-util 都是依赖树里的既有版本（reqwest 传递依赖）。
            tokio::spawn(async move {
                let service =
                    hyper::service::service_fn(move |req: Request<hyper::body::Incoming>| {
                        let hub = hub.clone();
                        let shared_key = shared_key.clone();
                        let wakeup_sig = wakeup_sig.clone();
                        async move { handle_mail_request(hub, wakeup_sig, shared_key, req).await }
                    });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, service)
                    .await;
            });
        }
    });

    Ok((bound, handle))
}

/// 单个 HTTP 请求的处理：仅 `GET /.well-known/star-agent.json` 与
/// `POST /agent-mail/inbox` 两个路由，其余 404。
async fn handle_mail_request(
    hub: SharedAgentRouter,
    wakeup: Option<std::sync::Arc<super::MailWakeupSignal>>,
    shared_key: Option<String>,
    req: Request<hyper::body::Incoming>,
) -> Result<Response<http_body_util::Full<Bytes>>, std::convert::Infallible> {
    use http_body_util::Full;
    let response = match (req.method(), req.uri().path()) {
        (&hyper::Method::GET, "/.well-known/star-agent.json") => {
            let card = serde_json::json!({
                "kind": "starcode-agent-mail",
                "version": 1,
                // 收件地址要拿请求的 Host 头拼 —— 卡片里不带本机配好的 host，
                // 否则反过来发现的同学拿到一个绑定 0.0.0.0 的假地址。
                "inbox_path": "/agent-mail/inbox",
                "auth": if shared_key.is_some() { "bearer" } else { "none-loopback" },
            });
            Response::new(Full::new(
                serde_json::to_vec(&card).unwrap_or_default().into(),
            ))
        }
        (&hyper::Method::POST, "/agent-mail/inbox") => {
            // 鉴权：密钥已配置时强制 Bearer；未配置只允许回环部署（见
            // is_safe_to_start 的启动闸门），这里不再重复判断来源 IP。
            if let Some(ref key) = shared_key {
                let ok = req
                    .headers()
                    .get(hyper::header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .map(|v| v.trim_start_matches("Bearer ").trim_end() == key.as_str())
                    .unwrap_or(false);
                if !ok {
                    return Ok(json_response(
                        StatusCode::UNAUTHORIZED,
                        r#"{"error":"unauthorized"}"#,
                    ));
                }
            }

            let body = match req.into_body().collect().await {
                Ok(c) => c.to_bytes(),
                Err(_) => {
                    return Ok(json_response(
                        StatusCode::BAD_REQUEST,
                        r#"{"error":"body_read_failed"}"#,
                    ))
                }
            };
            if body.len() > MAX_BODY_BYTES {
                return Ok(json_response(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    r#"{"error":"message_too_large"}"#,
                ));
            }

            let msg: AgentMessage = match serde_json::from_slice(&body) {
                Ok(m) => m,
                Err(_) => {
                    return Ok(json_response(
                        StatusCode::BAD_REQUEST,
                        r#"{"error":"invalid_message"}"#,
                    ))
                }
            };

            // 无死信：投本地 hub。目标未注册 → 404 显式拒绝（发送方按失败上报）。
            let mut router = hub.lock().await;
            let to = msg.to.clone();
            // 出站转发的发送者记作其原 from（广播排除自身用的是本地视角，
            // 远端来信的 from_identity 只影响 "*" 广播，这里单条投递不受影响）。
            let delivery = router.deliver(msg, &to, "remote");
            // 到信唤醒（Phase 6）：投给 lead / 广播时通知空闲等待方。
            if let Some(ref sig) = wakeup {
                let wake = match &delivery {
                    super::Delivery::Delivered { mailbox_key, .. } => {
                        mailbox_key == super::LEAD_IDENTITY
                    }
                    super::Delivery::Broadcast { .. } => true,
                    super::Delivery::UnknownTarget { .. } => false,
                };
                if wake {
                    sig.notify_mail();
                }
            }
            match delivery {
                super::Delivery::Delivered { mailbox_key, .. } => {
                    let body = serde_json::json!({"status":"queued","mailbox":mailbox_key});
                    json_response(
                        StatusCode::ACCEPTED,
                        &serde_json::to_string(&body).unwrap_or_default(),
                    )
                }
                super::Delivery::Broadcast { delivered_to } => {
                    let body = serde_json::json!({"status":"queued","delivered_to":delivered_to});
                    json_response(
                        StatusCode::ACCEPTED,
                        &serde_json::to_string(&body).unwrap_or_default(),
                    )
                }
                super::Delivery::UnknownTarget { to } => {
                    let body = serde_json::json!({"error":"unknown_target","to":to});
                    json_response(
                        StatusCode::NOT_FOUND,
                        &serde_json::to_string(&body).unwrap_or_default(),
                    )
                }
            }
        }
        _ => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Full::new(Bytes::from_static(b"{\"error\":\"not_found\"}")))
            .unwrap(),
    };
    Ok(response)
}

fn json_response(status: StatusCode, body: &str) -> Response<http_body_util::Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(http_body_util::Full::new(body.as_bytes().to_vec().into()))
        .unwrap()
}

// ── 出站 fallback：LocalMissForwarder ────────────────────────────────────
// runner.deliver_message 在本地 UnknownTarget 且配置了 peers 时调 forward。

/// 环境快照（测试用：可注入 peers/key 而不必动进程环境）。
pub fn active_peers() -> Vec<String> {
    std::env::var("STAR_AGENT_PEERS")
        .map(|v| {
            v.split(',')
                .map(|p| p.trim().trim_end_matches('/').to_string())
                .filter(|p| !p.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::messaging::new_shared_router;

    fn cfg(host: &str, port: u16, key: Option<&str>, enabled: bool) -> MailServerConfig {
        MailServerConfig {
            host: host.into(),
            port,
            shared_key: key.map(String::from),
            enabled,
        }
    }

    #[test]
    fn mail_server_config_defaults_and_safety_gate() {
        // 默认回环无密钥 → 可以启动（回环免配置语义）
        assert!(cfg("127.0.0.1", 7901, None, true)
            .is_safe_to_start()
            .is_ok());
        // 非回环无密钥 → 拒绝启动（§7.2 安全闸门）
        let err = cfg("0.0.0.0", 7901, None, true).is_safe_to_start();
        assert!(err.is_err());
        // 非回环 + 密钥 → 放行
        assert!(cfg("0.0.0.0", 7901, Some("k"), true)
            .is_safe_to_start()
            .is_ok());
        // 未启用 → 不启动
        assert!(!cfg("127.0.0.1", 7901, None, false).enabled);
    }

    #[tokio::test]
    async fn inbound_delivers_to_local_mailbox_and_rejects_unknown() {
        let hub = new_shared_router();
        let mut router = hub.lock().await;
        router.register_entry("researcher", "agent-aaa111", Some("researcher".into()));
        drop(router);

        // 挑随机端口避免并行测试冲突：bind :0 然后用 bound addr（简化为
        // 直接用 spawn 返回的 bound）。
        let (bound, handle) = spawn_inbound_listener(hub.clone(), cfg("127.0.0.1", 0, None, true))
            .await
            .expect("listener should start on loopback");
        let url = format!("http://{}/agent-mail/inbox", bound);

        let http = reqwest::Client::new();

        // 未知目标 → 404 unknown_target（无死信语义）
        let mut ghost = AgentMessage::new("lead", "谁来收？", "投递");
        ghost.to = "ghost".into();
        let resp = http.post(&url).json(&ghost).send().await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["error"], "unknown_target");

        // 已注册目标 → 202 queued + 落进本地 mailbox
        let mut ok = AgentMessage::new("lead", "跨进程你好", "握手");
        ok.to = "researcher".into();
        let resp = http.post(&url).json(&ok).send().await.unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["status"], "queued");
        assert_eq!(
            hub.lock().await.drain("researcher")[0].content,
            "跨进程你好"
        );

        // well-known 发现卡片
        let resp = http
            .get(format!("http://{}/.well-known/star-agent.json", bound))
            .send()
            .await
            .unwrap();
        let card: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(card["kind"], "starcode-agent-mail");
        assert_eq!(card["version"], 1);

        handle.abort();
        let _ = bound;
    }

    #[tokio::test]
    async fn inbound_requires_bearer_when_key_configured() {
        let hub = new_shared_router();
        let (bound, handle) =
            spawn_inbound_listener(hub, cfg("127.0.0.1", 0, Some("sekret"), true))
                .await
                .unwrap();
        let url = format!("http://{}/agent-mail/inbox", bound);
        let http = reqwest::Client::new();

        let mut msg = AgentMessage::new("lead", "x", "s");
        msg.to = "someone".into();

        // 无头 / 错密钥 → 401
        let resp = http.post(&url).json(&msg).send().await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let resp = http
            .post(&url)
            .bearer_auth("wrong")
            .json(&msg)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // 对密钥 → 202（目标未注册也会 404，但鉴权先过；这里只验 401 之门）
        let resp = http
            .post(&url)
            .bearer_auth("sekret")
            .json(&msg)
            .send()
            .await
            .unwrap();
        assert_ne!(resp.status(), StatusCode::UNAUTHORIZED);

        handle.abort();
    }

    /// E2E：两个 hub + 两个 listener = 两个 StarCode 进程。
    /// 进程 A 本地对 "researcher" 无注册 → PeerMailClient 转发 → 进程 B 的
    /// researcher mailbox 落箱 → drain 消费。这是设计文档 §7.2 的全链路。
    #[tokio::test]
    async fn two_process_cross_forward_lands_in_peer_mailbox() {
        // 进程 A：只有 lead，没有 researcher
        let hub_a = new_shared_router();
        hub_a.lock().await.register_entry(
            crate::agent::messaging::LEAD_IDENTITY,
            "lead-a",
            Some(crate::agent::messaging::LEAD_IDENTITY.into()),
        );
        let (bound_a, handle_a) =
            spawn_inbound_listener(hub_a.clone(), cfg("127.0.0.1", 0, None, true))
                .await
                .unwrap();

        // 进程 B：注册了 researcher
        let hub_b = new_shared_router();
        hub_b
            .lock()
            .await
            .register_entry("researcher", "agent-b222", Some("researcher".into()));
        let (bound_b, handle_b) =
            spawn_inbound_listener(hub_b.clone(), cfg("127.0.0.1", 0, None, true))
                .await
                .unwrap();

        // A 的出站 client 指向 B 的入站地址（真实 HTTP 往返）
        let client = PeerMailClient::new(vec![format!("http://{}", bound_b)], None);
        let mut msg = AgentMessage::new(
            crate::agent::messaging::LEAD_IDENTITY,
            "B 上的 researcher 请查收",
            "跨进程派活",
        );
        msg.to = "researcher".into();
        let report = client.forward(msg, "researcher").await;

        // 对账：accepted = 传给 client 的 peer URL（含 scheme），无失败
        assert_eq!(report.accepted, vec![format!("http://{}", bound_b)]);
        assert!(report.failed.is_empty());

        // B 的 mailbox 真的收到了（from 保留 A 的 lead 身份）
        let drained = hub_b.lock().await.drain("researcher");
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].from, crate::agent::messaging::LEAD_IDENTITY);
        assert_eq!(drained[0].to, "researcher");
        assert!(drained[0].content.contains("B 上的 researcher"));

        // A 本地邮箱无残留（没投进 A）
        assert_eq!(hub_a.lock().await.pending_count("researcher"), 0);

        handle_a.abort();
        handle_b.abort();
    }

    #[test]
    fn peer_report_flags_total_failure() {
        let empty = PeerForwardReport {
            accepted: vec![],
            failed: vec![("p1".into(), "unreachable".into())],
        };
        assert!(empty.is_all_failed());
        let some = PeerForwardReport {
            accepted: vec!["p2".into()],
            failed: vec![],
        };
        assert!(!some.is_all_failed());
    }

    #[tokio::test]
    async fn forwarder_reports_unreachable_peers_explicitly() {
        // 指向一个没人监听的端口 → failed 显式上报（无死信）
        let client = PeerMailClient::new(vec!["http://127.0.0.1:1".into()], None);
        let msg = AgentMessage::new("lead", "no one home", "测试");
        let report = client.forward(msg, "researcher").await;
        assert!(report.is_all_failed());
        assert_eq!(report.failed.len(), 1);
    }
}
