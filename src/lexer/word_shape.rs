//! Shape checks on assembled word values.

/// Returns true if `value` is a non-empty plain bash identifier
/// (`[a-zA-Z_][a-zA-Z_0-9]*`) with no other characters.
///
/// Used to gate bracket-subscript absorption: `arr[i]=x` and `arr[i]`
/// only behave specially when the word so far is a bare identifier.
/// Words like `][[`, `[c[`, or `foo^` must fall through to ordinary
/// tokenization so `[...]` contents do not absorb whitespace.
pub(super) fn is_identifier_prefix(value: &str) -> bool {
    let bytes = value.as_bytes();
    let Some(first) = bytes.first() else {
        return false;
    };
    if !matches!(first, b'a'..=b'z' | b'A'..=b'Z' | b'_') {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|b| matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_'))
}

/// Byte offset just past the `=` of an assignment-shaped word value, or
/// `None` when the value does not match `NAME=`, `NAME+=`, `NAME[...]=`, or
/// `NAME[...]+=`.
pub(crate) const fn assignment_value_start(value: &str) -> Option<usize> {
    let bytes = value.as_bytes();
    // Must start with [a-zA-Z_]
    if !matches!(bytes.first(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_')) {
        return None;
    }
    let mut i = 1;
    // Continue with [a-zA-Z_0-9]
    while i < bytes.len() {
        match bytes[i] {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' => i += 1,
            b'[' => {
                // Skip subscript [...] — reject if it contains whitespace
                // (bash doesn't allow spaces in assignment subscripts)
                i += 1;
                let mut depth = 1;
                while i < bytes.len() && depth > 0 {
                    match bytes[i] {
                        b'[' => depth += 1,
                        b']' => depth -= 1,
                        b' ' | b'\t' | b'\n' => return None,
                        _ => {}
                    }
                    i += 1;
                }
            }
            b'+' if i + 1 < bytes.len() && bytes[i + 1] == b'=' => return Some(i + 2),
            b'=' => return Some(i + 1),
            _ => return None,
        }
    }
    None
}
