use std::collections::{HashMap, HashSet};

use super::{
    ident_continue, qualifier, Include, Index, Parameter, Symbol, SymbolKind, Token, TokenKind,
};

fn non_type(word: &str) -> bool {
    qualifier(word)
        || matches!(
            word,
            "if" | "else"
                | "while"
                | "for"
                | "do"
                | "switch"
                | "case"
                | "default"
                | "break"
                | "continue"
                | "discard"
                | "return"
                | "struct"
                | "layout"
                | "true"
                | "false"
        )
}

#[derive(Clone, Copy, Debug)]
struct Scope {
    start: usize,
    end: usize,
}

#[derive(Debug)]
struct BlockOwner {
    name: String,
    exposes_members: bool,
    parent_scope: Scope,
}

struct Code<'a> {
    source: &'a str,
    tokens: Vec<Token>,
    pairs: HashMap<usize, usize>,
    scopes: Vec<Scope>,
    global_scope: Scope,
}

impl<'a> Code<'a> {
    fn new(source: &'a str, tokens: &[Token]) -> Self {
        let code: Vec<Token> = tokens
            .iter()
            .filter(|token| !matches!(token.kind, TokenKind::Comment | TokenKind::Directive))
            .cloned()
            .collect();
        let word = |i: usize| code.get(i).map_or("", |token| token.text(source));
        let global_scope = Scope {
            start: 0,
            end: source.len(),
        };
        let mut pairs = HashMap::new();
        let mut stack: Vec<usize> = Vec::new();
        let mut scopes = vec![global_scope];
        for (i, token) in code.iter().enumerate() {
            match token.text(source) {
                "(" | "[" | "{" => stack.push(i),
                ")" | "]" | "}" => {
                    if let Some(&open) = stack.last() {
                        if matches!((word(open), word(i)), ("(", ")") | ("[", "]") | ("{", "}")) {
                            stack.pop();
                            pairs.insert(open, i);
                            if word(open) == "{" {
                                scopes.push(Scope {
                                    start: code[open].start,
                                    end: token.end,
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        for open in stack {
            if word(open) == "{" {
                scopes.push(Scope {
                    start: code[open].start,
                    end: source.len(),
                });
            }
        }
        let mut code = Self {
            source,
            tokens: code,
            pairs,
            scopes,
            global_scope,
        };
        code.add_loop_scopes();
        code
    }

    fn word(&self, at: usize) -> &str {
        self.tokens
            .get(at)
            .map_or("", |token| token.text(self.source))
    }

    fn scope_at(&self, at: usize) -> Scope {
        self.scopes
            .iter()
            .filter(|scope| scope.start <= at && at < scope.end)
            .min_by_key(|scope| scope.end - scope.start)
            .copied()
            .unwrap_or(self.global_scope)
    }

    fn add_loop_scopes(&mut self) {
        let source = self.source;
        let code = &self.tokens;
        let pairs = &self.pairs;
        let word = |i: usize| code.get(i).map_or("", |token| token.text(source));
        for i in 0..code.len() {
            if word(i) == "for" && word(i + 1) == "(" {
                if let Some(&close) = pairs.get(&(i + 1)) {
                    let end = if word(close + 1) == "{" {
                        pairs
                            .get(&(close + 1))
                            .map_or(source.len(), |j| code[*j].end)
                    } else {
                        code[close + 1..]
                            .iter()
                            .find(|t| t.text(source) == ";")
                            .map_or(source.len(), |t| t.end)
                    };
                    self.scopes.push(Scope {
                        start: code[i].start,
                        end,
                    });
                }
            }
        }
    }
}

#[derive(Default)]
struct Blocks {
    owners: HashMap<usize, BlockOwner>,
    consumed: HashSet<usize>,
}

struct Declaration {
    type_start: usize,
    name_at: usize,
    ty: String,
    scope: Scope,
}

pub(super) fn populate(source: &str, index: &mut Index) {
    let code = Code::new(source, &index.tokens);
    index_directives(source, index, code.global_scope);
    let mut blocks = Blocks::default();
    index_blocks(&code, &mut blocks, &mut index.symbols);
    index_declarations(&code, &mut blocks, &mut index.symbols);
    index.symbols.sort_by_key(|symbol| symbol.start);
}

fn index_blocks(context: &Code<'_>, blocks: &mut Blocks, symbols: &mut Vec<Symbol>) {
    let source = context.source;
    let code = &context.tokens;
    let pairs = &context.pairs;
    let word = |at| context.word(at);
    for i in 0..code.len() {
        if word(i) != "{" || i == 0 || code[i - 1].kind != TokenKind::Ident {
            continue;
        }
        let is_struct = i >= 2 && word(i - 2) == "struct";
        let prior = (0..i - 1)
            .rev()
            .take_while(|j| !matches!(word(*j), ";" | "{" | "}"))
            .any(|j| matches!(word(j), "uniform" | "buffer" | "in" | "out"));
        if !is_struct && !prior {
            continue;
        }
        let name = word(i - 1).to_owned();
        let close = pairs.get(&i).copied();
        let after = close.map_or(code.len(), |c| c + 1);
        let has_instance = code.get(after).is_some_and(|t| t.kind == TokenKind::Ident);
        let parent_scope = context.scope_at(code[i - 1].start);
        blocks.owners.insert(
            code[i].start,
            BlockOwner {
                name: name.clone(),
                exposes_members: !is_struct && !has_instance,
                parent_scope,
            },
        );
        let t = &code[i - 1];
        let mut symbol = make_symbol(
            &name,
            &name,
            SymbolKind::Struct,
            t.start,
            t.end,
            parent_scope,
        );
        symbol.signature = if is_struct {
            format!("struct {name}")
        } else {
            format!("interface block {name}")
        };
        symbol.extent_start = if is_struct {
            code[i - 2].start
        } else {
            t.start
        };
        symbol.extent_end = close.map_or(source.len(), |c| code[c].end);
        symbols.push(symbol);
        blocks.consumed.insert(i - 1);
        if has_instance {
            let mut j = after;
            loop {
                let t = &code[j];
                let mut symbol = make_symbol(
                    word(j),
                    &name,
                    SymbolKind::Variable,
                    t.start,
                    t.end,
                    parent_scope,
                );
                blocks.consumed.insert(j);
                j += 1;
                let begin = j;
                while word(j) == "[" {
                    j = pairs.get(&j).map_or(j + 1, |c| c + 1);
                }
                if j > begin {
                    symbol
                        .ty
                        .push_str(&source[code[begin].start..code[j - 1].end]);
                }
                symbol.signature = format!("{} {}", symbol.ty, symbol.name);
                symbols.push(symbol);
                if word(j) != "," || code.get(j + 1).is_none_or(|t| t.kind != TokenKind::Ident) {
                    break;
                }
                j += 1;
            }
        }
    }
}

fn index_declarations(context: &Code<'_>, blocks: &mut Blocks, symbols: &mut Vec<Symbol>) {
    let mut cursor = 0;
    while cursor + 1 < context.tokens.len() {
        let Some(declaration) = declaration_at(context, blocks, cursor) else {
            cursor += 1;
            continue;
        };
        if context.word(declaration.name_at + 1) == "(" {
            cursor = index_function(context, &declaration, symbols).unwrap_or(cursor + 1);
        } else {
            cursor = index_variables(context, &declaration, blocks, symbols).max(cursor + 1);
        }
    }
}

fn declaration_at(context: &Code<'_>, blocks: &Blocks, at: usize) -> Option<Declaration> {
    let code = &context.tokens;
    if blocks.consumed.contains(&at)
        || code[at].kind != TokenKind::Ident
        || non_type(context.word(at))
        || (at > 0 && matches!(context.word(at - 1), "." | "struct"))
    {
        return None;
    }
    let mut name_at = at + 1;
    while context.word(name_at) == "[" {
        let Some(close) = context.pairs.get(&name_at) else {
            break;
        };
        name_at = close + 1;
    }
    if code
        .get(name_at)
        .is_none_or(|token| token.kind != TokenKind::Ident)
        || blocks.consumed.contains(&name_at)
    {
        return None;
    }
    Some(Declaration {
        type_start: at,
        name_at,
        ty: context.source[code[at].start..code[name_at - 1].end].to_owned(),
        scope: context.scope_at(code[name_at].start),
    })
}

fn index_function(
    context: &Code<'_>,
    declaration: &Declaration,
    symbols: &mut Vec<Symbol>,
) -> Option<usize> {
    let code = &context.tokens;
    let name_at = declaration.name_at;
    let &close = context.pairs.get(&(name_at + 1))?;
    if !matches!(context.word(close + 1), "{" | ";") {
        return None;
    }
    let mut symbol = make_symbol(
        context.word(name_at),
        &declaration.ty,
        SymbolKind::Function,
        code[name_at].start,
        code[name_at].end,
        declaration.scope,
    );
    symbol.extent_start = code[declaration.type_start].start;
    symbol.extent_end = if context.word(close + 1) == "{" {
        context
            .pairs
            .get(&(close + 1))
            .map_or(context.source.len(), |end| code[*end].end)
    } else {
        code[close + 1].end
    };
    symbol.parameters = parse_parameters(context, name_at + 2, close);
    symbol.signature = format!(
        "{} {}({})",
        declaration.ty,
        symbol.name,
        symbol
            .parameters
            .iter()
            .map(|parameter| parameter.label.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    if context.word(close + 1) == "{" {
        let body_scope = Scope {
            start: code[close + 1].start,
            end: symbol.extent_end,
        };
        for parameter in &symbol.parameters {
            if !parameter.name.is_empty() {
                let mut parameter_symbol = make_symbol(
                    &parameter.name,
                    &parameter.ty,
                    SymbolKind::Parameter,
                    parameter.start,
                    parameter.end,
                    body_scope,
                );
                parameter_symbol.extent_start = parameter.type_start;
                symbols.push(parameter_symbol);
            }
        }
    }
    symbols.push(symbol);
    Some(close + 1)
}

fn parse_parameters(context: &Code<'_>, mut begin: usize, close: usize) -> Vec<Parameter> {
    let mut parameters = Vec::new();
    let mut cursor = begin;
    while cursor <= close {
        if cursor == close || context.word(cursor) == "," {
            if let Some(parameter) = parse_parameter(context, begin, cursor) {
                parameters.push(parameter);
            }
            begin = cursor + 1;
        }
        if context.word(cursor) == "[" {
            cursor = *context.pairs.get(&cursor).unwrap_or(&cursor);
        }
        cursor += 1;
    }
    parameters
}

fn parse_parameter(context: &Code<'_>, begin: usize, end: usize) -> Option<Parameter> {
    let code = &context.tokens;
    if begin >= end || (end == begin + 1 && context.word(begin) == "void") {
        return None;
    }
    let type_at = (begin..end)
        .find(|at| code[*at].kind == TokenKind::Ident && !qualifier(context.word(*at)))?;
    let mut cursor = type_at + 1;
    let mut array = String::new();
    append_parameter_arrays(context, &mut cursor, end, &mut array);
    let named = (cursor < end && code[cursor].kind == TokenKind::Ident).then_some(cursor);
    let location = named.unwrap_or(type_at);
    if named.is_some() {
        cursor += 1;
    }
    append_parameter_arrays(context, &mut cursor, end, &mut array);
    Some(Parameter {
        name: named.map_or("", |at| context.word(at)).into(),
        ty: format!("{}{array}", context.word(type_at)),
        label: context.source[code[begin].start..code[end - 1].end]
            .trim()
            .into(),
        type_start: code[type_at].start,
        start: code[location].start,
        end: code[location].end,
    })
}

fn append_parameter_arrays(context: &Code<'_>, cursor: &mut usize, end: usize, array: &mut String) {
    while *cursor < end && context.word(*cursor) == "[" {
        let Some(&close) = context.pairs.get(cursor) else {
            break;
        };
        array.push_str(&context.source[context.tokens[*cursor].start..context.tokens[close].end]);
        *cursor = close + 1;
    }
}

fn index_variables(
    context: &Code<'_>,
    declaration: &Declaration,
    blocks: &mut Blocks,
    symbols: &mut Vec<Symbol>,
) -> usize {
    let code = &context.tokens;
    let constant = (0..declaration.type_start)
        .rev()
        .take_while(|at| qualifier(context.word(*at)))
        .any(|at| context.word(at) == "const");
    let mut cursor = declaration.name_at;
    loop {
        if code
            .get(cursor)
            .is_none_or(|token| token.kind != TokenKind::Ident)
        {
            break;
        }
        let token = &code[cursor];
        let mut symbol = make_symbol(
            context.word(cursor),
            &declaration.ty,
            if constant {
                SymbolKind::Constant
            } else {
                SymbolKind::Variable
            },
            token.start,
            token.end,
            declaration.scope,
        );
        symbol.extent_start = code[declaration.type_start].start;
        blocks.consumed.insert(cursor);
        cursor += 1;
        let array_start = cursor;
        while context.word(cursor) == "[" {
            let Some(close) = context.pairs.get(&cursor) else {
                break;
            };
            cursor = close + 1;
        }
        if cursor > array_start {
            symbol
                .ty
                .push_str(&context.source[code[array_start].start..code[cursor - 1].end]);
        }
        symbol.signature = format!(
            "{}{} {}",
            if constant { "const " } else { "" },
            symbol.ty,
            symbol.name
        );
        if let Some(owner) = blocks.owners.get(&declaration.scope.start) {
            if owner.exposes_members {
                symbol.scope_start = owner.parent_scope.start;
                symbol.scope_end = owner.parent_scope.end;
            } else {
                symbol.kind = SymbolKind::Field;
                symbol.owner = Some(owner.name.clone());
            }
        }
        while cursor < code.len() && !matches!(context.word(cursor), "," | ";" | ")" | "}") {
            if matches!(context.word(cursor), "(" | "[" | "{") {
                if let Some(close) = context.pairs.get(&cursor) {
                    cursor = *close;
                }
            }
            cursor += 1;
        }
        symbol.extent_end = code.get(cursor).map_or(token.end, |token| token.end);
        symbols.push(symbol);
        if context.word(cursor) != "," {
            break;
        }
        cursor += 1;
    }
    cursor
}

fn index_directives(source: &str, index: &mut Index, global_scope: Scope) {
    for token in &index.tokens {
        if token.kind != TokenKind::Directive {
            continue;
        }
        let directive = token.text(source).trim_start_matches('#').trim_start();
        if let Some(rest) = directive
            .strip_prefix("include")
            .filter(|s| s.starts_with(char::is_whitespace) || s.starts_with(['"', '<']))
        {
            let rest = rest.trim_start();
            if let Some(first) = rest.as_bytes().first() {
                let end_char = if *first == b'"' {
                    '"'
                } else if *first == b'<' {
                    '>'
                } else {
                    continue;
                };
                if let Some(end) = rest[1..].find(end_char) {
                    let start = token.end - rest.len() + 1;
                    index.includes.push(Include {
                        name: rest[1..end + 1].into(),
                        start,
                        end: start + end,
                        angled: *first == b'<',
                    });
                }
            }
        }
        if let Some(rest) = directive
            .strip_prefix("define")
            .filter(|s| s.starts_with(char::is_whitespace))
        {
            let rest = rest.trim_start();
            let n = rest.bytes().take_while(|b| ident_continue(*b)).count();
            if n == 0 {
                continue;
            }
            let start = token.end - rest.len();
            let mut symbol = make_symbol(
                &rest[..n],
                "macro",
                SymbolKind::Macro,
                start,
                start + n,
                global_scope,
            );
            symbol.signature = format!("#define {}", rest.trim());
            symbol.extent_start = token.start;
            symbol.extent_end = token.end;
            index.symbols.push(symbol);
        }
    }
}

fn make_symbol(
    name: &str,
    ty: &str,
    kind: SymbolKind,
    start: usize,
    end: usize,
    scope: Scope,
) -> Symbol {
    Symbol {
        name: name.into(),
        ty: ty.into(),
        kind,
        start,
        end,
        extent_start: start,
        extent_end: end,
        scope_start: scope.start,
        scope_end: scope.end,
        owner: None,
        signature: format!("{ty} {name}"),
        parameters: Vec::new(),
    }
}
