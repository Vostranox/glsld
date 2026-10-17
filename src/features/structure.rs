use crate::{
    syntax::{Symbol, SymbolKind, TokenKind},
    text,
    workspace::{Source, Workspace},
};
use serde_json::{json, Value};
use std::collections::HashSet;

pub fn document_symbols(source: &Source) -> Value {
    fn item(source: &Source, s: &Symbol) -> Value {
        json!({
            "name": s.name,
            "detail": s.signature,
            "kind": s.kind.document(),
            "range": text::range(&source.text, s.extent_start, s.extent_end),
            "selectionRange": text::range(&source.text, s.start, s.end)
        })
    }
    let mut result = Vec::new();
    for s in source.index.symbols.iter().filter(|s| s.global()) {
        let mut value = item(source, s);
        if s.kind == SymbolKind::Struct {
            value["children"] = json!(source
                .index
                .symbols
                .iter()
                .filter(|f| f.owner.as_deref() == Some(&s.name))
                .map(|f| item(source, f))
                .collect::<Vec<_>>());
        } else if s.kind == SymbolKind::Function {
            value["children"] = json!(source
                .index
                .symbols
                .iter()
                .filter(|v| !v.global()
                    && v.owner.is_none()
                    && s.extent_start <= v.start
                    && v.end <= s.extent_end)
                .map(|v| item(source, v))
                .collect::<Vec<_>>());
        }
        result.push(value);
    }
    json!(result)
}

pub fn flat_document_symbols(source: &Source) -> Value {
    json!(source
        .index
        .symbols
        .iter()
        .map(|s| {
            let mut value = json!({
                "name": s.name,
                "kind": s.kind.document(),
                "location": {
                    "uri": source.uri,
                    "range": text::range(&source.text, s.start, s.end)
                }
            });
            if let Some(owner) = &s.owner {
                value["containerName"] = json!(owner);
            }
            value
        })
        .collect::<Vec<_>>())
}

pub fn workspace_symbols(workspace: &Workspace, query: &str) -> Value {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    let query = query.to_ascii_lowercase();
    for source in workspace.open.values() {
        if let Some(graph) = workspace.graph(&source.uri) {
            for unit in graph.sources {
                if !seen.insert(unit.path.clone()) {
                    continue;
                }
                for s in unit
                    .index
                    .symbols
                    .iter()
                    .filter(|s| s.global() && s.name.to_ascii_lowercase().contains(&query))
                {
                    result.push(json!({
                        "name": s.name,
                        "kind": s.kind.document(),
                        "location": {
                            "uri": unit.uri,
                            "range": text::range(&unit.text, s.start, s.end)
                        }
                    }));
                }
            }
        }
    }
    json!(result)
}

pub fn folding_ranges(source: &Source) -> Value {
    let mut stack = Vec::new();
    let mut ranges = Vec::new();
    for token in &source.index.tokens {
        let start = text::position(&source.text, token.start).line;
        let end = text::position(&source.text, token.end).line;
        if token.kind == TokenKind::Comment && end > start {
            ranges.push(json!({"startLine":start,"endLine":end,"kind":"comment"}));
        }
        if token.text(&source.text) == "{" {
            stack.push(start);
        }
        if token.text(&source.text) == "}" {
            if let Some(first) = stack.pop() {
                if start > first {
                    ranges.push(json!({"startLine":first,"endLine":start}));
                }
            }
        }
    }
    json!(ranges)
}
