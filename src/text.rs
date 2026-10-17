use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

pub fn offset(text: &str, position: Position) -> Option<usize> {
    let mut start = 0;
    for _ in 0..position.line {
        start += text.get(start..)?.find('\n')? + 1;
    }
    let line = text
        .get(start..)?
        .split('\n')
        .next()?
        .trim_end_matches('\r');
    let mut units = 0;
    for (byte, ch) in line.char_indices() {
        if units == position.character {
            return Some(start + byte);
        }
        units += ch.len_utf16() as u32;
        if units > position.character {
            return None;
        }
    }
    Some(start + line.len())
}

pub fn position(text: &str, byte: usize) -> Position {
    let mut byte = byte.min(text.len());
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    let prefix = &text[..byte];
    let start = prefix.rfind('\n').map_or(0, |n| n + 1);
    Position {
        line: prefix.bytes().filter(|b| *b == b'\n').count() as u32,
        character: prefix[start..]
            .trim_end_matches('\r')
            .encode_utf16()
            .count() as u32,
    }
}

pub fn range(text: &str, start: usize, end: usize) -> Range {
    Range {
        start: position(text, start),
        end: position(text, end),
    }
}

pub fn replace(text: &mut String, change: Range, replacement: &str) -> Result<(), String> {
    let start = offset(text, change.start).ok_or("Invalid edit start")?;
    let end = offset(text, change.end).ok_or("Invalid edit end")?;
    if start > end {
        return Err("Reversed edit range".into());
    }
    text.replace_range(start..end, replacement);
    Ok(())
}

pub fn normalize(path: &Path) -> PathBuf {
    if let Ok(path) = path.canonicalize() {
        return path;
    }
    let mut result = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            part => result.push(part.as_os_str()),
        }
    }
    result
}

pub fn path_to_uri(path: &Path) -> String {
    let raw = path.to_string_lossy();
    #[cfg(windows)]
    let raw = windows_uri_path(&raw);
    let mut uri = String::from("file://");
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~:".contains(&b) {
            uri.push(b as char);
        } else {
            use std::fmt::Write;
            let _ = write!(uri, "%{b:02X}");
        }
    }
    uri
}

pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let tail = uri.strip_prefix("file://")?;
    let tail = if tail.starts_with("localhost/") {
        &tail["localhost".len()..]
    } else {
        tail
    };
    if !tail.starts_with('/') || tail.contains(['?', '#']) {
        return None;
    }
    let mut bytes = Vec::new();
    let mut i = 0;
    while i < tail.len() {
        if tail.as_bytes()[i] == b'%' {
            bytes.push(u8::from_str_radix(tail.get(i + 1..i + 3)?, 16).ok()?);
            i += 3;
        } else {
            bytes.push(tail.as_bytes()[i]);
            i += 1;
        }
    }
    if bytes.contains(&0) {
        return None;
    }
    let decoded = String::from_utf8(bytes).ok()?;
    #[cfg(windows)]
    let decoded = if decoded.as_bytes().get(2) == Some(&b':') {
        &decoded[1..]
    } else {
        &decoded
    };
    Some(normalize(Path::new(&decoded)))
}

#[cfg(windows)]
fn windows_uri_path(path: &str) -> String {
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    let path = path.replace('\\', "/");
    if path.starts_with('/') {
        path
    } else {
        format!("/{path}")
    }
}
