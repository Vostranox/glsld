use super::resolution::{expression_type, valid_swizzle, vector, vector_type, Macros};
use crate::{
    builtins,
    syntax::{self, SymbolKind, TokenKind},
    text,
    workspace::{Graph, Workspace},
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

struct CompletionItems<'a> {
    prefix: &'a str,
    prefix_lower: String,
    edit_range: text::Range,
    items: BTreeMap<String, Value>,
    incomplete: bool,
}

impl<'a> CompletionItems<'a> {
    fn new(prefix: &'a str, edit_range: text::Range) -> Self {
        Self {
            prefix,
            prefix_lower: prefix.to_ascii_lowercase(),
            edit_range,
            items: BTreeMap::new(),
            incomplete: false,
        }
    }

    fn add(&mut self, name: &str, kind: u32, detail: &str, documentation: &str) {
        if !name.to_ascii_lowercase().starts_with(&self.prefix_lower) {
            return;
        }
        self.items.entry(name.to_owned()).or_insert_with(|| {
            json!({
                "label": name,
                "kind": kind,
                "detail": detail,
                "documentation": {"kind": "plaintext", "value": documentation},
                "textEdit": {"range": self.edit_range, "newText": name},
                "sortText": format!("{}{name}", if name.starts_with("gl_") { "1" } else { "0" })
            })
        });
    }

    fn finish(self) -> Value {
        json!({
            "isIncomplete": self.incomplete,
            "items": self.items.into_values().collect::<Vec<_>>()
        })
    }
}

pub fn completion(_workspace: &Workspace, graph: &Graph, at: usize) -> Value {
    let source = &graph.sources[0];
    if let Some(result) = include_completion(graph, at) {
        return result;
    }
    if source.index.tokens.iter().any(|token| {
        token.start <= at
            && at < token.end
            && matches!(token.kind, TokenKind::Comment | TokenKind::String)
    }) {
        return json!([]);
    }
    let mut start = at;
    while start > 0 && syntax::ident_continue(source.text.as_bytes()[start - 1]) {
        start -= 1;
    }
    let prefix = &source.text[start..at];
    let mut items = CompletionItems::new(prefix, text::range(&source.text, start, at));
    let code: Vec<_> = source
        .index
        .code_tokens()
        .filter(|token| token.end <= start)
        .collect();
    if code
        .last()
        .is_some_and(|token| token.text(&source.text) == ".")
    {
        if let Some(owner) = code
            .len()
            .checked_sub(2)
            .and_then(|end| expression_type(graph, 0, &code, end))
        {
            complete_members(graph, &owner, &mut items);
        }
    } else {
        complete_scope(graph, at, &mut items);
    }
    items.finish()
}

fn complete_members(graph: &Graph, owner: &str, items: &mut CompletionItems<'_>) {
    if let Some((scalar, size)) = vector(owner) {
        complete_swizzle(scalar, size, items);
    } else if owner.contains('[') {
        items.add("length", 2, "int length()", "Returns the array length.");
    } else {
        for source in &graph.sources {
            for member in source
                .index
                .symbols
                .iter()
                .filter(|symbol| symbol.owner.as_deref() == Some(owner))
            {
                items.add(
                    &member.name,
                    5,
                    &member.signature,
                    "Structure or interface block member.",
                );
            }
        }
    }
}

fn complete_swizzle(scalar: &str, size: usize, items: &mut CompletionItems<'_>) {
    // Offer one additional component at a time. Mark the response incomplete so
    // clients ask again after typing instead of filtering the bare-dot list.
    items.incomplete = true;
    let prefix = items.prefix;
    if !prefix.is_empty() && !valid_swizzle(prefix, size) {
        return;
    }
    let mut add = |word: &str| {
        let result = if word.len() == 1 {
            scalar.into()
        } else {
            vector_type(scalar, word.len())
        };
        items.add(word, 5, &result, "Vector component selection.");
    };
    if !prefix.is_empty() {
        add(prefix);
    }
    if prefix.len() < 4 {
        for alphabet in ["xyzw", "rgba", "stpq"] {
            let letters = &alphabet[..size];
            if prefix.chars().all(|ch| letters.contains(ch)) {
                for ch in letters.chars() {
                    add(&format!("{prefix}{ch}"));
                }
            }
        }
    }
}

fn complete_scope(graph: &Graph, at: usize, items: &mut CompletionItems<'_>) {
    let source = &graph.sources[0];
    let macros = Macros::at(graph, 0, at);
    for definition in macros.iter() {
        items.add(
            &definition.name,
            14,
            &definition.signature,
            "Preprocessor macro.",
        );
    }
    let mut locals: Vec<_> = source
        .index
        .symbols
        .iter()
        .filter(|symbol| {
            symbol.kind != SymbolKind::Macro
                && macros.get(&symbol.name).is_none()
                && symbol.visible(at)
        })
        .collect();
    // Match resolution::lookup: the innermost scope and latest declaration win.
    locals.sort_by_key(|symbol| {
        (
            symbol.scope_end - symbol.scope_start,
            usize::MAX - symbol.start,
        )
    });
    for symbol in locals {
        items.add(
            &symbol.name,
            symbol.kind.completion(),
            &symbol.signature,
            "Declared in this shader.",
        );
    }
    for source in graph.sources.iter().skip(1) {
        for symbol in source.index.symbols.iter().filter(|symbol| {
            symbol.kind != SymbolKind::Macro
                && macros.get(&symbol.name).is_none()
                && symbol.global()
        }) {
            items.add(
                &symbol.name,
                symbol.kind.completion(),
                &symbol.signature,
                &format!("Declared in {}.", source.path.display()),
            );
        }
    }
    let stage = graph.config.config.stage(&source.path);
    for builtin in builtins::all().iter().filter(|builtin| {
        macros.get(&builtin.name).is_none() && builtin.available(stage.as_deref())
    }) {
        items.add(
            &builtin.name,
            builtin.kind,
            &builtin.signature,
            &builtin.description,
        );
    }
}

fn include_completion(graph: &Graph, at: usize) -> Option<Value> {
    let source = &graph.sources[0];
    let line_start = source.text[..at].rfind('\n').map_or(0, |i| i + 1);
    let line = source.text[line_start..at]
        .trim_start()
        .strip_prefix('#')?
        .trim_start();
    let rest = line.strip_prefix("include")?.trim_start();
    let angled = rest.starts_with('<');
    let typed = rest.strip_prefix(if angled { '<' } else { '"' })?;
    if typed.contains(['"', '>']) {
        return None;
    }
    let (directory, prefix) = typed
        .rsplit_once('/')
        .map_or(("", typed), |(d, p)| (&typed[..d.len() + 1], p));
    let mut roots = graph.config.config.include_paths.clone();
    if !angled {
        roots.insert(0, source.path.parent()?.to_path_buf());
    }
    let mut items = BTreeMap::new();
    for root in roots {
        if let Ok(entries) = std::fs::read_dir(root.join(directory)) {
            for entry in entries.flatten().take(4096) {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.starts_with(prefix) || name.starts_with('.') {
                    continue;
                }
                let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
                let new_text = format!("{name}{}", if is_dir { "/" } else { "" });
                items.insert(
                    name.clone(),
                    json!({
                        "label": new_text,
                        "kind": if is_dir { 19 } else { 17 },
                        "textEdit": {
                            "range": text::range(&source.text, at - prefix.len(), at),
                            "newText": new_text
                        }
                    }),
                );
            }
        }
    }
    Some(json!({"isIncomplete": false, "items":items.into_values().collect::<Vec<_>>()}))
}
