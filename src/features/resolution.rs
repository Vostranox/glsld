use crate::{
    syntax::{Symbol, SymbolKind, Token},
    workspace::{Graph, Source},
};

mod expressions;
mod macros;

pub(super) use expressions::{argument_types, expression_type};
pub(super) use macros::Macros;

#[derive(Clone, Copy, Debug)]
pub(super) struct SymbolRef {
    pub(super) source_id: usize,
    pub(super) symbol_id: usize,
}

pub(super) fn symbol(graph: &Graph, reference: SymbolRef) -> (&Source, &Symbol) {
    let source = &graph.sources[reference.source_id];
    (source, &source.index.symbols[reference.symbol_id])
}

pub(super) fn lookup(graph: &Graph, source_id: usize, name: &str, at: usize) -> Option<SymbolRef> {
    let macros = Macros::at(graph, source_id, at);
    if !macros.is_valid() {
        return None;
    }
    if let Some(definition) = macros.get(name) {
        let (source_id, start) = definition.origin?;
        let symbol_id = graph.sources[source_id]
            .index
            .symbols
            .iter()
            .position(|s| s.kind == SymbolKind::Macro && s.start == start)?;
        return Some(SymbolRef {
            source_id,
            symbol_id,
        });
    }
    let source = &graph.sources[source_id];
    if let Some((id, _)) = source
        .index
        .symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| s.name == name && s.kind != SymbolKind::Macro && s.visible(at))
        .min_by_key(|(_, s)| (s.scope_end - s.scope_start, usize::MAX - s.start))
    {
        return Some(SymbolRef {
            source_id,
            symbol_id: id,
        });
    }
    for (other_source_id, source) in graph.sources.iter().enumerate() {
        if other_source_id == source_id {
            continue;
        }
        if let Some((id, _)) = source
            .index
            .symbols
            .iter()
            .enumerate()
            .find(|(_, s)| s.name == name && s.kind != SymbolKind::Macro && s.global())
        {
            return Some(SymbolRef {
                source_id: other_source_id,
                symbol_id: id,
            });
        }
    }
    None
}

pub(super) fn symbol_type(graph: &Graph, reference: SymbolRef) -> Option<String> {
    let s = symbol(graph, reference).1;
    declared_type(graph, reference.source_id, &s.ty, s.extent_start)
}

fn declared_type(graph: &Graph, source_id: usize, ty: &str, at: usize) -> Option<String> {
    let expanded = Macros::at(graph, source_id, at).expand(ty)?;
    let (tokens, _) = crate::syntax::lex(&expanded);
    let tokens: Vec<_> = tokens
        .iter()
        .skip_while(|t| crate::syntax::qualifier(t.text(&expanded)))
        .collect();
    if tokens.first()?.kind != crate::syntax::TokenKind::Ident {
        return None;
    }
    let mut brackets = 0usize;
    for token in tokens.iter().skip(1) {
        match token.text(&expanded) {
            "[" => brackets += 1,
            "]" => brackets = brackets.checked_sub(1)?,
            _ if brackets == 0 => return None,
            _ => {}
        }
    }
    (brackets == 0).then(|| tokens.iter().map(|t| t.text(&expanded)).collect())
}

fn field(graph: &Graph, owner: &str, name: &str) -> Option<SymbolRef> {
    for (source_id, source) in graph.sources.iter().enumerate() {
        if let Some((id, _)) = source
            .index
            .symbols
            .iter()
            .enumerate()
            .find(|(_, s)| s.owner.as_deref() == Some(owner) && s.name == name)
        {
            return Some(SymbolRef {
                source_id,
                symbol_id: id,
            });
        }
    }
    None
}

