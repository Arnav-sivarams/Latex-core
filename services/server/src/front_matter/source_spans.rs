//! Locate literal metadata values without reserializing TeX source.

use super::{FrontMatterError, tex_code_line};
use std::ops::Range;

pub(super) fn mask_comments(source: &str) -> String {
    source
        .split_inclusive('\n')
        .fold(String::new(), |mut code, line| {
            let active = tex_code_line(line);
            code.push_str(active);
            code.extend(std::iter::repeat_n(' ', line.len() - active.len()));
            code
        })
}

fn whitespace(code: &[u8], mut offset: usize) -> usize {
    while code.get(offset).is_some_and(u8::is_ascii_whitespace) {
        offset += 1;
    }
    offset
}

fn control_word(code: &[u8], offset: usize) -> Option<Range<usize>> {
    if code.get(offset) != Some(&b'\\') {
        return None;
    }
    let start = offset + 1;
    let mut end = start;
    while code.get(end).is_some_and(u8::is_ascii_alphabetic) {
        end += 1;
    }
    (start != end).then_some(start..end)
}

fn group(code: &[u8], start: usize) -> Option<Range<usize>> {
    if code.get(start) != Some(&b'{') {
        return None;
    }
    let mut depth = 1;
    let mut offset = start + 1;
    while let Some(byte) = code.get(offset) {
        match byte {
            b'\\' => offset += 1, // A control symbol consumes escaped braces.
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + 1..offset);
                }
            }
            _ => {}
        }
        offset += 1;
    }
    None
}

pub(super) fn top_level(code: &str) -> bool {
    let bytes = code.as_bytes();
    let mut depth = 0_usize;
    let mut offset = 0;
    while let Some(byte) = bytes.get(offset) {
        match byte {
            b'\\' => offset += 1,
            b'{' => depth += 1,
            b'}' => {
                let Some(remaining) = depth.checked_sub(1) else {
                    return false;
                };
                depth = remaining;
            }
            _ => {}
        }
        offset += 1;
    }
    depth == 0
}

/// Select the effective top-level zero-argument definition. A later renewal
/// wins; a providecommand only takes effect if no earlier definition exists.
/// Unsupported definitions are errors, never a reason to add a duplicate.
pub(super) fn value_span(
    code: &str,
    command: &str,
) -> Result<Option<Range<usize>>, FrontMatterError> {
    let unsupported = || {
        FrontMatterError::InvalidValue(format!(
            "Cannot bind \\{command}: use a top-level literal zero-argument \\newcommand, \\renewcommand, \\providecommand, \\DeclareRobustCommand or \\def definition; edit unsupported source manually."
        ))
    };
    let bytes = code.as_bytes();
    let mut offset = 0;
    let mut depth = 0_usize;
    let mut selected = None;
    while offset < bytes.len() {
        if bytes[offset] != b'\\' {
            match bytes[offset] {
                b'{' => depth += 1,
                b'}' => depth = depth.saturating_sub(1),
                _ => {}
            }
            offset += 1;
            continue;
        }
        let Some(word) = control_word(bytes, offset) else {
            offset += 2;
            continue;
        };
        let kind = &code[word.clone()];
        offset = word.end;
        if !matches!(
            kind,
            "newcommand"
                | "renewcommand"
                | "providecommand"
                | "DeclareRobustCommand"
                | "def"
                | "gdef"
                | "edef"
                | "xdef"
                | "let"
        ) {
            continue;
        }
        if bytes.get(offset) == Some(&b'*') {
            offset += 1;
        }
        let name_start = whitespace(bytes, offset);
        let (name, after_name) = if let Some(name) = group(bytes, name_start) {
            (code[name.clone()].trim(), name.end + 1)
        } else if let Some(name) = control_word(bytes, name_start) {
            (&code[name_start..name.end], name.end)
        } else {
            continue;
        };
        if name != format!("\\{command}") {
            // Scan the body too, to reject nested definitions of this target.
            continue;
        }
        if depth != 0 || matches!(kind, "gdef" | "edef" | "xdef" | "let") {
            return Err(unsupported());
        }
        let mut body_start = whitespace(bytes, after_name);
        if code[body_start..].starts_with("[0]") && kind != "def" {
            body_start = whitespace(bytes, body_start + 3);
        }
        let body = group(bytes, body_start).ok_or_else(unsupported)?;
        if kind != "providecommand" || selected.is_none() {
            selected = Some(body.clone());
        }
        offset = body.end + 1;
    }
    Ok(selected)
}

/// Copy every byte between authorized replacements directly from the source.
pub(super) fn apply(
    source: &str,
    mut edits: Vec<(Range<usize>, String)>,
) -> Result<String, FrontMatterError> {
    edits.sort_by_key(|(range, _)| range.start);
    let mut output = String::new();
    let mut cursor = 0;
    for (range, value) in edits {
        if range.start < cursor || range.end < range.start || range.end > source.len() {
            return Err(FrontMatterError::InvalidValue(
                "Overlapping metadata replacement spans; source was not changed.".into(),
            ));
        }
        output.push_str(&source[cursor..range.start]);
        output.push_str(&value);
        cursor = range.end;
    }
    output.push_str(&source[cursor..]);
    Ok(output)
}
