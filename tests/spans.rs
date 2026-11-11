#![allow(clippy::expect_used)]
//! Tests for node spans and the word spans exposed on `Word` nodes.

use rable::{Node, NodeKind, QuotingContext, Span, WordSpanKind};

/// The word nodes of the first command in `source`.
fn words(source: &str) -> Vec<Node> {
    let nodes = rable::parse(source, false).expect("parse");
    let Some(NodeKind::Command { words, .. }) = nodes.into_iter().next().map(|node| node.kind)
    else {
        return Vec::new();
    };
    words
}

#[test]
fn spans_are_byte_offsets() {
    let words = words("echo café bar");
    let spans: Vec<&Span> = words.iter().map(|word| &word.span).collect();
    assert_eq!(
        spans,
        [&Span::new(0, 4), &Span::new(5, 10), &Span::new(11, 14)]
    );
}

#[test]
fn word_spans_cover_line_continuations_inside_the_word() {
    let source = "su\\\ndo true";
    let texts: Vec<&str> = words(source)
        .iter()
        .map(|word| word.source_text(source))
        .collect();
    assert_eq!(texts, ["su\\\ndo", "true"]);
}

#[test]
fn word_spans_exclude_trailing_line_continuations() {
    let source = "echo a\\\n b";
    let texts: Vec<&str> = words(source)
        .iter()
        .map(|word| word.source_text(source))
        .collect();
    assert_eq!(texts, ["echo", "a", "b"]);
}

#[test]
fn word_spans_cover_whole_arrays() {
    let source = "arr=(\n  one\n  two\n)";
    let nodes = rable::parse(source, false).expect("parse");
    assert_eq!(nodes[0].source_text(source), source);
}

#[test]
fn word_spans_expose_construct_kind_and_quoting() {
    let words = words("echo \"$PWD\" $HOME");
    let NodeKind::Word { spans, .. } = &words[1].kind else {
        unreachable!("expected Word");
    };
    let quoted = spans
        .iter()
        .find(|span| span.kind() == &WordSpanKind::DoubleQuoted)
        .expect("double-quoted span");
    let var = spans
        .iter()
        .find(|span| span.kind() == &WordSpanKind::SimpleVar)
        .expect("variable span");
    assert_eq!(quoted.range(), 0..6);
    assert_eq!(quoted.context(), QuotingContext::None);
    assert_eq!(var.range(), 1..5);
    assert_eq!(var.context(), QuotingContext::DoubleQuote);

    let NodeKind::Word { spans, .. } = &words[2].kind else {
        unreachable!("expected Word");
    };
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].kind(), &WordSpanKind::SimpleVar);
    assert_eq!(spans[0].context(), QuotingContext::None);
}
