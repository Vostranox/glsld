use super::{State, ValidationTrigger};
use crate::text::{self, Range};
use serde::Deserialize;
use serde_json::Value;

impl State {
    pub(super) fn notification(
        &mut self,
        method: &str,
        params: &Value,
    ) -> Result<Option<ValidationTrigger>, String> {
        match method {
            "textDocument/didOpen" => {
                let doc = &params["textDocument"];
                let uri = string(doc, "uri")?;
                let text = string(doc, "text")?;
                let version = doc["version"].as_i64().ok_or("Missing document version")?;
                self.workspace.open(uri.into(), text.into(), version)?;
                Ok(Some(ValidationTrigger::Immediate))
            }
            "textDocument/didChange" => self.change_document(params),
            "textDocument/didSave" => {
                let uri = string(&params["textDocument"], "uri")?;
                if let Some(contents) = params["text"].as_str() {
                    let path = text::uri_to_path(uri).ok_or("Invalid document URI")?;
                    if let Some(source) = self.workspace.open.get(&path) {
                        self.workspace.open(
                            uri.into(),
                            contents.into(),
                            source.version.unwrap_or(0),
                        )?;
                    }
                }
                Ok(Some(ValidationTrigger::Immediate))
            }
            "textDocument/didClose" => {
                let uri = string(&params["textDocument"], "uri")?;
                if let Some(path) = text::uri_to_path(uri) {
                    self.workspace.open.remove(&path);
                }
                self.diagnostics.close(uri);
                Ok(Some(ValidationTrigger::Immediate))
            }
            "workspace/didChangeWatchedFiles" | "workspace/didChangeConfiguration" => {
                self.diagnostics.clear_warnings();
                Ok(Some(ValidationTrigger::Immediate))
            }
            "workspace/didChangeWorkspaceFolders" => {
                if let Some(removed) = params["event"]["removed"].as_array() {
                    for folder in removed {
                        if let Some(path) = folder["uri"].as_str().and_then(text::uri_to_path) {
                            self.workspace.roots.retain(|p| p != &path);
                        }
                    }
                }
                if let Some(added) = params["event"]["added"].as_array() {
                    for folder in added {
                        if let Some(path) = folder["uri"].as_str().and_then(text::uri_to_path) {
                            if !self.workspace.roots.contains(&path) {
                                self.workspace.roots.push(path);
                            }
                        }
                    }
                }
                Ok(Some(ValidationTrigger::Immediate))
            }
            "$/cancelRequest" | "$/setTrace" | "initialized" | "textDocument/willSave" => Ok(None),
            _ => Ok(None),
        }
    }

    fn change_document(&mut self, params: &Value) -> Result<Option<ValidationTrigger>, String> {
        let uri = string(&params["textDocument"], "uri")?;
        let path = text::uri_to_path(uri).ok_or("Invalid document URI")?;
        let source = self
            .workspace
            .open
            .get(&path)
            .ok_or("Change received for unopened document")?;
        let version = params["textDocument"]["version"]
            .as_i64()
            .ok_or("Missing document version")?;
        if source.version.is_some_and(|old| version <= old) {
            return Ok(None);
        }
        let changes = params["contentChanges"]
            .as_array()
            .ok_or("Missing contentChanges")?;
        let mut contents = source.text.clone();
        for change in changes {
            ContentChange::parse(change)?.apply(&mut contents)?;
        }
        self.workspace.open(uri.into(), contents, version)?;
        Ok(Some(ValidationTrigger::Edit))
    }
}

fn string<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value[field]
        .as_str()
        .ok_or_else(|| format!("Missing string field {field}"))
}

enum ContentChange<'a> {
    Replace(&'a str),
    Edit { range: Range, text: &'a str },
}

impl<'a> ContentChange<'a> {
    fn parse(value: &'a Value) -> Result<Self, String> {
        let text = string(value, "text")?;
        match value.get("range") {
            Some(range) => {
                let range = Range::deserialize(range)
                    .map_err(|error| format!("Invalid change range: {error}"))?;
                Ok(Self::Edit { range, text })
            }
            None => Ok(Self::Replace(text)),
        }
    }

    fn apply(self, contents: &mut String) -> Result<(), String> {
        match self {
            Self::Replace(text) => {
                *contents = text.into();
                Ok(())
            }
            Self::Edit { range, text } => text::replace(contents, range, text),
        }
    }
}
