//! Expansion and quoting constructs recorded inside word tokens.

use std::ops::Range;

use crate::ast::Node;

/// An expansion or quoting construct inside a `Word` or `CondTerm` node.
///
/// Each span records where the construct sits in the word's `value`, what
/// kind of construct it is, and the quoting context it appears in. Spans
/// nest: a `$VAR` inside `"..."` yields a [`WordSpanKind::DoubleQuoted`] span
/// containing a [`WordSpanKind::SimpleVar`] span whose context is
/// [`QuotingContext::DoubleQuote`]. Only the parser constructs spans.
///
/// `body` carries the already-parsed AST for command / process / backtick
/// substitution spans. The lexer produces it while locating the matching
/// delimiter, so word decomposition can reuse it instead of re-parsing the
/// same source. It is `None` for every other span kind.
#[derive(Debug, Clone, PartialEq)]
pub struct WordSpan {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) kind: WordSpanKind,
    pub(crate) context: QuotingContext,
    pub(crate) body: Option<Box<Node>>,
}

impl WordSpan {
    /// Byte range of the construct within the word's `value`.
    ///
    /// The value has line continuations removed, so the range indexes the
    /// value rather than the source.
    pub const fn range(&self) -> Range<usize> {
        self.start..self.end
    }

    /// The kind of construct.
    pub const fn kind(&self) -> &WordSpanKind {
        &self.kind
    }

    /// The quoting context the construct appears in.
    pub const fn context(&self) -> QuotingContext {
        self.context
    }
}

/// The quoting context at the point where a span was recorded.
///
/// Bash has context-sensitive expansion rules. For example, `$'...'`
/// is an ANSI-C quote at top level or inside `${...}`, but is NOT
/// special inside `"..."` (it's just literal `$` + `'...'`).
///
/// Each span records the innermost context it appears in, so downstream
/// consumers can make correct decisions without re-deriving context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum QuotingContext {
    /// Top-level word context (no quoting).
    None,
    /// Inside double quotes `"..."`.
    DoubleQuote,
    /// Inside parameter expansion `${...}` (resets quoting context).
    ParamExpansion,
    /// Inside command substitution `$(...)` (resets quoting context).
    CommandSub,
    /// Inside backtick command substitution `` `...` ``.
    Backtick,
}

/// The kind of expansion or quoting construct a `WordSpan` represents.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WordSpanKind {
    // -- Sexp-relevant (formatter extracts content from these) --
    /// Command substitution: `$(...)`.
    CommandSub,
    /// ANSI-C quoting: `$'...'`.
    AnsiCQuote,
    /// Locale translation string: `$"..."`.
    LocaleString,
    /// Process substitution: `<(...)` or `>(...)`.
    ProcessSub(char),

    // -- Structural (formatter treats as literal text) --
    /// Single-quoted string: `'...'`.
    SingleQuoted,
    /// Double-quoted string: `"..."`.
    DoubleQuoted,
    /// Parameter expansion: `${...}`.
    ParamExpansion,
    /// Simple variable: `$VAR`.
    SimpleVar,
    /// Arithmetic substitution: `$((...))`.
    ArithmeticSub,
    /// Backtick command substitution: `` `...` ``.
    Backtick,
    /// Array subscript: `[...]`.
    BracketSubscript,
    /// Extended glob pattern: `@(...)`, `?(...)`, etc.
    Extglob(char),
    /// Deprecated arithmetic: `$[...]`.
    DeprecatedArith,
    /// Backslash escape: `\X` (not `\<newline>` line continuations).
    Escape,
    /// Brace expansion: `{a,b,c}` or `{1..10}`.
    BraceExpansion,
}
