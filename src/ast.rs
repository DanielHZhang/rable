use crate::lexer::word_builder::{QuotingContext, WordSpanKind};

/// Source span representing a **character** range in the original input.
///
/// Offsets are character indices (the lexer scans the source as a `Vec<char>`),
/// not byte offsets — they diverge for multibyte UTF-8. Use
/// [`Node::source_text`] to recover the source slice; it converts these
/// character indices to byte offsets before slicing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

/// Opaque span metadata attached to `Word` and `CondTerm` nodes.
///
/// Records where each expansion starts/ends in the raw token text.
/// External consumers see this type through the variant field but cannot
/// read or construct it — the fields are crate-private.
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

impl Span {
    /// Creates a new span with the given byte offsets.
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// Creates an empty span (used for synthetic nodes).
    pub const fn empty() -> Self {
        Self { start: 0, end: 0 }
    }

    /// Returns true if this span has no extent (synthetic or unset).
    pub const fn is_empty(&self) -> bool {
        self.start >= self.end
    }
}

/// A spanned AST node combining a [`NodeKind`] with its source [`Span`].
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub kind: NodeKind,
    pub span: Span,
}

impl Node {
    /// Creates a new node with the given kind and span.
    pub const fn new(kind: NodeKind, span: Span) -> Self {
        Self { kind, span }
    }

    /// Creates a node with an empty span (for synthetic or temporary nodes).
    pub const fn empty(kind: NodeKind) -> Self {
        Self {
            kind,
            span: Span::empty(),
        }
    }

    /// Extracts the source text for this node from the original source string.
    ///
    /// Spans use character indices (matching `Token.pos` semantics).
    /// Returns an empty string for synthetic nodes or invalid spans.
    pub fn source_text<'a>(&self, source: &'a str) -> &'a str {
        if self.span.is_empty() {
            return "";
        }
        // Convert char indices to byte offsets
        let byte_start = source.char_indices().nth(self.span.start).map(|(i, _)| i);
        let byte_end = source
            .char_indices()
            .nth(self.span.end)
            .map_or(source.len(), |(i, _)| i);
        match byte_start {
            Some(s) if byte_end <= source.len() => &source[s..byte_end],
            _ => "",
        }
    }
}

/// AST node representing all bash constructs.
///
/// This enum mirrors Parable's AST node classes exactly, ensuring
/// S-expression output compatibility.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::use_self)]
pub enum NodeKind {
    /// A word token, possibly containing expansion parts.
    ///
    /// `dequoted` is the statically-known literal value after quote and
    /// backslash removal, or `None` when the word contains an expansion
    /// whose runtime value cannot be known. This lets consumers match on
    /// the effective command/argument text (`'sudo'` → `Some("sudo")`)
    /// without implementing shell dequoting themselves.
    Word {
        value: String,
        dequoted: Option<String>,
        parts: Vec<Node>,
        spans: Vec<WordSpan>,
    },

    /// A literal text segment within a word's parts list.
    WordLiteral { value: String },

    /// A simple command: assignments, words, and redirects.
    Command {
        assignments: Vec<Node>,
        words: Vec<Node>,
        redirects: Vec<Node>,
    },

    /// A pipeline of commands separated by `|` or `|&`.
    Pipeline {
        commands: Vec<Node>,
        separators: Vec<PipeSep>,
    },

    /// A list of commands with operators (`;`, `&&`, `||`, `&`, `\n`).
    List { items: Vec<ListItem> },

    // -- Compound commands --
    /// `if condition; then body; [elif ...; then ...;] [else ...;] fi`
    If {
        condition: Box<Node>,
        then_body: Box<Node>,
        else_body: Option<Box<Node>>,
        redirects: Vec<Node>,
    },

    /// `while condition; do body; done`
    While {
        condition: Box<Node>,
        body: Box<Node>,
        redirects: Vec<Node>,
    },

    /// `until condition; do body; done`
    Until {
        condition: Box<Node>,
        body: Box<Node>,
        redirects: Vec<Node>,
    },

    /// `for var [in words]; do body; done`
    For {
        var: String,
        words: Option<Vec<Node>>,
        body: Box<Node>,
        redirects: Vec<Node>,
    },

    /// C-style for loop: `for (( init; cond; incr )); do body; done`
    ForArith {
        init: String,
        cond: String,
        incr: String,
        body: Box<Node>,
        redirects: Vec<Node>,
    },

    /// `select var [in words]; do body; done`
    Select {
        var: String,
        words: Option<Vec<Node>>,
        body: Box<Node>,
        redirects: Vec<Node>,
    },

    /// `case word in pattern) body;; ... esac`
    Case {
        word: Box<Node>,
        patterns: Vec<CasePattern>,
        redirects: Vec<Node>,
    },

