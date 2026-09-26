//! Command, process, and arithmetic substitution (`$(…)`, `<(…)`, `$((…))`).
//! Depth-guarded via [`super::DepthGuard`] to prevent runaway recursion on
//! inputs like `$($($($($(…))))))`.

use crate::ast::{ListItem, Node, NodeKind, Span};
use crate::lexer::word_builder::WordSpan;

use super::{DepthGuard, decompose_region, literal_fallback};

/// Decomposes the inner text of a `$((...))` arithmetic substitution
/// into a typed `ArithmeticExpansion` node. On parse failure the
/// `expression` field is left as `None`, matching the existing best-effort
/// semantics used elsewhere in word decomposition.
///
/// `parts` additionally decomposes the expression text, so command
/// substitutions the arithmetic expression parser cannot represent
/// (e.g. `$((1 + $(cmd)))`) are still reachable.
pub(super) fn arithmetic_sub_to_node(
    inner: &str,
    value: &str,
    spans: &mut [WordSpan],
    index: usize,
) -> Node {
    let expression = crate::parser::arithmetic::parse_arith_expression(inner)
        .ok()
        .map(Box::new);
    let (start, end) = (spans[index].start, spans[index].end);
    let parts = decompose_region(value, spans, start + 3, end.saturating_sub(2));
    Node::empty(NodeKind::ArithmeticExpansion { expression, parts })
}

pub(super) fn cmdsub_to_node(content: &str) -> Node {
    parse_sub(content, &format!("$({content})"), |cmd| {
        // brace: false — this is $(...) syntax, not ${...} brace expansion
        NodeKind::CommandSubstitution {
            command: cmd,
            brace: false,
        }
    })
}

pub(super) fn procsub_to_node(direction: char, content: &str) -> Node {
    parse_sub(content, &format!("{direction}({content})"), |cmd| {
        NodeKind::ProcessSubstitution {
            direction: direction.to_string(),
            command: cmd,
        }
    })
}

/// Shared logic for parsing command/process substitution content.
fn parse_sub(
    content: &str,
    fallback_value: &str,
    make_kind: impl FnOnce(Box<Node>) -> NodeKind,
) -> Node {
    let Some(_guard) = DepthGuard::enter() else {
        return literal_fallback(fallback_value);
    };
    crate::parse(content, false).map_or_else(
        |_| literal_fallback(fallback_value),
        |nodes| Node::empty(make_kind(Box::new(wrap_nodes(nodes)))),
    )
}

/// Wraps a list of parsed nodes into a single node.
fn wrap_nodes(mut nodes: Vec<Node>) -> Node {
    match nodes.len() {
        0 => Node::empty(NodeKind::Empty),
        1 => nodes.swap_remove(0),
        _ => Node::new(
            NodeKind::List {
                items: nodes
                    .into_iter()
                    .map(|n| ListItem {
                        command: n,
                        operator: None,
                    })
                    .collect(),
            },
            Span::empty(),
        ),
    }
}
