mod indexer;
mod lexer;

pub use lexer::{ident_continue, ident_start, lex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Ident,
    Number,
    Punct,
    Comment,
    Directive,
    String,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub kind: TokenKind,
    pub start: usize,
    pub end: usize,
}

impl Token {
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        &source[self.start..self.end]
    }
}

#[derive(Clone, Debug)]
pub struct Problem {
    pub start: usize,
    pub end: usize,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct Include {
    pub name: String,
    pub start: usize,
    pub end: usize,
    pub angled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolKind {
    Variable,
    Constant,
    Function,
    Struct,
    Field,
    Macro,
    Parameter,
}

impl SymbolKind {
    pub fn completion(self) -> u32 {
        match self {
            Self::Function => 3,
            Self::Struct => 22,
            Self::Field => 5,
            Self::Constant => 21,
            Self::Macro => 14,
            _ => 6,
        }
    }
    pub fn document(self) -> u32 {
        match self {
            Self::Function => 12,
            Self::Struct => 23,
            Self::Field => 8,
            Self::Constant => 14,
            _ => 13,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Parameter {
    pub name: String,
    pub ty: String,
    pub label: String,
    pub type_start: usize,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug)]
pub struct Symbol {
    pub name: String,
    pub ty: String,
    pub kind: SymbolKind,
    pub start: usize,
    pub end: usize,
    pub extent_start: usize,
    pub extent_end: usize,
    pub scope_start: usize,
    pub scope_end: usize,
    pub owner: Option<String>,
    pub signature: String,
    pub parameters: Vec<Parameter>,
}

impl Symbol {
    pub fn visible(&self, at: usize) -> bool {
        self.owner.is_none()
            && self.scope_start <= at
            && at <= self.scope_end
            && (self.start <= at || self.kind == SymbolKind::Parameter)
    }
    pub fn global(&self) -> bool {
        self.scope_start == 0 && self.owner.is_none()
    }
}

#[derive(Clone, Debug, Default)]
pub struct Index {
    pub tokens: Vec<Token>,
    pub symbols: Vec<Symbol>,
    pub includes: Vec<Include>,
    pub problems: Vec<Problem>,
}

pub fn qualifier(word: &str) -> bool {
    matches!(
        word,
        "const"
            | "in"
            | "out"
            | "inout"
            | "uniform"
            | "buffer"
            | "shared"
            | "attribute"
            | "varying"
            | "centroid"
            | "sample"
            | "patch"
            | "flat"
            | "smooth"
            | "noperspective"
            | "invariant"
            | "precise"
            | "highp"
            | "mediump"
            | "lowp"
            | "coherent"
            | "volatile"
            | "restrict"
            | "readonly"
            | "writeonly"
            | "precision"
    )
}

pub fn index(source: &str) -> Index {
    let (tokens, problems) = lex(source);
    let mut result = Index {
        tokens,
        problems,
        ..Index::default()
    };
    indexer::populate(source, &mut result);
    result
}

impl Index {
    pub fn identifier_at(&self, at: usize) -> Option<&Token> {
        self.tokens
            .iter()
            .find(|t| t.kind == TokenKind::Ident && t.start <= at && at < t.end)
            .or_else(|| {
                self.tokens
                    .iter()
                    .find(|t| t.kind == TokenKind::Ident && t.end == at)
            })
    }
    pub fn code_tokens(&self) -> impl DoubleEndedIterator<Item = &Token> {
        self.tokens
            .iter()
            .filter(|t| !matches!(t.kind, TokenKind::Comment | TokenKind::Directive))
    }
}