    /// A function definition: `name() { body; }` or `function name { body; }`
    Function { name: String, body: Box<Node> },

    /// A subshell: `( commands )`
    Subshell {
        body: Box<Node>,
        redirects: Vec<Node>,
    },

    /// A brace group: `{ commands; }`
    BraceGroup {
        body: Box<Node>,
        redirects: Vec<Node>,
    },

    /// A coprocess: `coproc [name] command`
    Coproc {
        name: Option<String>,
        command: Box<Node>,
    },

    // -- Redirections --
    /// I/O redirection: `[fd]op target`
    ///
    /// `varfd` holds the variable name when the source used the
    /// `{name}op` form (e.g. `{fd}>file`). It is `None` for plain
    /// numeric or default-fd redirects.
    Redirect {
        op: String,
        target: Box<Node>,
        fd: i32,
        varfd: Option<String>,
    },

    /// Here-document: `<<[-]DELIM\ncontent\nDELIM`
    ///
    /// `parts` is the decomposed body for unquoted (expanding) heredocs,
    /// exposing nested command/parameter expansions. It is empty for
    /// quoted heredocs, whose body is literal.
    HereDoc {
        delimiter: String,
        content: String,
        strip_tabs: bool,
        quoted: bool,
        fd: i32,
        complete: bool,
        parts: Vec<Node>,
    },

    // -- Expansions --
    /// Parameter expansion: `$var` or `${var[op arg]}`
    ///
    /// `parts` decomposes the expansion body (array subscript and/or
    /// operator argument), exposing nested command substitutions such as
    /// `${x:-$(cmd)}` and `${arr[$(cmd)]}`.
    ParamExpansion {
        param: String,
        op: Option<String>,
        arg: Option<String>,
        parts: Vec<Node>,
    },

    /// Parameter length: `${#var}`
    ParamLength { param: String },

    /// Indirect expansion: `${!var[op arg]}`
    ///
    /// `parts` decomposes the expansion body as for [`Self::ParamExpansion`].
    ParamIndirect {
        param: String,
        op: Option<String>,
        arg: Option<String>,
        parts: Vec<Node>,
    },

    /// Command substitution: `$(cmd)` or `` `cmd` ``
    CommandSubstitution { command: Box<Node>, brace: bool },

    /// Process substitution: `<(cmd)` or `>(cmd)`
    ProcessSubstitution {
        direction: String,
        command: Box<Node>,
    },

    /// ANSI-C quoting: `$'...'`.
    ///
    /// `content` is the raw inner text including backslash escape sequences
    /// (e.g. `foo\nbar` is stored as the literal 9-character string).
    /// `decoded` is the same text with escapes processed per the bash manual
    /// (e.g. `\n` becomes an actual newline, `\x41` becomes `A`) so consumers
    /// can statically resolve the quoted value without re-parsing escapes.
    AnsiCQuote { content: String, decoded: String },

    /// Locale string: `$"..."`.
    ///
    /// `content` is the raw inner text **including** the surrounding double
    /// quotes (for backwards-compatible S-expression output). `inner` is the
    /// same text with the outer pair of double quotes stripped so consumers
    /// can treat it as a plain translatable string.
    LocaleString { content: String, inner: String },

    /// Brace expansion: `{a,b,c}` or `{1..10}`.
    BraceExpansion { content: String },

    /// Arithmetic expansion: `$(( expr ))`
    ///
    /// `parts` decomposes the expression text, exposing nested command
    /// substitutions such as `$((1 + $(cmd)))` that the arithmetic
    /// expression parser does not itself represent.
    ArithmeticExpansion {
        expression: Option<Box<Node>>,
        parts: Vec<Node>,
    },

    /// Arithmetic command: `(( expr ))`
    ArithmeticCommand {
        expression: Option<Box<Node>>,
        redirects: Vec<Node>,
        raw_content: String,
        parts: Vec<Node>,
    },

    // -- Arithmetic expression nodes --
    /// A numeric literal in arithmetic context.
    ArithNumber { value: String },

    /// A variable reference in arithmetic context.
    ArithVar { name: String },

    /// A binary operation in arithmetic context.
    ArithBinaryOp {
        op: String,
        left: Box<Node>,
        right: Box<Node>,
    },

    /// A unary operation in arithmetic context.
    ArithUnaryOp { op: String, operand: Box<Node> },

    /// Pre-increment `++var`.
    ArithPreIncr { operand: Box<Node> },

    /// Post-increment `var++`.
    ArithPostIncr { operand: Box<Node> },

    /// Pre-decrement `--var`.
    ArithPreDecr { operand: Box<Node> },

    /// Post-decrement `var--`.
    ArithPostDecr { operand: Box<Node> },

