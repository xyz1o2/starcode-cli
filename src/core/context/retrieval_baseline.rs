//! 检索质量基线 —— 给 context 引擎的改造提供可复现的度量。
//!
//! 为什么需要它：context 引擎的每次改造（分块策略、召回通道、重排信号）
//! 都是「感觉变好了」式的调整，没有基线就无法区分「真的更准」和
//! 「碰巧在这次手测的问题上更好」。这里用一个小而确定的合成语料，
//! 把「召回」这件事变成可断言的数字。
//!
//! 语料刻意包含两类难点：
//! - **跨语言**：代码里的标识符是英文（`handle_error`），而提问是中文
//!   （「错误处理」）。这类查询在纯字面检索下必然零命中。
//! - **同义改写**：提问用的词和代码里的词不一样（提问「重试」，代码
//!   `retry_backoff`）。这类查询靠字面匹配也命中不了。
//!
//! 这两类恰好是真实使用中最常见的形态，也正是纯词法检索的短板所在。

use super::chunking::CodeChunk;
use super::search_engine::SearchEngine;

/// 构造一条 chunk。
fn chunk(path: &str, header: &str, content: &str, start_line: usize) -> CodeChunk {
    CodeChunk {
        content: content.to_string(),
        start_line,
        end_line: start_line + content.lines().count(),
        context_header: Some(header.to_string()),
    }
}

/// 一份小而全的合成语料。
///
/// 每个条目：(文件路径, 上下文头, 内容, 期望被召回的查询)
fn corpus() -> Vec<(String, CodeChunk)> {
    vec![
        (
            "src/http/error.rs".to_string(),
            chunk(
                "src/http/error.rs",
                "fn handle_error(err: &Error) -> Response",
                "pub fn handle_error(err: &Error) -> Response {\n    let status = match err.kind() {\n        ErrorKind::NotFound => 404,\n        ErrorKind::Invalid => 400,\n    };\n    Response::new(status)\n}",
                10,
            ),
        ),
        (
            "src/net/retry.rs".to_string(),
            chunk(
                "src/net/retry.rs",
                "fn retry_backoff(attempts: u32) -> Duration",
                "pub fn retry_backoff(attempts: u32) -> Duration {\n    let exp = 2u64.pow(attempts);\n    Duration::from_millis(BASE_MS * exp)\n}",
                5,
            ),
        ),
        (
            "src/auth/limiter.rs".to_string(),
            chunk(
                "src/auth/limiter.rs",
                "fn rate_limit(key: &str) -> bool",
                "pub fn rate_limit(key: &str) -> bool {\n    let bucket = BUCKETS.entry(key).or_default();\n    bucket.count < MAX_PER_MINUTE\n}",
                20,
            ),
        ),
        (
            "src/auth/token.rs".to_string(),
            chunk(
                "src/auth/token.rs",
                "fn verify_token(raw: &str) -> Claims",
                "pub fn verify_token(raw: &str) -> Claims {\n    let decoded = decode_segment(raw)?;\n    Claims::from(decoded)\n}",
                3,
            ),
        ),
        (
            "src/db/pool.rs".to_string(),
            chunk(
                "src/db/pool.rs",
                "fn acquire_connection(pool: &Pool) -> Connection",
                "pub fn acquire_connection(pool: &Pool) -> Connection {\n    pool.try_get().expect(\"pool exhausted\")\n}",
                42,
            ),
        ),
        // 中文注释 —— 中文查询应当能命中中文注释里的内容。
        // 这是 CJK 分词的直接价值：以前「错误处理」整段是一个 token，
        // 查「错误」永远命中不了这里。
        (
            "src/http/handler.rs".to_string(),
            chunk(
                "src/http/handler.rs",
                "fn dispatch(req: Request) -> Response",
                "// 统一错误处理入口：把内部异常翻译成对外的 HTTP 响应\npub fn dispatch(req: Request) -> Response {\n    match route(req) {\n        Ok(v) => ok(v),\n        Err(e) => error_response(e),\n    }\n}",
                3,
            ),
        ),
    ]
}

fn engine_with_corpus() -> SearchEngine {
    let mut engine = SearchEngine::new();
    for (path, ch) in corpus() {
        engine.add_document(path, vec![ch]);
    }
    engine
}

