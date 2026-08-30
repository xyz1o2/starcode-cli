/// SSH探测

/// SSH探测结果
#[derive(Debug)]
pub struct SSHProbeResult {
    /// 主机
    pub host: String,
    /// 端口
    pub port: u16,
    /// 是否可达
    pub reachable: bool,
    /// 响应时间（毫秒）
    pub response_time_ms: u64,
    /// 错误信息
    pub error: Option<String>,
}

/// SSH探测
pub struct SSHProbe;

impl SSHProbe {
    /// 创建新的SSH探测
    pub fn new() -> Self {
        Self
    }

    /// 探测主机
    pub fn probe(&self, host: &str, port: u16) -> SSHProbeResult {
        let start = std::time::Instant::now();
        
        // 简单的TCP连接测试
        let reachable = std::net::TcpStream::connect_timeout(
            &format!("{}:{}", host, port).parse().unwrap(),
            std::time::Duration::from_secs(5),
        ).is_ok();

        let response_time_ms = start.elapsed().as_millis() as u64;

        SSHProbeResult {
            host: host.to_string(),
            port,
            reachable,
            response_time_ms,
            error: if reachable { None } else { Some("Connection failed".to_string()) },
        }
    }
}