    /// Assignment in arithmetic context.
    ArithAssign {
        op: String,
        target: Box<Node>,
        value: Box<Node>,
    },

    /// Ternary `cond ? true : false`.
    ArithTernary {
        condition: Box<Node>,
        if_true: Option<Box<Node>>,
        if_false: Option<Box<Node>>,
    },

    /// Comma operator in arithmetic context.
    ArithComma { left: Box<Node>, right: Box<Node> },

    /// Array subscript in arithmetic context.
    ArithSubscript { array: String, index: Box<Node> },

    /// Empty arithmetic expression.
    ArithEmpty,

    /// An escaped character in arithmetic context.
    ArithEscape { ch: String },

    /// Deprecated `$[expr]` arithmetic.
    ArithDeprecated { expression: String },

    /// Concatenation in arithmetic context (e.g., `0x$var`).
    ArithConcat { parts: Vec<Node> },

    // -- Conditional expression nodes (`[[ ]]`) --
    /// `[[ expr ]]`
    ConditionalExpr {
        body: Box<Node>,
        redirects: Vec<Node>,
    },

    /// Unary test: `-f file`, `-z string`, etc.
    UnaryTest { op: String, operand: Box<Node> },

    /// Binary test: `a == b`, `a -nt b`, etc.
    BinaryTest {
        op: String,
        left: Box<Node>,
        right: Box<Node>,
    },

    /// `[[ a && b ]]`
    CondAnd { left: Box<Node>, right: Box<Node> },

    /// `[[ a || b ]]`
    CondOr { left: Box<Node>, right: Box<Node> },

    /// `[[ ! expr ]]`
    CondNot { operand: Box<Node> },

    /// `[[ ( expr ) ]]`
    CondParen { inner: Box<Node> },

    /// A term (word) in a conditional expression.
    ///
    /// `parts` decomposes the term, exposing nested command substitutions
    /// such as `[[ $(cmd) == x ]]`.
    CondTerm {
        value: String,
        spans: Vec<WordSpan>,
        parts: Vec<Node>,
    },

    // -- Other --
    /// Pipeline negation with `!`.
    Negation { pipeline: Box<Node> },

    /// `time [-p] pipeline`
    Time { pipeline: Box<Node>, posix: bool },

    /// Array literal: `(a b c)`.
    Array { elements: Vec<Node> },

    /// An empty node.
    Empty,

    /// A comment: `# text`.
    Comment { text: String },
}

/// Operator between commands in a list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListOperator {
    /// `&&`
    And,
    /// `||`
    Or,
    /// `;` or `\n`
    Semi,
    /// `&`
    Background,
}

/// Separator between commands in a pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipeSep {
    /// `|` — pipe stdout only.
    Pipe,
    /// `|&` — pipe both stdout and stderr.
    PipeBoth,
}

/// An item in a command list: a command with an optional trailing operator.
#[derive(Debug, Clone, PartialEq)]
pub struct ListItem {
    pub command: Node,
    pub operator: Option<ListOperator>,
}

/// A single case pattern clause within a `case` statement.
#[derive(Debug, Clone, PartialEq)]
pub struct CasePattern {
    pub patterns: Vec<Node>,
    pub body: Option<Node>,
    pub terminator: String,
}

impl CasePattern {
    pub const fn new(patterns: Vec<Node>, body: Option<Node>, terminator: String) -> Self {
        Self {
            patterns,
            body,
            terminator,
        }
    }
}

/// Visits `Option<&Node>` if present.
fn visit_opt(node: Option<&Node>, f: &mut impl FnMut(&Node)) {
    if let Some(node) = node {
        node.visit(f);
    }
}

/// Visits every node in a slice.
fn visit_slice(nodes: &[Node], f: &mut impl FnMut(&Node)) {
    for node in nodes {
        node.visit(f);
    }
}

