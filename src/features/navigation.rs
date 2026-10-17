use super::resolution::{
    argument_types, expression_type, functions, lookup, resolve, select_overload, symbol,
    symbol_type, Macros, SymbolRef,
};
use crate::{
    builtins,
    syntax::{SymbolKind, TokenKind},
    text,
    workspace::{Graph, Workspace},
};
use serde_json::{json, Value};
use std::collections::HashSet;

pub fn hover(graph: &Graph, at: usize) -> Value {
    let source = &graph.sources[0];
    let Some(token) = source.index.identifier_at(at) else {
        return Value::Null;
    };
    let name = token.text(&source.text);
    let stage = graph.config.config.stage(&source.path);
    let macros = Macros::at(graph, 0, at);
    let content = if let Some(definition) = macros.get(name) {
        if !macros.is_valid() || definition.uncertain {
            return Value::Null;
        }
        let location = definition.origin.map_or_else(
            || "Configured in glsld.toml".into(),
            |(source_id, start)| {
                let source = &graph.sources[source_id];
                format!(
                    "{}:{}",
                    source.path.display(),
                    text::position(&source.text, start).line + 1
                )
            },
        );
        format!("{}\n\n{location}", definition.signature)
    } else if let Some(reference) = resolve(graph, 0, token) {
        let (unit, s) = symbol(graph, reference);
        format!(
            "{}\n\n{}:{}",
            s.signature,
            unit.path.display(),
            text::position(&unit.text, s.start).line + 1
        )
    } else if let Some(b) = builtins::all().iter().find(|b| {
        b.name == name
            && b.available(stage.as_deref())
            && lookup(graph, 0, name, token.start).is_none()
    }) {
        format!("{}\n\n{}", b.signature, b.description)
    } else {
        let code: Vec<_> = source.index.code_tokens().collect();
        let inferred = code
            .iter()
            .position(|t| t.start == token.start)
            .and_then(|i| expression_type(graph, 0, &code, i));
        let Some(ty) = inferred else {
            return Value::Null;
        };
        format!("{ty} {name}")
    };
    json!({
        "contents": {"kind": "plaintext", "value": content},
        "range": text::range(&source.text, token.start, token.end)
    })
}

pub fn definition(graph: &Graph, at: usize, type_definition: bool) -> Value {
    let source = &graph.sources[0];
    if !type_definition {
        if let Some(include) = source
            .index
            .includes
            .iter()
            .find(|i| i.start <= at && at <= i.end)
        {
            if let Some(path) = graph.includes.get(&(source.path.clone(), include.start)) {
                return json!({"uri":text::path_to_uri(path),"range":text::Range::default()});
            }
        }
    }
    let Some(token) = source.index.identifier_at(at) else {
        return Value::Null;
    };
    if !type_definition {
        let macros = Macros::at(graph, 0, at);
        if let Some(definition) = macros.get(token.text(&source.text)) {
            if !macros.is_valid() || definition.uncertain {
                return Value::Null;
            }
            return definition.origin.zip(definition.name_end).map_or(
                Value::Null,
                |((source_id, start), end)| {
                    let source = &graph.sources[source_id];
                    json!({"uri":source.uri,"range":text::range(&source.text,start,end)})
                },
            );
        }
    }
    let Some(mut reference) = resolve(graph, 0, token) else {
        return Value::Null;
    };
    if type_definition {
        let Some(ty) = symbol_type(graph, reference) else {
            return Value::Null;
        };
        let base = ty.split('[').next().unwrap_or(&ty);
        let Some(found) = graph
            .sources
            .iter()
            .enumerate()
            .find_map(|(source_id, unit)| {
                unit.index
                    .symbols
                    .iter()
                    .position(|s| s.kind == SymbolKind::Struct && s.name == base)
                    .map(|id| SymbolRef {
                        source_id,
                        symbol_id: id,
                    })
            })
        else {
            return Value::Null;
        };
        reference = found;
    }
    let (unit, s) = symbol(graph, reference);
    json!({"uri":unit.uri,"range":text::range(&unit.text,s.start,s.end)})
}

