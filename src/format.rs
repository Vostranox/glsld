use crate::{
    syntax::{lex, TokenKind},
    text,
};
use serde_json::{json, Value};
use std::collections::HashSet;

pub fn format(source: &str, tab_size: usize, spaces: bool) -> String {
    let (tokens, _) = lex(source);
    let mut protected = HashSet::new();
    for t in &tokens {
        if t.kind == TokenKind::Directive
            || (t.kind == TokenKind::Comment && t.text(source).contains('\n'))
        {
            let first = text::position(source, t.start).line as usize;
            let last = text::position(source, t.end.saturating_sub(1)).line as usize;
            protected.extend(first..=last);
        }
    }
    let unit = if spaces {
        " ".repeat(tab_size.clamp(1, 16))
    } else {
        "\t".into()
    };
    let mut output = String::with_capacity(source.len());
    let mut start = 0;
    let mut depth = 0usize;
    let mut parens = 0usize;
    let mut token_at = 0;
    for (line_no, line) in source.split_inclusive('\n').enumerate() {
        let end = start + line.len();
        let eol = if line.ends_with("\r\n") {
            "\r\n"
        } else if line.ends_with('\n') {
            "\n"
        } else {
            ""
        };
        let content = &line[..line.len() - eol.len()];
        let trim = content.trim();
        let mut first = token_at;
        while first < tokens.len() && tokens[first].end <= start {
            first += 1;
        }
        token_at = first;
        let mut last = first;
        while last < tokens.len() && tokens[last].start < end {
            last += 1;
        }
        let code: Vec<_> = tokens[first..last]
            .iter()
            .filter(|t| t.kind == TokenKind::Punct || t.kind == TokenKind::Ident)
            .collect();
        let closing = code.iter().take_while(|t| t.text(source) == "}").count();
        let continuation = usize::from(parens > 0 && !trim.starts_with(')'));
        if protected.contains(&line_no) || content.trim_end().ends_with('\\') {
            output.push_str(content);
        } else if !trim.is_empty() {
            let label = usize::from(trim.starts_with("case ") || trim.starts_with("default:"));
            output.push_str(
                &unit.repeat((depth.saturating_sub(closing + label) + continuation).min(128)),
            );
            output.push_str(trim);
        }
        output.push_str(eol);
        for token in code {
            match token.text(source) {
                "{" => depth += 1,
                "}" => depth = depth.saturating_sub(1),
                "(" => parens += 1,
                ")" => parens = parens.saturating_sub(1),
                _ => {}
            }
        }
        start = end;
    }
    output
}

pub fn edits(source: &str, options: &Value) -> Value {
    let tab_size = options["tabSize"].as_u64().unwrap_or(4) as usize;
    let spaces = options["insertSpaces"].as_bool().unwrap_or(true);
    let mut formatted = format(source, tab_size, spaces);
    let eol = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    if options["trimFinalNewlines"].as_bool() == Some(true) {
        let had_newline = formatted.ends_with('\n');
        formatted.truncate(formatted.trim_end_matches(['\n', '\r']).len());
        if had_newline {
            formatted.push_str(eol);
        }
    }
    if options["insertFinalNewline"].as_bool() == Some(true) && !formatted.ends_with('\n') {
        formatted.push_str(eol);
    }
    if formatted == source {
        json!([])
    } else {
        json!([{"range":text::range(source,0,source.len()),"newText":formatted}])
    }
}