impl Node {
    /// Visits this node and all of its descendants in depth-first order.
    ///
    /// Unlike a plain AST walk, this follows the *resolved* structure:
    /// substitutions nested inside opaque operands (parameter-expansion
    /// arguments and subscripts, arithmetic bodies, conditional terms, and
    /// unquoted here-document bodies) are reachable through the `parts`
    /// fields on their owning nodes. Consumers can therefore inspect every
    /// command without re-parsing expansion text themselves.
    #[allow(clippy::too_many_lines, clippy::match_same_arms)]
    pub fn visit(&self, f: &mut impl FnMut(&Self)) {
        f(self);
        match &self.kind {
            NodeKind::Word { parts, .. }
            | NodeKind::HereDoc { parts, .. }
            | NodeKind::ParamExpansion { parts, .. }
            | NodeKind::ParamIndirect { parts, .. }
            | NodeKind::CondTerm { parts, .. } => visit_slice(parts, f),
            NodeKind::Command {
                assignments,
                words,
                redirects,
            } => {
                visit_slice(assignments, f);
                visit_slice(words, f);
                visit_slice(redirects, f);
            }
            NodeKind::Pipeline { commands, .. } => visit_slice(commands, f),
            NodeKind::List { items } => {
                for item in items {
                    item.command.visit(f);
                }
            }
            NodeKind::If {
                condition,
                then_body,
                else_body,
                redirects,
            } => {
                condition.visit(f);
                then_body.visit(f);
                visit_opt(else_body.as_deref(), f);
                visit_slice(redirects, f);
            }
            NodeKind::While {
                condition,
                body,
                redirects,
            }
            | NodeKind::Until {
                condition,
                body,
                redirects,
            } => {
                condition.visit(f);
                body.visit(f);
                visit_slice(redirects, f);
            }
            NodeKind::For {
                words,
                body,
                redirects,
                ..
            }
            | NodeKind::Select {
                words,
                body,
                redirects,
                ..
            } => {
                if let Some(words) = words {
                    visit_slice(words, f);
                }
                body.visit(f);
                visit_slice(redirects, f);
            }
            NodeKind::ForArith {
                body, redirects, ..
            } => {
                body.visit(f);
                visit_slice(redirects, f);
            }
            NodeKind::Case {
                word,
                patterns,
                redirects,
            } => {
                word.visit(f);
                for item in patterns {
                    visit_slice(&item.patterns, f);
                    visit_opt(item.body.as_ref(), f);
                }
                visit_slice(redirects, f);
            }
            NodeKind::Function { body, .. }
            | NodeKind::Coproc { command: body, .. }
            | NodeKind::Negation { pipeline: body }
            | NodeKind::Time { pipeline: body, .. } => body.visit(f),
            NodeKind::Subshell { body, redirects } | NodeKind::BraceGroup { body, redirects } => {
                body.visit(f);
                visit_slice(redirects, f);
            }
            NodeKind::Redirect { target, .. } => target.visit(f),
            NodeKind::CommandSubstitution { command, .. }
            | NodeKind::ProcessSubstitution { command, .. } => command.visit(f),
            NodeKind::ArithmeticExpansion {
                expression, parts, ..
            } => {
                visit_opt(expression.as_deref(), f);
                visit_slice(parts, f);
            }
            NodeKind::ArithmeticCommand {
                expression,
                redirects,
                parts,
                ..
            } => {
                visit_opt(expression.as_deref(), f);
                visit_slice(redirects, f);
                visit_slice(parts, f);
            }
            NodeKind::ArithBinaryOp { left, right, .. }
            | NodeKind::ArithComma { left, right, .. } => {
                left.visit(f);
                right.visit(f);
            }
            NodeKind::ArithUnaryOp { operand, .. }
            | NodeKind::ArithPreIncr { operand }
            | NodeKind::ArithPostIncr { operand }
            | NodeKind::ArithPreDecr { operand }
            | NodeKind::ArithPostDecr { operand } => operand.visit(f),
            NodeKind::ArithAssign { target, value, .. } => {
                target.visit(f);
                value.visit(f);
            }
            NodeKind::ArithTernary {
                condition,
                if_true,
                if_false,
            } => {
                condition.visit(f);
                visit_opt(if_true.as_deref(), f);
                visit_opt(if_false.as_deref(), f);
            }
            NodeKind::ArithSubscript { index, .. } => index.visit(f),
            NodeKind::ArithConcat { parts } => visit_slice(parts, f),
            NodeKind::ConditionalExpr { body, redirects } => {
                body.visit(f);
                visit_slice(redirects, f);
            }
            NodeKind::UnaryTest { operand, .. } => operand.visit(f),
            NodeKind::BinaryTest { left, right, .. }
            | NodeKind::CondAnd { left, right }
            | NodeKind::CondOr { left, right } => {
                left.visit(f);
                right.visit(f);
            }
            NodeKind::CondNot { operand } | NodeKind::CondParen { inner: operand } => {
                operand.visit(f);
            }
            NodeKind::Array { elements } => visit_slice(elements, f),
            NodeKind::WordLiteral { .. }
            | NodeKind::ParamLength { .. }
            | NodeKind::AnsiCQuote { .. }
            | NodeKind::LocaleString { .. }
            | NodeKind::BraceExpansion { .. }
            | NodeKind::ArithNumber { .. }
            | NodeKind::ArithVar { .. }
            | NodeKind::ArithEmpty
            | NodeKind::ArithEscape { .. }
            | NodeKind::ArithDeprecated { .. }
            | NodeKind::Empty
            | NodeKind::Comment { .. } => {}
        }
    }
}
