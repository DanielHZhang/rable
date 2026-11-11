#![allow(clippy::redundant_pub_crate)]

//! Word builder for accumulating word tokens with expansion span tracking.
//!
//! `WordBuilder` bundles the word value string with a list of `WordSpan`s
//! that record where each expansion starts and ends, along with the
//! quoting context at the point of recording. This eliminates the need
//! for downstream code to re-parse word values.
//!
//! The `WordSpan` type itself lives in `crate::word_span` — its fields are
//! `pub(crate)` and construction happens here via the `record()` method on
//! `WordBuilder`, so consumers can read spans but never forge them.

use crate::word_span::{QuotingContext, WordSpan, WordSpanKind};

/// Accumulates a word token's value string and expansion spans.
pub(crate) struct WordBuilder {
    /// The word value being built character by character.
    pub(crate) value: String,
    /// Expansion spans recorded during lexing, ordered by start offset.
    pub(crate) spans: Vec<WordSpan>,
    /// Stack of quoting contexts — the current context is the last entry.
    /// Empty means top-level (no quoting).
    context_stack: Vec<QuotingContext>,
    /// Parsed AST for the substitution currently being read. Set by the
    /// fork-and-merge readers and consumed by the next `record`, so the
    /// corresponding `WordSpan` can carry the body without re-parsing.
    pending_body: Option<Box<crate::ast::Node>>,
}

impl WordBuilder {
    pub(crate) const fn new() -> Self {
        Self {
            value: String::new(),
            spans: Vec::new(),
            context_stack: Vec::new(),
            pending_body: None,
        }
    }

    /// Stores the AST parsed for the substitution currently being read.
    /// The next [`Self::record`] attaches it to the resulting span.
    pub(crate) fn set_pending_body(&mut self, body: crate::ast::Node) {
        self.pending_body = Some(Box::new(body));
    }

    pub(crate) fn push(&mut self, c: char) {
        self.value.push(c);
    }

    pub(crate) fn ends_with(&self, c: char) -> bool {
        self.value.ends_with(c)
    }

    pub(crate) const fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    /// Returns the current byte offset — use before an expansion to
    /// capture its start position.
    pub(crate) const fn span_start(&self) -> usize {
        self.value.len()
    }

    /// Records a completed expansion span from `start` to the current
    /// end of the value string, capturing the current quoting context.
    pub(crate) fn record(&mut self, start: usize, kind: WordSpanKind) {
        self.spans.push(WordSpan {
            start,
            end: self.value.len(),
            kind,
            context: self.current_context(),
            body: self.pending_body.take(),
        });
    }

    /// Returns the current quoting context.
    pub(crate) fn current_context(&self) -> QuotingContext {
        self.context_stack
            .last()
            .copied()
            .unwrap_or(QuotingContext::None)
    }

    /// Pushes a quoting context onto the stack.
    pub(crate) fn enter_context(&mut self, ctx: QuotingContext) {
        self.context_stack.push(ctx);
    }

    /// Pops the current quoting context from the stack.
    pub(crate) fn leave_context(&mut self) {
        self.context_stack.pop();
    }
}