pub(super) fn functions(graph: &Graph, source_id: usize, name: &str, at: usize) -> Vec<SymbolRef> {
    if lookup(graph, source_id, name, at)
        .is_some_and(|r| symbol(graph, r).1.kind != SymbolKind::Function)
    {
        return Vec::new();
    }
    let mut candidates: Vec<SymbolRef> = Vec::new();
    for (unit_id, source) in graph.sources.iter().enumerate() {
        for (symbol_id, s) in source.index.symbols.iter().enumerate() {
            if s.name != name
                || s.kind != SymbolKind::Function
                || !(if unit_id == source_id {
                    s.visible(at)
                } else {
                    s.global()
                })
            {
                continue;
            }
            let reference = SymbolRef {
                source_id: unit_id,
                symbol_id,
            };
            let previous = candidates.iter().position(|r| {
                let other = symbol(graph, *r).1;
                symbol_type(graph, *r)
                    .zip(symbol_type(graph, reference))
                    .is_some_and(|(a, b)| same_type(&a, &b))
                    && other.parameters.len() == s.parameters.len()
                    && other.parameters.iter().zip(&s.parameters).all(|(a, b)| {
                        declared_type(graph, r.source_id, &a.ty, a.type_start)
                            .zip(declared_type(graph, unit_id, &b.ty, b.type_start))
                            .is_some_and(|(a, b)| same_type(&a, &b))
                    })
            });
            if let Some(i) = previous {
                if source.text[..s.extent_end].trim_end().ends_with('}') {
                    candidates[i] = reference;
                }
            } else {
                candidates.push(reference);
            }
        }
    }
    candidates
}

