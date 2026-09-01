use lsp_types::{
    notification::{Initialized, Notification},
    request::{Initialize, Request},
    ClientCapabilities, Diagnostic, InitializeParams, Uri,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::Stdio;
use std::str::FromStr;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, Mutex};
use url::Url;

pub struct LspClient {
    writer_tx: mpsc::Sender<String>,
    pending_requests: Arc<Mutex<HashMap<u64, mpsc::Sender<Value>>>>,
    pub diagnostics: Arc<Mutex<HashMap<Uri, Vec<Diagnostic>>>>,
    _child: Child,
}

impl LspClient {
    pub async fn new(server_path: &str, args: &[String]) -> Result<Self, String> {
        let mut child = Command::new(server_path)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to spawn LSP server: {}", e))?;

        let mut stdin = child.stdin.take().ok_or("Failed to open stdin")?;
        let stdout = child.stdout.take().ok_or("Failed to open stdout")?;

        let (writer_tx, mut writer_rx) = mpsc::channel::<String>(32);
        let pending_requests = Arc::new(Mutex::new(HashMap::<u64, mpsc::Sender<Value>>::new()));
        let diagnostics = Arc::new(Mutex::new(HashMap::new()));

        // Writer task
        tokio::spawn(async move {
            while let Some(msg) = writer_rx.recv().await {
                let frame = format!("Content-Length: {}\r\n\r\n{}", msg.len(), msg);
                if stdin.write_all(frame.as_bytes()).await.is_err() {
                    break;
                }
                let _ = stdin.flush().await;
            }
        });

        // Reader task
        let pending_clone = pending_requests.clone();
        let diag_clone = diagnostics.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();

            loop {
                line.clear();
                if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                    break;
                }

                if line.starts_with("Content-Length: ") {
                    let len: usize = line["Content-Length: ".len()..].trim().parse().unwrap_or(0);

                    // Skip empty line
                    line.clear();
                    reader.read_line(&mut line).await.ok();

                    let mut buffer = vec![0u8; len];
                    reader.read_exact(&mut buffer).await.ok();

                    if let Ok(value) = serde_json::from_slice::<Value>(&buffer) {
                        if let Some(id) = value.get("id").and_then(|v| v.as_u64()) {
                            let mut pending = pending_clone.lock().await;
                            if let Some(tx) = pending.remove(&id) {
                                let _ = tx.send(value).await;
                            }
                        } else if let Some(method) = value.get("method").and_then(|v| v.as_str()) {
                            if method == "textDocument/publishDiagnostics" {
                                if let Some(params) = value.get("params") {
                                    if let (Some(uri_str), Some(diags)) = (
                                        params.get("uri").and_then(|v| v.as_str()),
                                        params.get("diagnostics").and_then(|v| v.as_array()),
                                    ) {
                                        if let Ok(uri) = Uri::from_str(uri_str) {
                                            let parsed_diags: Vec<Diagnostic> = diags
                                                .iter()
                                                .filter_map(|d| {
                                                    serde_json::from_value(d.clone()).ok()
                                                })
                                                .collect();
                                            diag_clone.lock().await.insert(uri, parsed_diags);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        let client = Self {
            writer_tx,
            pending_requests,
            diagnostics,
            _child: child,
        };

        // Perform initialization
        client.initialize().await?;

        Ok(client)
    }

    async fn initialize(&self) -> Result<(), String> {
        let root_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"));
        let root_url =
            Url::from_directory_path(root_dir).map_err(|_| "Invalid root path".to_string())?;
        let root_uri =
            Uri::from_str(root_url.as_str()).map_err(|e| format!("Invalid root URI: {}", e))?;

        let params = InitializeParams {
            process_id: Some(std::process::id()),
            capabilities: ClientCapabilities::default(),
            workspace_folders: Some(vec![lsp_types::WorkspaceFolder {
                uri: root_uri,
                name: "root".to_string(),
            }]),
            ..Default::default()
        };

        // ID 0 for initialize
        let _result = self.send_request::<Initialize>(0, params).await?;

        self.send_notification::<Initialized>(lsp_types::InitializedParams {})
            .await?;

        Ok(())
    }

    pub async fn send_request<R>(&self, id: u64, params: R::Params) -> Result<Value, String>
    where
        R: Request,
    {
        let (tx, mut rx) = mpsc::channel(1);
        {
            let mut pending = self.pending_requests.lock().await;
            pending.insert(id, tx);
        }

        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": R::METHOD,
            "params": params
        });

        self.writer_tx
            .send(req.to_string())
            .await
            .map_err(|e| e.to_string())?;

        rx.recv()
            .await
            .ok_or("No response from LSP server".to_string())
    }

    pub async fn send_notification<N>(&self, params: N::Params) -> Result<(), String>
    where
        N: Notification,
    {
        let notif = json!({
            "jsonrpc": "2.0",
            "method": N::METHOD,
            "params": params
        });

        self.writer_tx
            .send(notif.to_string())
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn goto_definition(
        &self,
        url: Url,
        line: u32,
        character: u32,
    ) -> Result<Vec<lsp_types::Location>, String> {
        let uri = Uri::from_str(url.as_str()).map_err(|e| format!("Invalid URI: {}", e))?;
        let params = lsp_types::GotoDefinitionParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: lsp_types::TextDocumentIdentifier { uri },
                position: lsp_types::Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        // Random ID for request
        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let response = self
            .send_request::<lsp_types::request::GotoDefinition>(id, params)
            .await?;

        if let Ok(location) = serde_json::from_value::<lsp_types::Location>(response.clone()) {
            Ok(vec![location])
        } else if let Ok(locations) =
            serde_json::from_value::<Vec<lsp_types::Location>>(response.clone())
        {
            Ok(locations)
        } else if let Ok(links) = serde_json::from_value::<Vec<lsp_types::LocationLink>>(response) {
            // Convert LocationLink to Location (simplified)
            Ok(links
                .into_iter()
                .map(|l| lsp_types::Location {
                    uri: l.target_uri,
                    range: l.target_range,
                })
                .collect())
        } else {
            // It might be null if no definition found
            Ok(vec![])
        }
    }

    pub async fn hover(
        &self,
        url: Url,
        line: u32,
        character: u32,
    ) -> Result<Option<lsp_types::Hover>, String> {
        let uri = Uri::from_str(url.as_str()).map_err(|e| format!("Invalid URI: {}", e))?;
        let params = lsp_types::HoverParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: lsp_types::TextDocumentIdentifier { uri },
                position: lsp_types::Position { line, character },
            },
            work_done_progress_params: Default::default(),
        };

        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let response = self
            .send_request::<lsp_types::request::HoverRequest>(id, params)
            .await?;

        if response.is_null() {
            return Ok(None);
        }

        serde_json::from_value(response).map_err(|e| e.to_string())
    }

    pub async fn references(
        &self,
        url: Url,
        line: u32,
        character: u32,
    ) -> Result<Vec<lsp_types::Location>, String> {
        let uri = Uri::from_str(url.as_str()).map_err(|e| format!("Invalid URI: {}", e))?;
        let params = lsp_types::ReferenceParams {
            text_document_position: lsp_types::TextDocumentPositionParams {
                text_document: lsp_types::TextDocumentIdentifier { uri },
                position: lsp_types::Position { line, character },
            },
            context: lsp_types::ReferenceContext {
                include_declaration: true,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let response = self
            .send_request::<lsp_types::request::References>(id, params)
            .await?;

        if response.is_null() {
            return Ok(vec![]);
        }

        serde_json::from_value(response).map_err(|e| e.to_string())
    }

    pub async fn document_symbol(
        &self,
        url: Url,
    ) -> Result<Vec<lsp_types::DocumentSymbol>, String> {
        let uri = Uri::from_str(url.as_str()).map_err(|e| format!("Invalid URI: {}", e))?;
        let params = lsp_types::DocumentSymbolParams {
            text_document: lsp_types::TextDocumentIdentifier { uri },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let response = self
            .send_request::<lsp_types::request::DocumentSymbolRequest>(id, params)
            .await?;

        if response.is_null() {
            return Ok(vec![]);
        }

        // Response can be DocumentSymbol[] or SymbolInformation[]
        if let Ok(symbols) =
            serde_json::from_value::<Vec<lsp_types::DocumentSymbol>>(response.clone())
        {
            Ok(symbols)
        } else if let Ok(infos) =
            serde_json::from_value::<Vec<lsp_types::SymbolInformation>>(response)
        {
            // Convert SymbolInformation to DocumentSymbol (lossy but usable structure)
            // Note: This is a simplification. DocumentSymbol is hierarchical, SymbolInformation is flat.
            // For now, we just map them flatly.
            Ok(infos
                .into_iter()
                .map(|info| {
                    #[allow(deprecated)]
                    lsp_types::DocumentSymbol {
                        name: info.name,
                        detail: None,
                        kind: info.kind,
                        tags: info.tags,
                        deprecated: info.deprecated,
                        range: info.location.range,
                        selection_range: info.location.range,
                        children: None,
                    }
                })
                .collect())
        } else {
            Ok(vec![])
        }
    }

    pub async fn workspace_symbol(
        &self,
        query: String,
    ) -> Result<Vec<lsp_types::SymbolInformation>, String> {
        let params = lsp_types::WorkspaceSymbolParams {
            query,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let response = self
            .send_request::<lsp_types::request::WorkspaceSymbolRequest>(id, params)
            .await?;

        if response.is_null() {
            return Ok(vec![]);
        }

        serde_json::from_value(response).map_err(|e| e.to_string())
    }

    pub async fn type_definition(
        &self,
        url: Url,
        line: u32,
        character: u32,
    ) -> Result<Vec<lsp_types::Location>, String> {
        let uri = Uri::from_str(url.as_str()).map_err(|e| format!("Invalid URI: {}", e))?;
        let params = lsp_types::GotoDefinitionParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: lsp_types::TextDocumentIdentifier { uri },
                position: lsp_types::Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let response = self
            .send_request::<lsp_types::request::GotoTypeDefinition>(id, params)
            .await?;

        if let Ok(location) = serde_json::from_value::<lsp_types::Location>(response.clone()) {
            Ok(vec![location])
        } else if let Ok(locations) =
            serde_json::from_value::<Vec<lsp_types::Location>>(response.clone())
        {
            Ok(locations)
        } else if let Ok(links) = serde_json::from_value::<Vec<lsp_types::LocationLink>>(response) {
            Ok(links
                .into_iter()
                .map(|l| lsp_types::Location {
                    uri: l.target_uri,
                    range: l.target_range,
                })
                .collect())
        } else {
            Ok(vec![])
        }
    }

    pub async fn implementation(
        &self,
        url: Url,
        line: u32,
        character: u32,
    ) -> Result<Vec<lsp_types::Location>, String> {
        let uri = Uri::from_str(url.as_str()).map_err(|e| format!("Invalid URI: {}", e))?;
        let params = lsp_types::GotoDefinitionParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: lsp_types::TextDocumentIdentifier { uri },
                position: lsp_types::Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let response = self
            .send_request::<lsp_types::request::GotoImplementation>(id, params)
            .await?;

        if let Ok(location) = serde_json::from_value::<lsp_types::Location>(response.clone()) {
            Ok(vec![location])
        } else if let Ok(locations) =
            serde_json::from_value::<Vec<lsp_types::Location>>(response.clone())
        {
            Ok(locations)
        } else if let Ok(links) = serde_json::from_value::<Vec<lsp_types::LocationLink>>(response) {
            Ok(links
                .into_iter()
                .map(|l| lsp_types::Location {
                    uri: l.target_uri,
                    range: l.target_range,
                })
                .collect())
        } else {
            Ok(vec![])
        }
    }
}
