use crate::core::tools::diff_options::create_patch;
use std::path::{Path, PathBuf};

pub struct ModifyContext<TParams> {
    pub get_file_path: Box<dyn Fn(&TParams) -> String + Send + Sync>,
    pub get_current_content: Box<
        dyn Fn(
                &TParams,
            ) -> std::pin::Pin<
                Box<
                    dyn std::future::Future<Output = Result<String, Box<dyn std::error::Error>>>
                        + Send,
                >,
            > + Send
            + Sync,
    >,
    pub get_proposed_content: Box<
        dyn Fn(
                &TParams,
            ) -> std::pin::Pin<
                Box<
                    dyn std::future::Future<Output = Result<String, Box<dyn std::error::Error>>>
                        + Send,
                >,
            > + Send
            + Sync,
    >,
    pub create_updated_params: Box<dyn Fn(String, String, TParams) -> TParams + Send + Sync>,
}

#[derive(Debug, Clone)]
pub struct ModifyResult<TParams> {
    pub updated_params: TParams,
    pub updated_diff: String,
}

#[derive(Debug, Clone)]
pub struct ModifyContentOverrides {
    pub current_content: Option<String>,
    pub proposed_content: Option<String>,
}

pub trait ModifiableDeclarativeTool<TParams>: Send + Sync {
    fn get_modify_context(&self) -> ModifyContext<TParams>;
}

pub fn create_temp_files_for_modify(
    current_content: &str,
    proposed_content: &str,
    file_path: &Path,
) -> Result<(PathBuf, PathBuf, PathBuf), Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let diff_dir = temp_dir.path();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(diff_dir, std::fs::Permissions::from_mode(0o700))?;
    }

    let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let file_name = file_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis();

    let temp_old_path = diff_dir.join(format!("{}-old-{}{}", file_name, timestamp, ext));
    let temp_new_path = diff_dir.join(format!("{}-new-{}{}", file_name, timestamp, ext));

    std::fs::write(&temp_old_path, current_content)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp_old_path, std::fs::Permissions::from_mode(0o600))?;
    }

    std::fs::write(&temp_new_path, proposed_content)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp_new_path, std::fs::Permissions::from_mode(0o600))?;
    }

    Ok((temp_old_path, temp_new_path, diff_dir.to_path_buf()))
}

pub fn get_updated_params<TParams: Clone>(
    tmp_old_path: &Path,
    temp_new_path: &Path,
    original_params: &TParams,
    modify_context: &ModifyContext<TParams>,
) -> Result<(TParams, String), Box<dyn std::error::Error>> {
    let old_content = std::fs::read_to_string(tmp_old_path).unwrap_or_default();
    let new_content = std::fs::read_to_string(temp_new_path).unwrap_or_default();

    let updated_params = (modify_context.create_updated_params)(
        old_content.clone(),
        new_content.clone(),
        original_params.clone(),
    );

    let file_name = tmp_old_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let updated_diff = create_patch(file_name, &old_content, &new_content, "Current", "Proposed");

    Ok((updated_params, updated_diff))
}

pub fn delete_temp_files(old_path: &Path, new_path: &Path, dir_path: &Path) {
    let _ = std::fs::remove_file(old_path);
    let _ = std::fs::remove_file(new_path);
    let _ = std::fs::remove_dir(dir_path);
}

pub async fn modify_with_editor<TParams: Clone + Send + 'static>(
    original_params: TParams,
    modify_context: ModifyContext<TParams>,
    _abort_signal: &tokio_util::sync::CancellationToken,
    overrides: Option<ModifyContentOverrides>,
) -> Result<ModifyResult<TParams>, Box<dyn std::error::Error>> {
    let has_current_override = overrides
        .as_ref()
        .and_then(|o| o.current_content.as_ref())
        .is_some();
    let has_proposed_override = overrides
        .as_ref()
        .and_then(|o| o.proposed_content.as_ref())
        .is_some();

    let current_content = if has_current_override {
        overrides
            .as_ref()
            .and_then(|o| o.current_content.clone())
            .unwrap_or_default()
    } else {
        (modify_context.get_current_content)(&original_params).await?
    };

    let proposed_content = if has_proposed_override {
        overrides
            .as_ref()
            .and_then(|o| o.proposed_content.clone())
            .unwrap_or_default()
    } else {
        (modify_context.get_proposed_content)(&original_params).await?
    };

    let file_path_str = (modify_context.get_file_path)(&original_params);
    let file_path = PathBuf::from(&file_path_str);

    let (old_path, new_path, dir_path) =
        create_temp_files_for_modify(&current_content, &proposed_content, &file_path)?;

    // Implement editor opening logic
    // 1. Try VS Code with --diff
    let vscode_status = tokio::process::Command::new("code")
        .arg("--wait")
        .arg("--diff")
        .arg(&old_path)
        .arg(&new_path)
        .status()
        .await;

    let opened = match vscode_status {
        Ok(s) => s.success(),
        Err(_) => false,
    };

    if !opened {
        // 2. Try EDITOR env var
        if let Ok(editor) = std::env::var("EDITOR") {
            let _ = tokio::process::Command::new(editor)
                .arg(&new_path)
                .status()
                .await;
        } else {
            // 3. Fallback to manual check
            println!("⚠️ No EDITOR set and 'code' (VS Code) not found.");
            println!(
                "Please inspect/edit the proposed changes at: {}",
                new_path.display()
            );
            println!("Press Enter to continue...");

            let _ = tokio::task::spawn_blocking(|| {
                let _ = std::io::stdin().read_line(&mut String::new());
            })
            .await;
        }
    }

    let result = get_updated_params(&old_path, &new_path, &original_params, &modify_context)?;

    delete_temp_files(&old_path, &new_path, &dir_path);

    Ok(ModifyResult {
        updated_params: result.0,
        updated_diff: result.1,
    })
}