pub(super) fn select_overload(
    graph: &Graph,
    candidates: &[SymbolRef],
    arguments: &[Option<String>],
    partial: bool,
) -> Option<SymbolRef> {
    let matching: Vec<_> = candidates
        .iter()
        .copied()
        .filter_map(|reference| {
            let parameters = &symbol(graph, reference).1.parameters;
            if parameters.len() != arguments.len()
                && !(partial && parameters.len() >= arguments.len())
            {
                return None;
            }
            let conversions: Option<Vec<_>> = parameters
                .iter()
                .zip(arguments)
                .map(|(p, argument)| {
                    let Some(argument) = argument else {
                        return Some(Conversion::Exact);
                    };
                    let parameter_type =
                        declared_type(graph, reference.source_id, &p.ty, p.type_start)?;
                    let (tokens, _) = crate::syntax::lex(&p.label);
                    let code = tokens
                        .iter()
                        .filter(|t| {
                            !matches!(
                                t.kind,
                                crate::syntax::TokenKind::Comment
                                    | crate::syntax::TokenKind::Directive
                            )
                        })
                        .map(|t| t.text(&p.label))
                        .collect::<Vec<_>>()
                        .join(" ");
                    let label =
                        Macros::at(graph, reference.source_id, p.type_start).expand(&code)?;
                    let (tokens, _) = crate::syntax::lex(&label);
                    let direction = tokens
                        .iter()
                        .filter(|t| t.kind == crate::syntax::TokenKind::Ident)
                        .map(|t| t.text(&label))
                        .find(|word| matches!(*word, "out" | "inout"));
                    match direction {
                        Some("inout") => {
                            same_type(argument, &parameter_type).then_some(Conversion::Exact)
                        }
                        Some("out") => conversion(&parameter_type, argument),
                        _ => conversion(argument, &parameter_type),
                    }
                })
                .collect();
            Some((reference, conversions?))
        })
        .collect();
    if !partial && matching.len() > 1 && arguments.iter().any(Option::is_none) {
        return None;
    }
    matching
        .iter()
        .enumerate()
        .find_map(|(i, (reference, ranks))| {
            matching
                .iter()
                .enumerate()
                .all(|(j, (_, other))| {
                    i == j
                        || (ranks.iter().zip(other).any(|(a, b)| a.better_than(*b))
                            && !ranks.iter().zip(other).any(|(a, b)| b.better_than(*a)))
                })
                .then_some(*reference)
        })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Conversion {
    Exact,
    FloatToDouble,
    IntegerToFloat,
    IntegerToDouble,
    IntegerToUnsigned,
}

impl Conversion {
    fn better_than(self, other: Self) -> bool {
        self != other
            && (self == Self::Exact
                || (other != Self::Exact && self == Self::FloatToDouble)
                || (self == Self::IntegerToFloat && other == Self::IntegerToDouble))
    }
}

fn conversion(from: &str, to: &str) -> Option<Conversion> {
    if same_type(from, to) {
        return Some(Conversion::Exact);
    }
    if to.starts_with("dmat")
        && same_type(from, &to[1..])
        && crate::builtins::all()
            .iter()
            .any(|b| b.name == from && b.kind == 7)
    {
        return Some(Conversion::FloatToDouble);
    }
    let (from_scalar, from_size) = vector(from).unwrap_or((from, 1));
    let (to_scalar, to_size) = vector(to).unwrap_or((to, 1));
    if from_size != to_size {
        return None;
    }
    match (from_scalar, to_scalar) {
        ("float", "double") => Some(Conversion::FloatToDouble),
        ("int" | "uint", "float") => Some(Conversion::IntegerToFloat),
        ("int" | "uint", "double") => Some(Conversion::IntegerToDouble),
        ("int", "uint") => Some(Conversion::IntegerToUnsigned),
        _ => None,
    }
}

fn same_type(left: &str, right: &str) -> bool {
    fn canonical(ty: &str) -> String {
        let ty: String = ty.chars().filter(|c| !c.is_whitespace()).collect();
        for prefix in ["mat", "dmat"] {
            for size in 2..=4 {
                if ty == format!("{prefix}{size}x{size}") {
                    return format!("{prefix}{size}");
                }
            }
        }
        ty
    }
    left == right || canonical(left) == canonical(right)
}

pub(super) fn resolve(graph: &Graph, source_id: usize, token: &Token) -> Option<SymbolRef> {
    let source = &graph.sources[source_id];
    if let Some((id, _)) = source
        .index
        .symbols
        .iter()
        .enumerate()
        .find(|(_, s)| s.start == token.start)
    {
        return Some(SymbolRef {
            source_id,
            symbol_id: id,
        });
    }
    let code: Vec<_> = source.index.code_tokens().collect();
    let i = code.iter().position(|t| t.start == token.start)?;
    if i >= 2 && code[i - 1].text(&source.text) == "." {
        let owner = expression_type(graph, source_id, &code, i - 2)?;
        return field(graph, &owner, token.text(&source.text));
    }
    if code.get(i + 1).is_some_and(|t| t.text(&source.text) == "(") {
        if let Some(reference) = lookup(graph, source_id, token.text(&source.text), token.start) {
            if matches!(
                symbol(graph, reference).1.kind,
                SymbolKind::Struct | SymbolKind::Macro
            ) {
                return Some(reference);
            }
        }
        let candidates = functions(graph, source_id, token.text(&source.text), token.start);
        let Some(close) = expressions::matching_close(&code, &source.text, i + 1) else {
            return (candidates.len() == 1).then(|| candidates[0]);
        };
        let arguments = argument_types(graph, source_id, &code[i + 2..close])?;
        return select_overload(graph, &candidates, &arguments, false);
    }
    lookup(graph, source_id, token.text(&source.text), token.start)
}

pub(super) fn vector(ty: &str) -> Option<(&'static str, usize)> {
    for (prefix, scalar) in [
        ("vec", "float"),
        ("ivec", "int"),
        ("uvec", "uint"),
        ("bvec", "bool"),
        ("dvec", "double"),
    ] {
        if let Some(n) = ty
            .strip_prefix(prefix)
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|n| (2..=4).contains(n))
        {
            return Some((scalar, n));
        }
    }
    None
}

pub(super) fn vector_type(scalar: &str, size: usize) -> String {
    format!(
        "{}vec{size}",
        match scalar {
            "int" => "i",
            "uint" => "u",
            "bool" => "b",
            "double" => "d",
            _ => "",
        }
    )
}

pub(super) fn valid_swizzle(word: &str, size: usize) -> bool {
    (1..=4).contains(&word.len())
        && ["xyzw", "rgba", "stpq"]
            .iter()
            .any(|set| word.chars().all(|ch| set[..size].contains(ch)))
}
