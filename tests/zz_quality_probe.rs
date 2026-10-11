//! TEMPORARY probe #2: quantify ranking quality + chunk size issues.
use std::path::Path;

#[test]
fn probe_quality() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    // (query, file we KNOW is correct)
    let cases = [
        ("tool shortlist selection", "src/agent/tool_routing/helpers.rs"),
        ("abort token cancellation", "src/agent/agent_core.rs"),
        ("user abort token", "src/agent/agent_core.rs"),
        ("search engine scoring", "src/core/context/search_engine.rs"),
    ];
    for (q, expected) in cases {
        println!("\n===== QUERY: {}  (expect {}) =====", q, expected);
        let out =
            starcode_cli::core::tools::codebase_search::search_codebase(root, q, None).unwrap();
        let mut rank = 0usize;
        let mut found = false;
        for line in out.lines() {
            if let Some(rest) = line.strip_prefix("File: ") {
                rank += 1;
                let path = rest.split(" (Score").next().unwrap_or("");
                let lines = rest.split("Lines: ").nth(1).unwrap_or("").trim();
                let span: usize = lines
                    .split_once('-')
                    .and_then(|(a, b)| {
                        a.trim().parse::<usize>().ok().zip(b.trim().parse::<usize>().ok())
                    })
                    .map(|(a, b)| b.saturating_sub(a) + 1)
                    .unwrap_or(0);
                let hit = path == expected;
                if hit {
                    found = true;
                }
                println!(
                    "  #{:<2} chunk_lines={:<5} {}",
                    rank,
                    span,
                    path
                );
                if rank >= 10 {
                    break;
                }
            }
        }
        println!("  => correct file rank: {}", if found { "in top10" } else { "MISSING" });
    }
}