pub fn references(
    workspace: &Workspace,
    graph: &Graph,
    at: usize,
    include_declaration: bool,
    highlight: bool,
) -> Value {
    let root = &graph.sources[0];
    let Some(token) = root.index.identifier_at(at) else {
        return json!([]);
    };
    let Some(target) = resolve(graph, 0, token) else {
        return json!([]);
    };
    let (declaration, s) = symbol(graph, target);
    let identity = (declaration.path.clone(), s.start);
    let name = s.name.clone();
    let mut locations = Vec::new();
    let mut seen = HashSet::new();
    let mut graphs = vec![graph.clone()];
    if !highlight {
        for source in workspace.open.values() {
            if source.path != root.path {
                if let Some(other) = workspace.graph(&source.uri) {
                    graphs.push(other);
                }
            }
        }
    }
    for graph in &graphs {
        for (source_id, unit) in graph.sources.iter().enumerate() {
            if highlight && unit.path != root.path {
                continue;
            }
            for token in unit
                .index
                .tokens
                .iter()
                .filter(|t| t.kind == TokenKind::Ident && t.text(&unit.text) == name)
            {
                let Some(found) = resolve(graph, source_id, token) else {
                    continue;
                };
                let (decl, s) = symbol(graph, found);
                if (decl.path.clone(), s.start) != identity
                    || (!include_declaration
                        && unit.path == identity.0
                        && token.start == identity.1)
                {
                    continue;
                }
                if !seen.insert((unit.path.clone(), token.start)) {
                    continue;
                }
                let range = text::range(&unit.text, token.start, token.end);
                locations.push(if highlight {
                    json!({"range":range,"kind":1})
                } else {
                    json!({"uri":unit.uri,"range":range})
                });
            }
        }
    }
    json!(locations)
}

pub fn signature_help(graph: &Graph, at: usize) -> Value {
    let source = &graph.sources[0];
    let code: Vec<_> = source
        .index
        .code_tokens()
        .filter(|t| t.start < at)
        .collect();
    let mut nesting = 0usize;
    let mut parameter = 0usize;
    let mut call = None;
    for i in (0..code.len()).rev() {
        match code[i].text(&source.text) {
            ")" | "]" | "}" => nesting += 1,
            "(" if nesting == 0 => {
                call = i.checked_sub(1);
                break;
            }
            "(" | "[" | "{" => nesting = nesting.saturating_sub(1),
            "," if nesting == 0 => parameter += 1,
            _ => {}
        }
    }
    let Some(call) = call else {
        return Value::Null;
    };
    let Some(expanded) =
        Macros::at(graph, 0, code[call].start).expand(code[call].text(&source.text))
    else {
        return Value::Null;
    };
    let (name_tokens, _) = crate::syntax::lex(&expanded);
    if name_tokens.len() != 1 || name_tokens[0].kind != TokenKind::Ident {
        return Value::Null;
    }
    let name = name_tokens[0].text(&expanded);
    let mut signatures = Vec::new();
    let mut seen = HashSet::new();
    let candidates = functions(graph, 0, name, code[call].start);
    let arguments = argument_types(graph, 0, &code[call + 2..]);
    let selected = arguments
        .as_deref()
        .and_then(|arguments| select_overload(graph, &candidates, arguments, true));
    if let Some(arguments) = &arguments {
        parameter = arguments.len().saturating_sub(1);
    }
    let mut active_signature = 0;
    for reference in candidates.iter().copied() {
        let s = symbol(graph, reference).1;
        if seen.insert(s.signature.clone()) {
            if selected.is_some_and(|selected| {
                selected.source_id == reference.source_id
                    && selected.symbol_id == reference.symbol_id
            }) {
                active_signature = signatures.len();
            }
            signatures.push(json!({
                "label": s.signature,
                "parameters": s.parameters.iter().map(|p| json!({"label": p.label})).collect::<Vec<_>>(),
                "activeParameter": parameter.min(s.parameters.len().saturating_sub(1))
            }));
        }
    }
    let stage = graph.config.config.stage(&source.path);
    let user_declared = lookup(graph, 0, name, code[call].start).is_some();
    for b in builtins::all().iter().filter(|b| {
        b.name == name && b.kind == 3 && b.available(stage.as_deref()) && !user_declared
    }) {
        signatures.push(json!({
            "label": b.signature,
            "documentation": b.description,
            "parameters": b.parameters.iter().map(|p| json!({"label": p})).collect::<Vec<_>>(),
            "activeParameter": parameter.min(b.parameters.len().saturating_sub(1))
        }));
    }
    if signatures.is_empty() {
        Value::Null
    } else {
        json!({"signatures":signatures,"activeSignature":active_signature,"activeParameter":signatures[active_signature]["activeParameter"]})
    }
}
