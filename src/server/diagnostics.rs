use super::send;
use crate::{validate::Validation, workspace::Workspace};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    io::Write,
};

#[derive(Default)]
pub(super) struct Diagnostics {
    reports: BTreeMap<String, BTreeMap<String, Vec<Value>>>,
    published: HashSet<String>,
    warnings: HashSet<String>,
}

impl Diagnostics {
    pub fn close(&mut self, uri: &str) {
        self.reports.remove(uri);
    }

    pub fn clear_warnings(&mut self) {
        self.warnings.clear();
    }

    pub fn accept(
        &mut self,
        workspace: &Workspace,
        results: Vec<Validation>,
        writer: &mut impl Write,
    ) -> Result<(), String> {
        for result in results {
            if workspace
                .source(&result.uri)
                .is_none_or(|source| source.version != result.version)
            {
                continue;
            }
            if let Some(warning) = result.warning {
                if self.warnings.insert(warning.clone()) {
                    send(
                        writer,
                        json!({"jsonrpc":"2.0","method":"window/logMessage","params":{"type":2,"message":warning}}),
                    )?;
                }
            }
            self.reports.insert(result.uri, result.diagnostics);
        }
        Ok(())
    }

    pub fn publish(
        &mut self,
        workspace: &Workspace,
        writer: &mut impl Write,
    ) -> Result<(), String> {
        let mut merged: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for report in self.reports.values() {
            for (uri, diagnostics) in report {
                let collected = merged.entry(uri.clone()).or_default();
                for diagnostic in diagnostics {
                    if !collected.contains(diagnostic) {
                        collected.push(diagnostic.clone());
                    }
                }
            }
        }
        let current: HashSet<_> = merged.keys().cloned().collect();
        for uri in &self.published {
            merged.entry(uri.clone()).or_default();
        }
        for (uri, diagnostics) in merged {
            let mut params = json!({"uri":uri,"diagnostics":diagnostics});
            if let Some(version) = workspace.source(&uri).and_then(|s| s.version) {
                params["version"] = json!(version);
            }
            send(
                writer,
                json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":params}),
            )?;
        }
        self.published = current;
        Ok(())
    }
}
