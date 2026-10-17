use super::{Problem, Token, TokenKind};

pub fn lex(source: &str) -> (Vec<Token>, Vec<Problem>) {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut problems = Vec::new();
    let mut i = 0;
    let mut line_start = true;
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            if bytes[i] == b'\n' {
                line_start = true;
            }
            i += 1;
            continue;
        }
        let start = i;
        let kind;
        if bytes[i..].starts_with(b"//") {
            i = source[i..].find('\n').map_or(bytes.len(), |n| i + n);
            kind = TokenKind::Comment;
        } else if bytes[i..].starts_with(b"/*") {
            if let Some(n) = source[i + 2..].find("*/") {
                i += n + 4;
            } else {
                i = bytes.len();
                problems.push(Problem {
                    start,
                    end: (start + 2).min(i),
                    message: "Unterminated block comment".into(),
                });
            }
            if source[start..i].contains('\n') {
                line_start = true;
            }
            kind = TokenKind::Comment;
        } else if bytes[i] == b'#' && line_start {
            loop {
                let end = source[i..].find('\n').map_or(bytes.len(), |n| i + n);
                let continued = source[i..end].trim_end_matches('\r').ends_with('\\');
                i = end;
                if !continued || i == bytes.len() {
                    break;
                }
                i += 1;
            }
            kind = TokenKind::Directive;
        } else if ident_start(bytes[i]) {
            i += 1;
            while i < bytes.len() && ident_continue(bytes[i]) {
                i += 1;
            }
            kind = TokenKind::Ident;
            line_start = false;
        } else if bytes[i].is_ascii_digit()
            || (bytes[i] == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            i += 1;
            while i < bytes.len() {
                let b = bytes[i];
                if b.is_ascii_alphanumeric()
                    || b == b'.'
                    || ((b == b'+' || b == b'-')
                        && matches!(bytes[i - 1], b'e' | b'E' | b'p' | b'P'))
                {
                    i += 1;
                } else {
                    break;
                }
            }
            kind = TokenKind::Number;
            line_start = false;
        } else if bytes[i] == b'"' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' && bytes[i] != b'\n' {
                if bytes[i] == b'\\' {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
            }
            if bytes.get(i) == Some(&b'"') {
                i += 1;
            } else {
                problems.push(Problem {
                    start,
                    end: start + 1,
                    message: "Unterminated string".into(),
                });
            }
            while i < bytes.len() && !source.is_char_boundary(i) {
                i += 1;
            }
            kind = TokenKind::String;
            line_start = false;
        } else {
            let rest = &source[i..];
            let multi = [
                "<<=", ">>=", "++", "--", "==", "!=", "<=", ">=", "&&", "||", "^^", "+=", "-=",
                "*=", "/=", "%=", "<<", ">>", "&=", "|=", "^=",
            ];
            i += multi
                .iter()
                .find(|op| rest.starts_with(**op))
                .map_or_else(|| rest.chars().next().unwrap().len_utf8(), |op| op.len());
            kind = TokenKind::Punct;
            line_start = false;
        }
        tokens.push(Token {
            kind,
            start,
            end: i,
        });
    }
    (tokens, problems)
}

pub fn ident_start(b: u8) -> bool {
    b == b'_' || b.is_ascii_alphabetic()
}
pub fn ident_continue(b: u8) -> bool {
    ident_start(b) || b.is_ascii_digit()
}
