pub mod client;

use crate::core::tools::constants::LSP_TOOL_NAME;
use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use crate::tools::lsp::client::LspClient;
use async_trait::async_trait;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

pub struct LspTool {
    clients: Arc<tokio::sync::Mutex<HashMap<String, LspClient>>>,
}

impl LspTool {
    pub fn new() -> Self {
        Self {
            clients: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    fn detect_language(path: &str) -> Option<&'static str> {
        if path.ends_with(".rs") {
            Some("rust")
        } else if path.ends_with(".ts") || path.ends_with(".tsx") {
            Some("typescript")
        } else if path.ends_with(".js")
            || path.ends_with(".jsx")
            || path.ends_with(".mjs")
            || path.ends_with(".cjs")
        {
            Some("javascript")
        } else if path.ends_with(".py") {
            Some("python")
        } else if path.ends_with(".go") {
            Some("go")
        } else if path.ends_with(".c") || path.ends_with(".h") {
            Some("c")
        } else if path.ends_with(".cpp") || path.ends_with(".hpp") || path.ends_with(".cc") {
            Some("cpp")
        } else if path.ends_with(".java") {
            Some("java")
        } else {
            None
        }
    }

    fn get_server_config(language: &str) -> Option<(String, Vec<String>)> {
        match language {
            "rust" => Some(("rust-analyzer".to_string(), vec![])),
            "typescript" | "javascript" => Some((
                "typescript-language-server".to_string(),
                vec!["--stdio".to_string()],
            )),
            "python" => Some(("pylsp".to_string(), vec![])),
            "go" => Some(("gopls".to_string(), vec![])),
            "c" | "cpp" => Some(("clangd".to_string(), vec![])),
            "java" => Some(("jdtls".to_string(), vec![])), // jdtls setup is complex, but this is a start
            _ => None,
        }
    }

    fn get_install_hint(language: &str) -> &'static str {
        match language {
            "rust" => "Try: rustup component add rust-analyzer",
            "typescript" | "javascript" => "Try: npm install -g typescript-language-server typescript",
            "python" => "Try: pip install python-lsp-server",
            "go" => "Try: go install golang.org/x/tools/gopls@latest",
            "c" | "cpp" => "Try installing clangd via your system package manager (e.g., brew install llvm, apt install clangd)",
            "java" => "Ensure jdtls is in your PATH.",
            _ => "Please install the corresponding LSP server.",
        }
    }

    async fn ensure_client(&self, language: &str) -> Result<(), String> {
        let mut clients = self.clients.lock().await;

        if clients.contains_key(language) {
            return Ok(());
        }

        let (cmd, args) = Self::get_server_config(language)
            .ok_or_else(|| format!("Unsupported language: {}", language))?;

        // Check environment variable override
        let env_var_name = format!("STAR_LSP_SERVER_{}", language.to_uppercase());
        let cmd = std::env::var(&env_var_name).unwrap_or(cmd);

        match LspClient::new(&cmd, &args).await {
            Ok(client) => {
                clients.insert(language.to_string(), client);
                Ok(())
            }
            Err(e) => {
                let hint = Self::get_install_hint(language);
                Err(format!(
                    "LSP server '{}' failed to start: {}. \n\n💡 Installation Hint: {}",
                    cmd, e, hint
                ))
            }
        }
    }
}

#[async_trait]
impl BaseDeclarativeTool for LspTool {
    fn name(&self) -> &str {
        LSP_TOOL_NAME
    }

    fn display_name(&self) -> &str {
        "LSP Client"
    }

    fn description(&self) -> &str {
        "Interacts with LSP servers to get code intelligence. Supports diagnostics, go-to-definition, find-references, hover, document-symbol, workspace-symbol, type-definition, and implementation. Supports Rust, TS/JS, Python, Go, C/C++, Java."
    }

    fn kind(&self) -> Kind {
        Kind::Search
    }

    fn parameter_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The absolute path of the file to operate on. Required for file-specific actions."
                },
                "action": {
                    "type": "string",
                    "enum": [
                        "diagnostics",
                        "definition",
                        "references",
                        "hover",
                        "document_symbol",
                        "workspace_symbol",
                        "type_definition",
                        "implementation"
                    ],
                    "description": "The LSP action to perform. Defaults to 'diagnostics'.",
                    "default": "diagnostics"
                },
                "line": {
                    "type": "integer",
                    "description": "Line number (0-based) for definition/references/hover/type_definition/implementation. Required for those actions."
                },
                "character": {
                    "type": "integer",
                    "description": "Character offset (0-based) for definition/references/hover/type_definition/implementation. Required for those actions."
                },
                "query": {
                    "type": "string",
                    "description": "Query string for workspace_symbol action."
                }
            },
            "required": ["action"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Box::new(LspInvocation {
            params,
            tool: self.clone_box(),
        }))
    }
}

