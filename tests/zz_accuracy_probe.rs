//! TEMPORARY probe: measure real codebase_search accuracy on this repo.
use std::path::Path;

#[test]
fn probe_accuracy() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let queries = [
        "how is authentication handled",
        "tool shortlist selection",
        "abort token cancellation",
        "does the project handle rate limiting",
    ];
    for q in queries {
        println!("\n================ QUERY: {} ================", q);
        match starcode_cli::core::tools::codebase_search::search_codebase(root, q, None) {
            Ok(out) => {
                for line in out.lines().take(14) {
                    println!("{}", line);
                }
            }
            Err(e) => println!("ERROR: {}", e),
        }
    }
}
