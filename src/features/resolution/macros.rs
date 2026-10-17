use crate::{
    syntax::{self, TokenKind},
    workspace::Graph,
};
use std::collections::{BTreeMap, HashSet};

const MAX_DEPTH: usize = 64;
const MAX_TOKENS: usize = 4096;
const MAX_BYTES: usize = 64 * 1024;
const MAX_DIRECTIVES: usize = 65536;
const MAX_INCLUDES: usize = 4096;
const MAX_VISIT_TOKENS: usize = 1_000_000;

#[derive(Clone, Debug)]
pub(in crate::features) struct Definition {
    pub(in crate::features) name: String,
    pub(in crate::features) replacement: Option<String>,
    pub(in crate::features) origin: Option<(usize, usize)>,
    pub(in crate::features) name_end: Option<usize>,
    pub(in crate::features) signature: String,
    pub(in crate::features) uncertain: bool,
}

#[derive(Default)]
pub(in crate::features) struct Macros {
    definitions: BTreeMap<String, Definition>,
    undefined: HashSet<String>,
    valid: bool,
}

impl Macros {
    pub(in crate::features) fn at(graph: &Graph, source_id: usize, at: usize) -> Self {
        let mut macros = Self {
            valid: true,
            ..Self::default()
        };
        for definition in &graph.config.config.macros {
            let (name, replacement) = definition.split_once('=').unwrap_or((definition, "1"));
            macros.definitions.insert(
                name.into(),
                Definition {
                    name: name.into(),
                    replacement: Some(replacement.into()),
                    origin: None,
                    name_end: None,
                    signature: format!("#define {name} {replacement}"),
                    uncertain: false,
                },
            );
        }
        let mut replay = Replay {
            graph,
            macros,
            target: (source_id, at),
            stack: Vec::new(),
            directives: 0,
            includes: 0,
            tokens: 0,
        };
        if graph.sources.is_empty() || !replay.visit(0, Activity::Active) {
            replay.macros.valid = false;
        }
        replay.macros
    }

    pub(in crate::features) fn get(&self, name: &str) -> Option<&Definition> {
        self.definitions.get(name)
    }

    pub(in crate::features) fn is_valid(&self) -> bool {
        self.valid
    }

    pub(in crate::features) fn iter(&self) -> impl Iterator<Item = &Definition> {
        self.definitions
            .values()
            .filter(|definition| self.valid && !definition.uncertain)
    }

    pub(in crate::features) fn expand(&self, text: &str) -> Option<String> {
        if !self.valid {
            return None;
        }
        let mut output = Vec::new();
        let mut budget = MAX_TOKENS;
        let mut bytes = MAX_BYTES;
        self.expand_into(text, &mut Vec::new(), &mut output, &mut budget, &mut bytes)?;
        Some(output.join(" "))
    }

    fn expand_into(
        &self,
        text: &str,
        stack: &mut Vec<String>,
        output: &mut Vec<String>,
        budget: &mut usize,
        bytes: &mut usize,
    ) -> Option<()> {
        if text.len() > MAX_BYTES || stack.len() >= MAX_DEPTH {
            return None;
        }
        let (tokens, problems) = syntax::lex(text);
        if !problems.is_empty() {
            return None;
        }
        for token in tokens {
            if token.kind == TokenKind::Comment {
                continue;
            }
            *budget = budget.checked_sub(1)?;
            let word = token.text(text);
            if token.kind == TokenKind::Directive || word == "#" {
                return None;
            }
            if token.kind == TokenKind::Ident {
                if let Some(definition) = self.get(word) {
                    if definition.uncertain || stack.iter().any(|name| name == word) {
                        return None;
                    }
                    let replacement = definition.replacement.as_deref()?;
                    stack.push(word.into());
                    self.expand_into(replacement, stack, output, budget, bytes)?;
                    stack.pop();
                    continue;
                }
            }
            *bytes = bytes.checked_sub(word.len() + usize::from(!output.is_empty()))?;
            output.push(word.into());
        }
        Some(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Activity {
    Active,
    Inactive,
    Unknown,
}

impl Activity {
    fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::Inactive, _) | (_, Self::Inactive) => Self::Inactive,
            (Self::Active, Self::Active) => Self::Active,
            _ => Self::Unknown,
        }
    }

    fn not(self) -> Self {
        match self {
            Self::Active => Self::Inactive,
            Self::Inactive => Self::Active,
            Self::Unknown => Self::Unknown,
        }
    }
}

struct Conditional {
    parent: Activity,
    remaining: Activity,
}

struct Replay<'a> {
    graph: &'a Graph,
    macros: Macros,
    target: (usize, usize),
    stack: Vec<usize>,
    directives: usize,
    includes: usize,
    tokens: usize,
}

