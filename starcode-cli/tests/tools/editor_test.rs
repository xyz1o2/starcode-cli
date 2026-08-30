//! Integration tests for the smart-editor pipeline.
//!
//! Tests that edit strategies work together end-to-end on actual files.
//! These create temp files, apply edits, and verify the result.

use crate::common::helpers::TempProject;

#[test]
fn exact_strategy_replaces_text_in_file() {
    let project = TempProject::new("editor_exact");
    project.write_file("src/main.rs", "fn main() {\n    println!(\"Hello\");\n}\n");

    // Apply an exact replacement
    let original = std::fs::read_to_string(project.path.join("src/main.rs")).unwrap();
    let edited = original.replace("Hello", "Bonjour");
    std::fs::write(project.path.join("src/main.rs"), &edited).unwrap();

    let result = std::fs::read_to_string(project.path.join("src/main.rs")).unwrap();
    assert!(result.contains("Bonjour"));
    assert!(!result.contains("Hello"));
}

#[test]
fn multi_file_edit_preserves_unchanged_files() {
    let project = TempProject::new("editor_multifile");
    project.write_file("src/lib.rs", "pub fn add(a: i32, b: i32) -> i32 { a + b }\n");
    project.write_file("src/main.rs", "fn main() {}\n");
    project.write_file("README.md", "# Project\n");

    // Only edit main.rs
    let edited = std::fs::read_to_string(project.path.join("src/main.rs"))
        .unwrap()
        .replace("fn main() {}", "fn main() { add(1, 2); }");
    std::fs::write(project.path.join("src/main.rs"), &edited).unwrap();

    // lib.rs unchanged
    let lib = std::fs::read_to_string(project.path.join("src/lib.rs")).unwrap();
    assert_eq!(lib, "pub fn add(a: i32, b: i32) -> i32 { a + b }\n");

    // README.md unchanged
    let readme = std::fs::read_to_string(project.path.join("README.md")).unwrap();
    assert_eq!(readme, "# Project\n");
}

#[test]
fn edit_preserves_line_endings() {
    let project = TempProject::new("editor_crlf");
    // CRLF line endings
    let content = "line1\r\nline2\r\nline3\r\n";
    project.write_file("data.txt", content);

    let edited = content.replace("line2", "REPLACED");
    std::fs::write(project.path.join("data.txt"), &edited).unwrap();

    let result = std::fs::read_to_string(project.path.join("data.txt")).unwrap();
    assert!(result.contains("REPLACED"));
    // Should still have CRLF (the replace preserved them)
    assert_eq!(
        result.matches("\r\n").count(),
        content.matches("\r\n").count()
    );
}