/// 查询 → 期望命中的文件路径片段。
struct Case {
    query: &'static str,
    expect: &'static str,
    why: &'static str,
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            query: "handle_error",
            expect: "error.rs",
            why: "标识符直接命中 —— 这是词法检索的主场",
        },
        Case {
            query: "error handling",
            expect: "error.rs",
            why: "英文短语，标识符被拆成 handle/error 两个 token",
        },
        Case {
            query: "retry backoff",
            expect: "retry.rs",
            why: "英文同义改写，token 与标识符一致",
        },
        Case {
            query: "rate limit",
            expect: "limiter.rs",
            why: "英文关键词",
        },
        Case {
            query: "错误处理",
            expect: "error.rs",
            why: "中文提问 + 英文代码 —— 跨语言召回",
        },
        Case {
            query: "重试",
            expect: "retry.rs",
            why: "中文近义词与英文标识符不对应",
        },
        Case {
            query: "限流",
            expect: "limiter.rs",
            why: "中文术语 vs 英文 rate_limit",
        },
        Case {
            query: "错误处理",
            expect: "handler.rs",
            why: "中文查询命中中文注释 —— CJK 分词的直接价值",
        },
    ]
}

/// 跑一遍所有用例，返回 (总命中数, 用例数, 明细)。
fn run_cases(engine: &SearchEngine) -> (usize, usize, Vec<(String, bool, String)>) {
    let mut hits = 0usize;
    let mut detail = Vec::new();
    for case in cases() {
        let results = engine.search(case.query, 10);
        let got = results.iter().any(|r| r.file_path.contains(case.expect));
        if got {
            hits += 1;
        }
        detail.push((case.query.to_string(), got, case.why.to_string()));
    }
    (hits, cases().len(), detail)
}

/// 基线：英文查询应当基本全中。
///
/// 这条断言锁的是「别把已经能用的搞坏」——它是改造的底线，不是目标。
#[test]
fn english_queries_recall_their_targets() {
    let engine = engine_with_corpus();
    let (hits, total, detail) = run_cases(&engine);
    let english: Vec<_> = detail.iter().filter(|(q, _, _)| q.is_ascii()).collect();
    let english_hits = english.iter().filter(|(_, ok, _)| *ok).count();
    assert_eq!(
        english_hits,
        english.len(),
        "英文查询回退了：\n  {}",
        english
            .iter()
            .filter(|(_, ok, _)| !*ok)
            .map(|(q, _, why)| format!("{q}（{why}）"))
            .collect::<Vec<_>>()
            .join("\n  ")
    );
    let _ = (hits, total);
}

/// 中文查询命中**中文注释** —— CJK 分词的价值所在。
///
/// 以前「错误处理」整段被切成**一个** token，于是查「错误处理」只能命中
/// 原文里逐字连续写下的那一串；代码里写「错误」「处理」两处就永远搜不到。
/// bigram 切分之后，中文查询能命中中文注释。
#[test]
fn chinese_query_reaches_chinese_comment() {
    let engine = engine_with_corpus();
    let results = engine.search("错误处理", 10);
    assert!(
        results.iter().any(|r| r.file_path.ends_with("handler.rs")),
        "中文查询应能命中中文注释；实际结果：{:?}",
        results.iter().map(|r| &r.file_path).collect::<Vec<_>>()
    );
}

/// 中文查询命中**英文标识符** —— 跨语言召回，当前**尚未实现**。
///
/// 这是纯字法检索的硬边界：「错误处理」和 `handle_error` 没有任何共同
/// token，靠分词、切分、融合都跨不过去。要跨过去只有两条路：
/// - **C1**：LLM 查询改写（把中文提问翻成英文代码关键词）
/// - **C2**：本地 embedding（语义跨语言）
///
/// 它现在是 `#[ignore]`：等其中一条落地后去掉 ignore 并改成正向断言。
/// 这样差距写在代码里，而不是散落在某个人的记忆里。
#[test]
#[ignore = "待实现跨语言召回（C1 LLM 查询改写 / C2 本地 embedding）"]
fn chinese_query_reaches_english_identifier_yet() {
    let engine = engine_with_corpus();
    let (hits, total, detail) = run_cases(&engine);
    let chinese: Vec<_> = detail.iter().filter(|(q, _, _)| !q.is_ascii()).collect();
    let chinese_hits = chinese.iter().filter(|(_, ok, _)| *ok).count();
    assert_eq!(
        chinese_hits,
        chinese.len(),
        "中文查询应全部召回；未命中：{:?}",
        chinese
            .iter()
            .filter(|(_, ok, _)| !*ok)
            .map(|(q, _, why)| format!("{q}（{why}）"))
            .collect::<Vec<_>>()
    );
    println!(
        "中文召回：{chinese_hits}/{}；总 {hits}/{total}",
        chinese.len()
    );
}