impl Replay<'_> {
    fn visit(&mut self, source_id: usize, mut activity: Activity) -> bool {
        if self.stack.contains(&source_id) {
            return false;
        }
        if self.stack.len() >= MAX_DEPTH || self.includes >= MAX_INCLUDES {
            self.macros.valid = false;
            return false;
        }
        self.includes += 1;
        self.stack.push(source_id);
        let source = &self.graph.sources[source_id];
        let mut conditionals = Vec::<Conditional>::new();
        for token in &source.index.tokens {
            if source_id == self.target.0
                && (token.start >= self.target.1 || token.end > self.target.1)
            {
                break;
            }
            self.tokens += 1;
            if self.tokens > MAX_VISIT_TOKENS {
                self.macros.valid = false;
                break;
            }
            if token.kind != TokenKind::Directive {
                continue;
            }
            self.directives += 1;
            if self.directives > MAX_DIRECTIVES || token.end - token.start > MAX_BYTES {
                self.macros.valid = false;
                break;
            }
            let (directive, offsets) = splice(token.text(&source.text));
            let body = &directive[1..];
            let (parts, problems) = syntax::lex(body);
            if !problems.is_empty() {
                self.macros.valid = false;
                break;
            }
            let parts: Vec<_> = parts
                .iter()
                .filter(|part| part.kind != TokenKind::Comment)
                .collect();
            let Some(command) = parts.first().map(|part| part.text(body)) else {
                continue;
            };
            let argument = parts.get(1).map(|part| part.text(body));
            match command {
                "if" | "ifdef" | "ifndef" => {
                    let condition = if command == "if" {
                        Activity::Unknown
                    } else {
                        let defined = argument.map_or(Activity::Unknown, |name| self.defined(name));
                        if command == "ifndef" {
                            defined.not()
                        } else {
                            defined
                        }
                    };
                    conditionals.push(Conditional {
                        parent: activity,
                        remaining: condition.not(),
                    });
                    activity = activity.and(condition);
                }
                "elif" => {
                    if let Some(frame) = conditionals.last_mut() {
                        activity = frame.parent.and(frame.remaining).and(Activity::Unknown);
                        frame.remaining = frame.remaining.and(Activity::Unknown);
                    }
                }
                "else" => {
                    if let Some(frame) = conditionals.last_mut() {
                        activity = frame.parent.and(frame.remaining);
                        frame.remaining = Activity::Inactive;
                    }
                }
                "endif" => {
                    if let Some(frame) = conditionals.pop() {
                        activity = frame.parent;
                    }
                }
                "define" | "undef" if activity != Activity::Inactive => {
                    let Some(name_token) =
                        parts.get(1).filter(|part| part.kind == TokenKind::Ident)
                    else {
                        continue;
                    };
                    let name = name_token.text(body);
                    if activity == Activity::Unknown {
                        self.macros.undefined.remove(name);
                        self.macros.definitions.insert(
                            name.into(),
                            Definition {
                                name: name.into(),
                                replacement: None,
                                origin: None,
                                name_end: None,
                                signature: format!("#define {name} (conditional)"),
                                uncertain: true,
                            },
                        );
                    } else if command == "undef" {
                        self.macros.definitions.remove(name);
                        self.macros.undefined.insert(name.into());
                    } else {
                        self.macros.undefined.remove(name);
                        let function_like = body.as_bytes().get(name_token.end) == Some(&b'(');
                        let origin = token.start + offsets[name_token.start + 1];
                        self.macros.definitions.insert(
                            name.into(),
                            Definition {
                                name: name.into(),
                                replacement: (!function_like)
                                    .then(|| body[name_token.end..].trim().into()),
                                origin: Some((source_id, origin)),
                                name_end: Some(token.start + offsets[name_token.end] + 1),
                                signature: format!("#define {}", body[name_token.start..].trim()),
                                uncertain: false,
                            },
                        );
                    }
                }
                "include" if activity != Activity::Inactive => {
                    let path = source
                        .index
                        .includes
                        .iter()
                        .find(|include| token.start <= include.start && include.start < token.end)
                        .and_then(|include| {
                            self.graph
                                .includes
                                .get(&(source.path.clone(), include.start))
                        });
                    let child = path.and_then(|path| {
                        self.graph
                            .sources
                            .iter()
                            .position(|source| &source.path == path)
                    });
                    if let Some(child) = child {
                        if self.visit(child, activity) {
                            self.stack.pop();
                            return true;
                        }
                    }
                }
                _ => {}
            }
        }
        self.stack.pop();
        source_id == self.target.0
    }

    fn defined(&self, name: &str) -> Activity {
        match self.macros.get(name) {
            Some(definition) if definition.uncertain => Activity::Unknown,
            Some(_) => Activity::Active,
            None if self.macros.undefined.contains(name) => Activity::Inactive,
            None if name.starts_with("GL_") || name.starts_with("__") => Activity::Unknown,
            None => Activity::Inactive,
        }
    }
}

fn splice(text: &str) -> (String, Vec<usize>) {
    let bytes = text.as_bytes();
    let mut result = Vec::with_capacity(bytes.len());
    let mut offsets = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"\\\n") {
            i += 2;
        } else if bytes[i..].starts_with(b"\\\r\n") {
            i += 3;
        } else {
            result.push(bytes[i]);
            offsets.push(i);
            i += 1;
        }
    }
    (String::from_utf8(result).unwrap(), offsets)
}
