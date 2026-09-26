//! Static dequoting for shell words.
//!
//! Reduces a raw word's source text to the literal string bash would use
//! after quote removal, when that value is statically knowable. Returns
//! `None` as soon as an expansion (`$var`, `${…}`, `$(…)`, backticks) is
//! encountered, since its runtime value cannot be known statically.

/// Dequotes a raw word value, returning the literal text after removing
/// quoting and backslash escapes. Returns `None` when the word contains
/// an expansion and therefore has no static value.
///
/// This deliberately models only quoting/escaping: `~`, globs, and brace
/// expansions are left verbatim, matching how they appear after quote
/// removal.
pub fn dequote(raw: &str) -> Option<String> {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\'' => i = copy_single_quoted(&chars, i + 1, &mut out),
            '"' => i = copy_double_quoted(&chars, i + 1, &mut out)?,
            '$' => {
                if chars.get(i + 1) == Some(&'\'') {
                    i = decode_ansi_c(&chars, i + 2, &mut out);
                } else {
                    return None; // $var, ${…}, $(…), $?, …
                }
            }
            '`' => return None,
            '\\' => i = copy_escape(&chars, i, &mut out),
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    Some(out)
}

/// Copies a single-quoted section (text through the closing quote) and
/// returns the index just past it.
fn copy_single_quoted(chars: &[char], mut i: usize, out: &mut String) -> usize {
    while i < chars.len() && chars[i] != '\'' {
        out.push(chars[i]);
        i += 1;
    }
    i + 1
}

/// Copies a double-quoted section, returning `None` if it contains an
/// expansion (`$`/backtick), which has no static value.
fn copy_double_quoted(chars: &[char], mut i: usize, out: &mut String) -> Option<usize> {
    while i < chars.len() && chars[i] != '"' {
        match chars[i] {
            '\\' if i + 1 < chars.len() => {
                let next = chars[i + 1];
                if matches!(next, '$' | '`' | '"' | '\\') {
                    out.push(next);
                    i += 2;
                } else {
                    out.push('\\');
                    i += 1;
                }
            }
            '$' | '`' => return None,
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    Some(i + 1)
}

/// Copies a backslash escape (or line continuation), returning the next index.
fn copy_escape(chars: &[char], i: usize, out: &mut String) -> usize {
    if i + 1 >= chars.len() {
        out.push('\\');
        return i + 1;
    }
    let next = chars[i + 1];
    if next != '\n' {
        out.push(next);
    }
    i + 2
}

/// Decodes a `$'…'` ANSI-C quoted section starting just after `$'`,
/// appending decoded text to `out`. Returns the index just past the
/// closing quote.
fn decode_ansi_c(chars: &[char], mut i: usize, out: &mut String) -> usize {
    while i < chars.len() && chars[i] != '\'' {
        if chars[i] == '\\' && i + 1 < chars.len() {
            i += 1;
            match chars[i] {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                'a' => out.push('\u{7}'),
                'b' => out.push('\u{8}'),
                'e' | 'E' => out.push('\u{1b}'),
                'f' => out.push('\u{c}'),
                'v' => out.push('\u{b}'),
                '\\' => out.push('\\'),
                '\'' => out.push('\''),
                '"' => out.push('"'),
                'x' => i = push_hex_escape(chars, i, out),
                '0' => i = push_octal_escape(chars, i, out),
                other => out.push(other),
            }
        } else {
            out.push(chars[i]);
        }
        i += 1;
    }
    i + 1 // past closing quote (or end of input)
}

/// Decodes `\xHH` (up to two hex digits) starting at the `x`. Returns the
/// index of the last consumed digit.
fn push_hex_escape(chars: &[char], mut i: usize, out: &mut String) -> usize {
    let mut value = 0u32;
    let mut digits = 0;
    while digits < 2 {
        let Some(d) = chars.get(i + 1).and_then(|c| c.to_digit(16)) else {
            break;
        };
        value = value * 16 + d;
        i += 1;
        digits += 1;
    }
    out.push(char::from_u32(value).unwrap_or('\u{fffd}'));
    i
}

/// Decodes `\0NNN` (up to three octal digits) starting at the `0`. Returns
/// the index of the last consumed digit.
fn push_octal_escape(chars: &[char], mut i: usize, out: &mut String) -> usize {
    let mut value = 0u32;
    let mut digits = 0;
    while digits < 3 {
        let Some(d) = chars.get(i + 1).and_then(|c| c.to_digit(8)) else {
            break;
        };
        value = value * 8 + d;
        i += 1;
        digits += 1;
    }
    out.push(char::from_u32(value).unwrap_or('\u{fffd}'));
    i
}