impl LspTool {
    fn clone_box(&self) -> Arc<LspTool> {
        Arc::new(LspTool {
            clients: self.clients.clone(),
        })
    }
}

pub struct LspInvocation {
    params: serde_json::Value,
    tool: Arc<LspTool>,
}

impl ToolInvocation for LspInvocation {
    fn get_description(&self) -> String {
        "Get LSP diagnostics".to_string()
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![]
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let tool = self.tool.clone();
        let params = self.params.clone();

        Box::pin(async move {
            let file_path_str = params.get("file_path").and_then(|v| v.as_str());
            let action = params
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("diagnostics");
            let line = params
                .get("line")
                .and_then(|v| v.as_u64())
                .map(|l| l as u32);
            let character = params
                .get("character")
                .and_then(|v| v.as_u64())
                .map(|c| c as u32);
            let query = params.get("query").and_then(|v| v.as_str());

            let mut target_languages = Vec::new();

            // Detect language
            if let Some(path) = file_path_str {
                if let Some(lang) = LspTool::detect_language(path) {
                    target_languages.push(lang);
                }
            } else if action == "workspace_symbol" {
                // For workspace symbol, we might want to search in all active clients or default to rust?
                // For now, let's try to infer from project files or default to rust if nothing else.
                // Or simply search in all initialized clients?
                // Let's assume the user has opened some files and thus initialized some clients.
                // But if no clients are initialized, we can't do much without a file path to hint the language.
                // TODO: Scan project root for files to detect languages?
                // For simplicity: If no file_path, check active clients.
            }

            // Ensure clients are running (if file_path provided)
            for lang in &target_languages {
                if let Err(e) = tool.ensure_client(lang).await {
                    return Ok(ToolResult {
                        llm_content: Some(format!("Error starting LSP for {}: {}", lang, e)),
                        return_display: None,
                        output: format!("Error starting LSP for {}: {}", lang, e),
                        error: None,
                        data: None,
                    });
                }
            }

            let clients_lock = tool.clients.lock().await;

            // Determine which client to use
            let mut clients_to_use = Vec::new();
            if !target_languages.is_empty() {
                let lang = target_languages.first().unwrap();
                if let Some(client) = clients_lock.get(*lang) {
                    clients_to_use.push((lang.to_string(), client));
                }
            } else if action == "workspace_symbol" {
                // Use all active clients
                for (lang, client) in clients_lock.iter() {
                    clients_to_use.push((lang.clone(), client));
                }
            }

            if clients_to_use.is_empty() {
                return Ok(ToolResult {
                    llm_content: Some("No active LSP clients found. Please provide a file_path to start the relevant language server.".to_string()),
                    return_display: None,
                    output: "No active LSP clients found. Please provide a file_path to start the relevant language server.".to_string(),
                    error: None,
                    data: None,
                });
            }

            let mut output = String::new();

            // Helper to get URI
            let get_uri = || -> Result<url::Url, String> {
                let path_buf = std::path::PathBuf::from(file_path_str.ok_or("file_path required")?);
                let uri = url::Url::from_file_path(&path_buf)
                    .map_err(|_| format!("Invalid file path: {}", file_path_str.unwrap()))?;
                Ok(uri)
            };

            for (lang, client) in clients_to_use {
                match action {
                    "definition" => {
                        let uri = match get_uri() {
                            Ok(u) => u,
                            Err(e) => {
                                output = e;
                                break;
                            }
                        };
                        if let (Some(l), Some(c)) = (line, character) {
                            match client.goto_definition(uri, l, c).await {
                                Ok(locs) => {
                                    if !locs.is_empty() {
                                        for loc in locs {
                                            output.push_str(&format!(
                                                "Definition ({}) at: {} {}:{}\n",
                                                lang,
                                                loc.uri.as_str(),
                                                loc.range.start.line,
                                                loc.range.start.character
                                            ));
                                        }
                                    }
                                }
                                Err(e) => output.push_str(&format!(
                                    "Error ({}) getting definition: {}\n",
                                    lang, e
                                )),
                            }
                        } else {
                            output = "Line and character are required for definition action."
                                .to_string();
                        }
                    }
                    "references" => {
                        let uri = match get_uri() {
                            Ok(u) => u,
                            Err(e) => {
                                output = e;
                                break;
                            }
                        };
                        if let (Some(l), Some(c)) = (line, character) {
                            match client.references(uri, l, c).await {
                                Ok(locs) => {
                                    if !locs.is_empty() {
                                        // Intelligent truncation: Show top 20 references to avoid context overflow
                                        let total = locs.len();
                                        let display_limit = 20;
                                        output.push_str(&format!(
                                            "References ({}) found (showing top {}/{}):\n",
                                            lang,
                                            std::cmp::min(total, display_limit),
                                            total
                                        ));

                                        for (i, loc) in locs.iter().enumerate().take(display_limit)
                                        {
                                            output.push_str(&format!(
                                                "  {}. {} {}:{}\n",
                                                i + 1,
                                                loc.uri.as_str(),
                                                loc.range.start.line,
                                                loc.range.start.character
                                            ));
                                        }

                                        if total > display_limit {
                                            output.push_str(&format!(
                                                "  ... and {} more references.\n",
                                                total - display_limit
                                            ));
                                        }
                                    } else {
                                        output.push_str(&format!(
                                            "No references ({}) found.\n",
                                            lang
                                        ));
                                    }
                                }
                                Err(e) => output.push_str(&format!(
                                    "Error ({}) getting references: {}\n",
                                    lang, e
                                )),
                            }
                        } else {
                            output = "Line and character are required for references action."
                                .to_string();
                        }
                    }
                    "type_definition" => {
                        let uri = match get_uri() {
                            Ok(u) => u,
                            Err(e) => {
                                output = e;
                                break;
                            }
                        };
                        if let (Some(l), Some(c)) = (line, character) {
                            match client.type_definition(uri, l, c).await {
                                Ok(locs) => {
                                    if !locs.is_empty() {
                                        for loc in locs {
                                            output.push_str(&format!(
                                                "Type Definition ({}) at: {} {}:{}\n",
                                                lang,
                                                loc.uri.as_str(),
                                                loc.range.start.line,
                                                loc.range.start.character
                                            ));
                                        }
                                    } else {
                                        output.push_str(&format!(
                                            "No type definition ({}) found.\n",
                                            lang
                                        ));
                                    }
                                }
                                Err(e) => output.push_str(&format!(
                                    "Error ({}) getting type definition: {}\n",
                                    lang, e
                                )),
                            }
                        } else {
                            output = "Line and character are required for type_definition action."
                                .to_string();
                        }
                    }
                    "implementation" => {
                        let uri = match get_uri() {
                            Ok(u) => u,
                            Err(e) => {
                                output = e;
                                break;
                            }
                        };
                        if let (Some(l), Some(c)) = (line, character) {
                            match client.implementation(uri, l, c).await {
                                Ok(locs) => {
                                    if !locs.is_empty() {
                                        for loc in locs {
                                            output.push_str(&format!(
                                                "Implementation ({}) at: {} {}:{}\n",
                                                lang,
                                                loc.uri.as_str(),
                                                loc.range.start.line,
                                                loc.range.start.character
                                            ));
                                        }
                                    } else {
                                        output.push_str(&format!(
                                            "No implementation ({}) found.\n",
                                            lang
                                        ));
                                    }
                                }
                                Err(e) => output.push_str(&format!(
                                    "Error ({}) getting implementation: {}\n",
                                    lang, e
                                )),
                            }
                        } else {
                            output = "Line and character are required for implementation action."
                                .to_string();
                        }
                    }
                    "hover" => {
                        let uri = match get_uri() {
                            Ok(u) => u,
                            Err(e) => {
                                output = e;
                                break;
                            }
                        };
                        if let (Some(l), Some(c)) = (line, character) {
                            match client.hover(uri, l, c).await {
                                Ok(Some(hover)) => {
                                    output.push_str(&format!("Hover ({}):\n", lang));
                                    match hover.contents {
                                        lsp_types::HoverContents::Scalar(marked_string) => {
                                            match marked_string {
                                                lsp_types::MarkedString::String(s) => {
                                                    output.push_str(&s)
                                                }
                                                lsp_types::MarkedString::LanguageString(ls) => {
                                                    output.push_str(&format!(
                                                        "```{}\n{}\n```",
                                                        ls.language, ls.value
                                                    ))
                                                }
                                            }
                                        }
                                        lsp_types::HoverContents::Array(arr) => {
                                            for ms in arr {
                                                match ms {
                                                    lsp_types::MarkedString::String(s) => {
                                                        output.push_str(&format!("{}\n", s))
                                                    }
                                                    lsp_types::MarkedString::LanguageString(ls) => {
                                                        output.push_str(&format!(
                                                            "```{}\n{}\n```\n",
                                                            ls.language, ls.value
                                                        ))
                                                    }
                                                }
                                            }
                                        }
                                        lsp_types::HoverContents::Markup(content) => {
                                            output.push_str(&content.value);
                                        }
                                    }
                                    output.push('\n');
                                }
                                Ok(None) => output
                                    .push_str(&format!("No hover information ({}) found.\n", lang)),
                                Err(e) => output
                                    .push_str(&format!("Error ({}) getting hover: {}\n", lang, e)),
                            }
                        } else {
                            output =
                                "Line and character are required for hover action.".to_string();
                        }
                    }
                    "document_symbol" => {
                        let uri = match get_uri() {
                            Ok(u) => u,
                            Err(e) => {
                                output = e;
                                break;
                            }
                        };
                        match client.document_symbol(uri).await {
                            Ok(symbols) => {
                                if !symbols.is_empty() {
                                    output.push_str(&format!("Document Symbols ({}):\n", lang));
                                    for sym in symbols {
                                        let kind = format!("{:?}", sym.kind);
                                        output.push_str(&format!(
                                            "  [{}] {} (line {})\n",
                                            kind, sym.name, sym.range.start.line
                                        ));
                                        // TODO: Recursively print children if needed? For now just top level or flat list.
                                    }
                                } else {
                                    output.push_str(&format!(
                                        "No document symbols ({}) found.\n",
                                        lang
                                    ));
                                }
                            }
                            Err(e) => output.push_str(&format!(
                                "Error ({}) getting document symbols: {}\n",
                                lang, e
                            )),
                        }
                    }
                    "workspace_symbol" => {
                        if let Some(q) = query {
                            match client.workspace_symbol(q.to_string()).await {
                                Ok(symbols) => {
                                    if !symbols.is_empty() {
                                        output.push_str(&format!(
                                            "Workspace Symbols ({}) matching '{}':\n",
                                            lang, q
                                        ));
                                        for sym in symbols {
                                            let kind = format!("{:?}", sym.kind);
                                            output.push_str(&format!(
                                                "  [{}] {} ({})\n",
                                                kind,
                                                sym.name,
                                                sym.location.uri.as_str()
                                            ));
                                        }
                                    }
                                }
                                Err(e) => output.push_str(&format!(
                                    "Error ({}) searching workspace symbols: {}\n",
                                    lang, e
                                )),
                            }
                        } else {
                            output =
                                "Query string is required for workspace_symbol action.".to_string();
                        }
                    }
                    "diagnostics" | _ => {
                        let diagnostics = client.diagnostics.lock().await;
                        let mut found_any = false;
                        // If file_path is provided, filter by it.
                        let target_path = file_path_str.unwrap_or("");

                        for (u, diags) in diagnostics.iter() {
                            if target_path.is_empty() || u.path().as_str().contains(target_path) {
                                if !diags.is_empty() {
                                    found_any = true;
                                    output.push_str(&format!(
                                        "Language: {} | File: {}\n",
                                        lang,
                                        u.as_str()
                                    ));
                                    for d in diags {
                                        let severity = format!(
                                            "{:?}",
                                            d.severity
                                                .unwrap_or(lsp_types::DiagnosticSeverity::HINT)
                                        );
                                        output.push_str(&format!(
                                            "  [{}] {}: {}\n",
                                            severity,
                                            d.range.start.line + 1,
                                            d.message
                                        ));
                                    }
                                }
                            }
                        }
                        if !found_any && !output.contains("found") {
                            output.push_str(&format!("No diagnostics found for {}.\n", lang));
                        }
                    }
                }
            }

            if output.is_empty() {
                output = "No results found.".to_string();
            }

            Ok(ToolResult {
                llm_content: Some(output.clone()),
                return_display: None,
                output,
                error: None,
                data: None,
            })
        })
    }
}
