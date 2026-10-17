use super::{Lifecycle, State};
use crate::{
    features, format,
    text::{self, Position},
    workspace::Graph,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;

type RpcResult = Result<Value, RequestError>;

pub(super) struct RequestError {
    pub code: i32,
    pub message: String,
}

impl RequestError {
    fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn invalid_params(message: &'static str) -> Self {
        Self::new(-32602, message)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DocumentParams<'a> {
    #[serde(borrow)]
    text_document: DocumentIdentifier<'a>,
}

#[derive(Deserialize)]
struct DocumentIdentifier<'a> {
    uri: &'a str,
}

enum DocumentRequest {
    Symbols,
    FoldingRanges,
    Formatting,
    WillSave,
    AtPosition(PositionRequest),
}

enum PositionRequest {
    Completion,
    Hover,
    Definition,
    TypeDefinition,
    References { include_declaration: bool },
    Highlights,
    SignatureHelp,
}

impl DocumentRequest {
    fn parse(method: &str, params: &Value) -> Result<Self, RequestError> {
        let positioned = match method {
            "textDocument/documentSymbol" => return Ok(Self::Symbols),
            "textDocument/foldingRange" => return Ok(Self::FoldingRanges),
            "textDocument/formatting" => return Ok(Self::Formatting),
            "textDocument/willSaveWaitUntil" => return Ok(Self::WillSave),
            "textDocument/completion" => PositionRequest::Completion,
            "textDocument/hover" => PositionRequest::Hover,
            "textDocument/definition" => PositionRequest::Definition,
            "textDocument/typeDefinition" => PositionRequest::TypeDefinition,
            "textDocument/references" => PositionRequest::References {
                include_declaration: params["context"]["includeDeclaration"]
                    .as_bool()
                    .unwrap_or(false),
            },
            "textDocument/documentHighlight" => PositionRequest::Highlights,
            "textDocument/signatureHelp" => PositionRequest::SignatureHelp,
            _ => {
                return Err(RequestError::new(
                    -32601,
                    format!("Unsupported method: {method}"),
                ))
            }
        };
        Ok(Self::AtPosition(positioned))
    }
}

impl State {
    pub(super) fn request(&mut self, method: &str, params: &Value) -> RpcResult {
        if self.lifecycle == Lifecycle::Shutdown {
            return Err(RequestError::new(-32600, "Server has shut down"));
        }
        if method == "initialize" {
            if self.lifecycle != Lifecycle::AwaitingInitialize {
                return Err(RequestError::new(-32600, "Server is already initialized"));
            }
            return self.initialize(params);
        }
        if self.lifecycle == Lifecycle::AwaitingInitialize {
            return Err(RequestError::new(-32002, "Server is not initialized"));
        }
        if method == "shutdown" {
            self.lifecycle = Lifecycle::Shutdown;
            return Ok(Value::Null);
        }
        if method == "workspace/symbol" {
            return Ok(features::workspace_symbols(
                &self.workspace,
                params["query"].as_str().unwrap_or(""),
            ));
        }
        let request = DocumentRequest::parse(method, params)?;
        // Serde structs also accept sequences; LSP parameters require objects.
        if !params.is_object() || !params["textDocument"].is_object() {
            return Err(RequestError::invalid_params("Missing document URI"));
        }
        let document = DocumentParams::deserialize(params)
            .map_err(|_| RequestError::invalid_params("Missing document URI"))?;
        let graph = self
            .workspace
            .graph(document.text_document.uri)
            .ok_or_else(|| {
                RequestError::invalid_params("Document is unavailable or is not a local file URI")
            })?;
        let source = &graph.sources[0];
        Ok(match request {
            DocumentRequest::Symbols => {
                if self.hierarchical_symbols {
                    features::document_symbols(source)
                } else {
                    features::flat_document_symbols(source)
                }
            }
            DocumentRequest::FoldingRanges => features::folding_ranges(source),
            DocumentRequest::Formatting => format::edits(&source.text, &params["options"]),
            DocumentRequest::WillSave => {
                if graph.config.config.format_on_save {
                    format::edits(&source.text, &Value::Null)
                } else {
                    json!([])
                }
            }
            DocumentRequest::AtPosition(request) => {
                let position = Position::deserialize(&params["position"])
                    .map_err(|_| RequestError::invalid_params("Invalid position"))?;
                let at = text::offset(&source.text, position).ok_or_else(|| {
                    RequestError::invalid_params("Position is outside the document")
                })?;
                self.position_request(request, &graph, at)
            }
        })
    }

    fn position_request(&self, request: PositionRequest, graph: &Graph, at: usize) -> Value {
        match request {
            PositionRequest::Completion => features::completion(&self.workspace, graph, at),
            PositionRequest::Hover => features::hover(graph, at),
            PositionRequest::Definition => features::definition(graph, at, false),
            PositionRequest::TypeDefinition => features::definition(graph, at, true),
            PositionRequest::References {
                include_declaration,
            } => features::references(&self.workspace, graph, at, include_declaration, false),
            PositionRequest::Highlights => {
                features::references(&self.workspace, graph, at, true, true)
            }
            PositionRequest::SignatureHelp => features::signature_help(graph, at),
        }
    }

    fn initialize(&mut self, params: &Value) -> RpcResult {
        if let Some(folders) = params["workspaceFolders"].as_array() {
            self.workspace.roots = folders
                .iter()
                .filter_map(|f| f["uri"].as_str().and_then(text::uri_to_path))
                .collect();
        }
        if self.workspace.roots.is_empty() {
            if let Some(path) = params["rootUri"]
                .as_str()
                .and_then(text::uri_to_path)
                .or_else(|| params["rootPath"].as_str().map(PathBuf::from))
            {
                self.workspace.roots.push(path);
            }
        }
        self.lifecycle = Lifecycle::Running;
        self.hierarchical_symbols = params["capabilities"]["textDocument"]["documentSymbol"]
            ["hierarchicalDocumentSymbolSupport"]
            .as_bool()
            .unwrap_or(false);
        Ok(json!({
            "capabilities": {
                "positionEncoding":"utf-16",
                "textDocumentSync":{"openClose":true,"change":2,"willSaveWaitUntil":true,"save":{"includeText":true}},
                "completionProvider":{"triggerCharacters":[".","\"","/","<"],"resolveProvider":false},
                "hoverProvider":true,"definitionProvider":true,"typeDefinitionProvider":true,
                "referencesProvider":true,"documentHighlightProvider":true,
                "documentSymbolProvider":true,"workspaceSymbolProvider":true,
                "signatureHelpProvider":{"triggerCharacters":["(",","],"retriggerCharacters":[","]},
                "documentFormattingProvider":true,"foldingRangeProvider":true,
                "workspace":{"workspaceFolders":{"supported":true,"changeNotifications":true}}
            },
            "serverInfo":{"name":"glsld","version":env!("CARGO_PKG_VERSION")}
        }))
    }
}
