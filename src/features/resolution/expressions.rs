use super::{
    field, functions, lookup, same_type, select_overload, symbol, symbol_type, valid_swizzle,
    vector, vector_type, Macros,
};
use crate::{
    builtins,
    syntax::{SymbolKind, Token, TokenKind},
    workspace::Graph,
};

const MAX_DEPTH: u8 = 24;

pub(in crate::features) fn expression_type(
    graph: &Graph,
    source_id: usize,
    code: &[&Token],
    end: usize,
) -> Option<String> {
    Inference::new(graph, source_id)
        .operand(code, end, 0)
        .map(|(ty, _)| ty)
}

pub(in crate::features) fn argument_types(
    graph: &Graph,
    source_id: usize,
    code: &[&Token],
) -> Option<Vec<Option<String>>> {
    Inference::new(graph, source_id).arguments(code, 0)
}

pub(super) fn matching_close(code: &[&Token], source: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, token) in code.iter().enumerate().skip(open) {
        match token.text(source) {
            ";" => return None,
            "(" => depth += 1,
            ")" => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn matching_open(
    code: &[&Token],
    source: &str,
    end: usize,
    open: &str,
    close: &str,
) -> Option<usize> {
    let mut depth = 0usize;
    for i in (0..=end).rev() {
        let word = code[i].text(source);
        if word == close {
            depth += 1;
        }
        if word == open {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

struct Inference<'a> {
    graph: &'a Graph,
    source_id: usize,
    text: &'a str,
    expanded_at: Option<usize>,
}

impl<'a> Inference<'a> {
    fn new(graph: &'a Graph, source_id: usize) -> Self {
        Self {
            graph,
            source_id,
            text: &graph.sources[source_id].text,
            expanded_at: None,
        }
    }

    fn source(&self) -> &str {
        self.text
    }

    fn position(&self, offset: usize) -> usize {
        self.expanded_at.unwrap_or(offset)
    }

    fn arguments(&self, code: &[&Token], depth: u8) -> Option<Vec<Option<String>>> {
        if code.is_empty() {
            return Some(Vec::new());
        }
        if depth > MAX_DEPTH {
            return None;
        }
        if self.expanded_at.is_none() {
            let start = code[0].start;
            let end = code.last()?.end;
            let expanded =
                Macros::at(self.graph, self.source_id, start).expand(&self.text[start..end])?;
            let (tokens, _) = crate::syntax::lex(&expanded);
            return Inference {
                graph: self.graph,
                source_id: self.source_id,
                text: &expanded,
                expanded_at: Some(start),
            }
            .arguments(&tokens.iter().collect::<Vec<_>>(), depth + 1);
        }
        let mut arguments = Vec::new();
        let mut nesting = 0usize;
        let mut start = 0;
        for (i, token) in code.iter().enumerate() {
            match token.text(self.source()) {
                "(" | "[" | "{" => nesting += 1,
                ")" | "]" | "}" => nesting = nesting.saturating_sub(1),
                "," if nesting == 0 => {
                    arguments.push(self.expression(&code[start..i], depth + 1));
                    start = i + 1;
                }
                _ => {}
            }
        }
        arguments.push(self.expression(&code[start..], depth + 1));
        Some(arguments)
    }

    fn expression(&self, code: &[&Token], depth: u8) -> Option<String> {
        if depth > MAX_DEPTH || code.is_empty() {
            return None;
        }
        if self.expanded_at.is_none() {
            let start = code[0].start;
            let end = code.last()?.end;
            let expanded =
                Macros::at(self.graph, self.source_id, start).expand(&self.text[start..end])?;
            let (tokens, _) = crate::syntax::lex(&expanded);
            return Inference {
                graph: self.graph,
                source_id: self.source_id,
                text: &expanded,
                expanded_at: Some(start),
            }
            .expression(&tokens.iter().collect::<Vec<_>>(), depth + 1);
        }
        let mut nesting = 0usize;
        let mut split = None;
        let mut question = None;
        let mut conditional_depth = 0usize;
        for (i, token) in code.iter().enumerate() {
            let word = token.text(self.source());
            match word {
                "(" | "[" | "{" => nesting += 1,
                ")" | "]" | "}" => nesting = nesting.checked_sub(1)?,
                "?" if nesting == 0 => {
                    question.get_or_insert(i);
                    conditional_depth += 1;
                }
                ":" if nesting == 0 => {
                    conditional_depth = conditional_depth.checked_sub(1)?;
                    if conditional_depth == 0 {
                        let question = question?;
                        let left = self.expression(&code[question + 1..i], depth + 1)?;
                        let right = self.expression(&code[i + 1..], depth + 1)?;
                        return same_type(&left, &right).then_some(left);
                    }
                }
                _ if nesting == 0 && conditional_depth == 0 && i > 0 => {
                    if let Some(precedence) = precedence(word) {
                        let previous = code[i - 1];
                        let after_operand =
                            matches!(previous.kind, TokenKind::Ident | TokenKind::Number)
                                || matches!(previous.text(self.source()), ")" | "]" | "++" | "--");
                        if after_operand && split.is_none_or(|(_, old)| precedence <= old) {
                            split = Some((i, precedence));
                        }
                    }
                }
                _ => {}
            }
        }
        if nesting != 0 || conditional_depth != 0 {
            return None;
        }
        if let Some((i, _)) = split {
            let left = self.expression(&code[..i], depth + 1)?;
            let right = self.expression(&code[i + 1..], depth + 1)?;
            return binary_type(code[i].text(self.source()), &left, &right);
        }
        match code[0].text(self.source()) {
            "+" | "-" | "~" | "++" | "--" => return self.expression(&code[1..], depth + 1),
            "!" => {
                return (self.expression(&code[1..], depth + 1)? == "bool").then(|| "bool".into())
            }
            _ => {}
        }
        let (ty, start) = self.operand(code, code.len() - 1, depth + 1)?;
        (start == 0).then_some(ty)
    }

    fn operand(&self, code: &[&Token], end: usize, depth: u8) -> Option<(String, usize)> {
        if depth > MAX_DEPTH {
            return None;
        }
        let token = code.get(end)?;
        let word = token.text(self.source());
        if token.kind == TokenKind::Ident && self.expanded_at.is_none() {
            let macros = Macros::at(self.graph, self.source_id, token.start);
            if macros.get(word).is_some() {
                if end > 0 && code[end - 1].text(self.source()) == "." {
                    return None;
                }
                let expanded = macros.expand(word)?;
                let (tokens, _) = crate::syntax::lex(&expanded);
                let last = tokens.len().checked_sub(1)?;
                let (ty, start) = Inference {
                    graph: self.graph,
                    source_id: self.source_id,
                    text: &expanded,
                    expanded_at: Some(token.start),
                }
                .operand(&tokens.iter().collect::<Vec<_>>(), last, depth + 1)?;
                return (start == 0).then_some((ty, end));
            }
        }
        if word == "]" {
            let open = matching_open(code, self.source(), end, "[", "]")?;
            let (ty, start) = self.operand(code, open.checked_sub(1)?, depth + 1)?;
            return indexed_type(&ty).map(|ty| (ty, start));
        }
        if word == ")" {
            let open = matching_open(code, self.source(), end, "(", ")")?;
            if let Some(previous) = open.checked_sub(1).filter(|i| {
                code[*i].kind == TokenKind::Ident
                    && !matches!(
                        code[*i].text(self.source()),
                        "return" | "case" | "if" | "while" | "switch" | "for" | "else" | "do"
                    )
            }) {
                let name = code[previous].text(self.source());
                let arguments = self.arguments(&code[open + 1..end], depth)?;
                if previous >= 2 && code[previous - 1].text(self.source()) == "." {
                    let (owner, start) = self.operand(code, previous - 2, depth + 1)?;
                    return (name == "length"
                        && arguments.is_empty()
                        && (owner.contains('[')
                            || vector(&owner).is_some()
                            || matrix(&owner).is_some()))
                    .then(|| ("int".into(), start));
                }
                return self
                    .call(name, self.position(code[previous].start), &arguments)
                    .map(|ty| (ty, previous));
            }
            return self
                .expression(&code[open + 1..end], depth + 1)
                .map(|ty| (ty, open));
        }
        if matches!(word, "++" | "--") {
            return self.operand(code, end.checked_sub(1)?, depth + 1);
        }
        if token.kind == TokenKind::Number {
            return Some((number_type(word)?.into(), end));
        }
        if token.kind != TokenKind::Ident {
            return None;
        }
        if end >= 2 && code[end - 1].text(self.source()) == "." {
            let (owner, start) = self.operand(code, end - 2, depth + 1)?;
            let ty = if let Some((scalar, size)) = vector(&owner) {
                if !valid_swizzle(word, size) {
                    return None;
                }
                if word.len() == 1 {
                    scalar.into()
                } else {
                    vector_type(scalar, word.len())
                }
            } else {
                symbol_type(self.graph, field(self.graph, &owner, word)?)?
            };
            return Some((ty, start));
        }
        if matches!(word, "true" | "false") {
            return Some(("bool".into(), end));
        }
        if let Some(reference) =
            lookup(self.graph, self.source_id, word, self.position(token.start))
        {
            let s = symbol(self.graph, reference).1;
            if matches!(
                s.kind,
                SymbolKind::Function | SymbolKind::Macro | SymbolKind::Struct
            ) {
                return None;
            }
            return symbol_type(self.graph, reference).map(|ty| (ty, end));
        }
        let stage = self.graph.config.config.stage(&self.graph.sources[0].path);
        builtins::all()
            .iter()
            .find(|b| b.name == word && b.kind == 6 && b.available(stage.as_deref()))
            .map(|b| (b.ty.clone(), end))
    }

    fn call(&self, name: &str, at: usize, arguments: &[Option<String>]) -> Option<String> {
        let expanded = if self.expanded_at.is_none() {
            Macros::at(self.graph, self.source_id, at).expand(name)?
        } else {
            name.to_owned()
        };
        let (tokens, _) = crate::syntax::lex(&expanded);
        if tokens.len() != 1 || tokens[0].kind != TokenKind::Ident {
            return None;
        }
        let name = tokens[0].text(&expanded);
        if let Some(reference) = lookup(self.graph, self.source_id, name, at) {
            let declaration = symbol(self.graph, reference).1;
            if declaration.kind == SymbolKind::Struct {
                return symbol_type(self.graph, reference);
            }
            if declaration.kind != SymbolKind::Function {
                return None;
            }
            let candidates = functions(self.graph, self.source_id, name, at);
            let selected = select_overload(self.graph, &candidates, arguments, false)?;
            return symbol_type(self.graph, selected);
        }
        let stage = self.graph.config.config.stage(&self.graph.sources[0].path);
        let builtin = builtins::all()
            .iter()
            .find(|b| b.name == name && b.available(stage.as_deref()))?;
        if builtin.kind == 7 {
            return Some(builtin.ty.clone());
        }
        builtins::return_type(name, arguments)
    }
}

fn number_type(word: &str) -> Option<&'static str> {
    let lower = word.to_ascii_lowercase();
    if lower.starts_with("0x") {
        let digits = lower.strip_suffix('u').unwrap_or(&lower);
        return (!digits[2..].is_empty() && digits[2..].chars().all(|c| c.is_ascii_hexdigit()))
            .then_some(if lower.ends_with('u') { "uint" } else { "int" });
    }
    if let Some(value) = lower.strip_suffix("lf") {
        value.parse::<f64>().ok()?;
        return Some("double");
    }
    if let Some(value) = lower.strip_suffix('u') {
        value.parse::<u64>().ok()?;
        return Some("uint");
    }
    let value = lower.strip_suffix('f').unwrap_or(&lower);
    value.parse::<f64>().ok()?;
    Some(if lower.contains(['.', 'e']) || lower.ends_with('f') {
        "float"
    } else {
        "int"
    })
}

fn indexed_type(ty: &str) -> Option<String> {
    if let Some(left) = ty.find('[') {
        let right = ty[left..].find(']')? + left;
        return Some(format!("{}{}", &ty[..left], &ty[right + 1..]));
    }
    if let Some((scalar, _)) = vector(ty) {
        return Some(scalar.into());
    }
    let (scalar, _, rows) = matrix(ty)?;
    Some(vector_type(scalar, rows))
}

fn matrix(ty: &str) -> Option<(&'static str, usize, usize)> {
    let (scalar, shape) = if let Some(shape) = ty.strip_prefix("dmat") {
        ("double", shape)
    } else {
        ("float", ty.strip_prefix("mat")?)
    };
    let (columns, rows) = shape.split_once('x').unwrap_or((shape, shape));
    let columns = columns.parse().ok()?;
    let rows = rows.parse().ok()?;
    ((2..=4).contains(&columns) && (2..=4).contains(&rows)).then_some((scalar, columns, rows))
}

fn precedence(operator: &str) -> Option<u8> {
    Some(match operator {
        "," => 1,
        "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | ">>=" => 2,
        "||" => 3,
        "^^" => 4,
        "&&" => 5,
        "|" => 6,
        "^" => 7,
        "&" => 8,
        "==" | "!=" => 9,
        "<" | ">" | "<=" | ">=" => 10,
        "<<" | ">>" => 11,
        "+" | "-" => 12,
        "*" | "/" | "%" => 13,
        _ => return None,
    })
}

fn binary_type(operator: &str, left: &str, right: &str) -> Option<String> {
    if operator == "," {
        return Some(right.into());
    }
    if matches!(
        operator,
        "==" | "!=" | "<" | ">" | "<=" | ">=" | "&&" | "||" | "^^"
    ) {
        return Some("bool".into());
    }
    if operator.ends_with('=') || matches!(operator, "<<" | ">>") {
        return Some(left.into());
    }
    if operator == "*" {
        if let (Some((a, ac, ar)), Some((b, bc, br))) = (matrix(left), matrix(right)) {
            if a != b || ac != br {
                return None;
            }
            return Some(format!(
                "{}mat{bc}x{ar}",
                if a == "double" { "d" } else { "" }
            ));
        }
        if let (Some((a, cols, rows)), Some((b, size))) = (matrix(left), vector(right)) {
            return (a == b && cols == size).then(|| vector_type(a, rows));
        }
        if let (Some((a, size)), Some((b, cols, rows))) = (vector(left), matrix(right)) {
            return (a == b && rows == size).then(|| vector_type(a, cols));
        }
    }
    if same_type(left, right) {
        return Some(left.into());
    }
    if vector(left).is_some_and(|(scalar, _)| scalar == right)
        || matrix(left).is_some_and(|(scalar, _, _)| scalar == right)
    {
        return Some(left.into());
    }
    if vector(right).is_some_and(|(scalar, _)| scalar == left)
        || matrix(right).is_some_and(|(scalar, _, _)| scalar == left)
    {
        return Some(right.into());
    }
    None
}
