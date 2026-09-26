//! Decomposes word values into structured AST parts.
//!
//! Reuses the segment parser from `sexp::word` to split a word's raw
//! value into typed segments, then maps each segment to an AST `Node`.
//!
//! # Layout
//!
//! | file                  | responsibility                                 |
//! |-----------------------|------------------------------------------------|
//! | `ansi_c.rs`           | `$'…'` escape decoding, locale-string trimming |
//! | `dequote.rs`          | static dequoting of a raw word value           |
//! | `param_expansion.rs`  | `$var` / `${…}` parsing including subscripts   |
//! | `substitution.rs`     | `$(…)`, `<(…)`, `$((…))` with depth guarding   |
//! | `tests.rs`            | integration tests for all segment kinds        |

use std::cell::Cell;

use crate::ast::{Node, NodeKind};
use crate::lexer::word_builder::{WordSpan, WordSpanKind};
use crate::sexp::word::WordSegment;

mod ansi_c;
mod dequote;
mod param_expansion;
mod substitution;

pub use dequote::dequote;

/// Returns true when a word contains any expansion whose runtime value is
/// not statically knowable (command/process/parameter/arithmetic/backtick
/// substitution, or brace expansion).
pub fn has_expansion_spans(spans: &[WordSpan]) -> bool {
    spans.iter().any(|s| {
        matches!(
            s.kind,
            WordSpanKind::CommandSub
                | WordSpanKind::ProcessSub(_)
                | WordSpanKind::ParamExpansion
                | WordSpanKind::SimpleVar
                | WordSpanKind::ArithmeticSub
                | WordSpanKind::Backtick
                | WordSpanKind::BraceExpansion
        )
    })
}

thread_local! {
    static DECOMPOSE_DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// RAII guard to prevent infinite recursion when decomposing nested
/// command/process substitutions.
///
/// Depth limit of 2 matches `format::DepthGuard` — allows `$(a $(b))`
/// but stops before unbounded nesting. The counter is separate from
/// the format module's counter since decomposition and formatting
/// are independent recursion paths.
pub(super) struct DepthGuard;

impl DepthGuard {
    pub(super) fn enter() -> Option<Self> {
        DECOMPOSE_DEPTH.with(|d| {
            let v = d.get();
            if v >= 2 {
                return None;
            }
            d.set(v + 1);
            Some(Self)
        })
    }
}

impl Drop for DepthGuard {
    fn drop(&mut self) {
        DECOMPOSE_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

/// Shared `WordLiteral` constructor used for synthetic values and error
/// recovery across submodules.
pub(super) fn literal_fallback(value: &str) -> Node {
    Node::empty(NodeKind::WordLiteral {
        value: value.to_string(),
    })
}

/// Decomposes a word using lexer spans — the primary path for all
/// token-derived words.
///
/// Spans are taken by mutable reference because the parsed substitution
/// bodies they carry are *moved* into the resulting `parts` (and out of
/// the operand spans). The word keeps its spans for formatting, but they
/// no longer own a second copy of each nested AST.
pub fn decompose_word_with_spans(value: &str, spans: &mut [WordSpan]) -> Vec<Node> {
    let top_level = crate::sexp::word::top_level_decomposable_indices(spans);
    let mut parts = Vec::new();
    let mut pos = 0usize;
    for index in top_level {
        let (start, end) = (spans[index].start, spans[index].end);
        if start > pos
            && let Some(text) = value.get(pos..start)
        {
            parts.push(literal_fallback(text));
        }
        for (seg_index, seg) in crate::sexp::word::span_segments(value, &spans[index])
            .into_iter()
            .enumerate()
        {
            parts.push(segment_to_node(seg, seg_index == 0, value, spans, index));
        }
        pos = end;
    }
    if pos < value.len()
        && let Some(text) = value.get(pos..)
    {
        parts.push(literal_fallback(text));
    }
    parts
}

/// Decomposes an arbitrary region `[start, end)` of `value` as a word,
/// selecting the spans that fall inside it and rebasing their offsets.
/// Used to recover the nested structure that the lexer recorded inside
/// opaque expansion operands (parameter arguments, arithmetic bodies,
/// conditional terms).
///
/// Bodies are moved out of the source spans so no nested AST is copied.
pub fn decompose_region(
    value: &str,
    spans: &mut [WordSpan],
    start: usize,
    end: usize,
) -> Vec<Node> {
    let Some(sub) = value.get(start..end) else {
        return Vec::new();
    };
    let mut rebased: Vec<WordSpan> = Vec::new();
    for span in spans.iter_mut() {
        if span.start >= start && span.end <= end {
            rebased.push(WordSpan {
                start: span.start - start,
                end: span.end - start,
                kind: span.kind.clone(),
                context: span.context,
                body: span.body.take(),
            });
        }
    }
    if sub.is_empty() {
        return Vec::new();
    }
    decompose_word_with_spans(sub, &mut rebased)
}

/// Creates a single `WordLiteral` for synthetic values (no lexer token).
/// Used for fd numbers (`"0"`, `"1"`), synthetic `"$@"`, etc.
pub(super) fn decompose_word_literal(value: &str) -> Vec<Node> {
    vec![literal_fallback(value)]
}

fn segment_to_node(
    seg: WordSegment,
    first: bool,
    value: &str,
    spans: &mut [WordSpan],
    index: usize,
) -> Node {
    match seg {
        WordSegment::Literal(text) => Node::empty(NodeKind::WordLiteral { value: text }),
        WordSegment::AnsiCQuote(content) => {
            let decoded = ansi_c::ansi_c_decode(&content);
            Node::empty(NodeKind::AnsiCQuote { content, decoded })
        }
        WordSegment::LocaleString(content) => {
            let inner = ansi_c::strip_locale_quotes(&content);
            Node::empty(NodeKind::LocaleString { content, inner })
        }
        // The lexer parsed the substitution body while locating its
        // delimiter; move it instead of re-parsing the same source.
        WordSegment::ArithmeticSub(inner) => {
            substitution::arithmetic_sub_to_node(&inner, value, spans, index)
        }
        WordSegment::CommandSubstitution(content) => {
            spans[index].body.take().filter(|_| first).map_or_else(
                || substitution::cmdsub_to_node(&content),
                |body| {
                    Node::empty(NodeKind::CommandSubstitution {
                        command: body,
                        brace: false,
                    })
                },
            )
        }
        WordSegment::ProcessSubstitution(direction, content) => {
            spans[index].body.take().filter(|_| first).map_or_else(
                || substitution::procsub_to_node(direction, &content),
                |body| {
                    Node::empty(NodeKind::ProcessSubstitution {
                        direction: direction.to_string(),
                        command: body,
                    })
                },
            )
        }
        WordSegment::SimpleVar(text) => param_expansion::parse_simple_var(&text),
        WordSegment::ParamExpansion(text) => {
            param_expansion::parse_braced_param(&text, value, spans, index)
        }
        WordSegment::BraceExpansion(text) => {
            Node::empty(NodeKind::BraceExpansion { content: text })
        }
    }
}

#[cfg(test)]
mod tests;
