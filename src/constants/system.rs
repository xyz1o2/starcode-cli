/// 系统常量

/// 进程名称
pub const PROCESS_NAME: &str = "starcode";
pub const PROCESS_NAME_CLI: &str = "starcode-cli";
pub const PROCESS_NAME_SERVER: &str = "starcode-server";

/// 端口
pub const DEFAULT_PORT: u16 = 3000;
pub const DEFAULT_PORT_SERVER: u16 = 8080;
pub const DEFAULT_PORT_DEBUG: u16 = 9229;

/// 环境变量
pub const ENV_API_KEY: &str = "STARCODE_API_KEY";
pub const ENV_BASE_URL: &str = "STARCODE_BASE_URL";
pub const ENV_MODEL: &str = "STARCODE_MODEL";
pub const ENV_PROVIDER: &str = "STARCODE_PROVIDER";
pub const ENV_DEBUG: &str = "STARCODE_DEBUG";
pub const ENV_LOG_LEVEL: &str = "STARCODE_LOG_LEVEL";
pub const ENV_CONFIG_PATH: &str = "STARCODE_CONFIG_PATH";
pub const ENV_DATA_DIR: &str = "STARCODE_DATA_DIR";
pub const ENV_CACHE_DIR: &str = "STARCODE_CACHE_DIR";

/// 信号
pub const SIGNAL_SIGINT: i32 = 2;
pub const SIGNAL_SIGTERM: i32 = 15;
pub const SIGNAL_SIGHUP: i32 = 1;
pub const SIGNAL_SIGKILL: i32 = 9;

/// 编码
pub const ENCODING_UTF8: &str = "utf-8";
pub const ENCODING_ASCII: &str = "ascii";
pub const ENCODING_LATIN1: &str = "latin-1";

/// MIME类型
pub const MIME_JSON: &str = "application/json";
pub const MIME_TEXT: &str = "text/plain";
pub const MIME_HTML: &str = "text/html";
pub const MIME_XML: &str = "text/xml";
pub const MIME_FORM: &str = "application/x-www-form-urlencoded";
pub const MIME_MULTIPART: &str = "multipart/form-data";
pub const MIME_OCTET_STREAM: &str = "application/octet-stream";

/// HTTP头
pub const HEADER_CONTENT_TYPE: &str = "Content-Type";
pub const HEADER_AUTHORIZATION: &str = "Authorization";
pub const HEADER_ACCEPT: &str = "Accept";
pub const HEADER_USER_AGENT: &str = "User-Agent";
pub const HEADER_CACHE_CONTROL: &str = "Cache-Control";
pub const HEADER_X_REQUESTED_WITH: &str = "X-Requested-With";

/// HTTP方法
pub const METHOD_GET: &str = "GET";
pub const METHOD_POST: &str = "POST";
pub const METHOD_PUT: &str = "PUT";
pub const METHOD_PATCH: &str = "PATCH";
pub const METHOD_DELETE: &str = "DELETE";
pub const METHOD_HEAD: &str = "HEAD";
pub const METHOD_OPTIONS: &str = "OPTIONS";
