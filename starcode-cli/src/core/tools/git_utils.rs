pub(crate) async fn run_git(dir: &std::path::Path, args: &[&str]) -> Result<String, String> {
    let output = tokio::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .await
        .map_err(|e| format!("git command failed: {}", e))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}
