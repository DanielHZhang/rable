#![allow(clippy::expect_used)]
//! Tests for the resolved-nesting additions: dequoted word values and the
//! structured `parts` that expose substitutions inside opaque operands.

use std::ops::ControlFlow;

use rable::NodeKind;

/// Collects the dequoted command names of every simple command reachable
/// from the parsed source, using only the public `Node::visit` helper.
fn names(source: &str) -> Vec<String> {
    let nodes = rable::parse(source, false).expect("parse");
    let mut out = Vec::new();
    for node in &nodes {
        let _ = node.visit(&mut |node| {
            if let NodeKind::Command { words, .. } = &node.kind
                && let Some(NodeKind::Word {
                    dequoted: Some(name),
                    ..
                }) = words.first().map(|w| &w.kind)
            {
                out.push(name.clone());
            }
            ControlFlow::<()>::Continue(())
        });
    }
    out
}

fn first_word_dequoted(source: &str) -> Option<String> {
    let nodes = rable::parse(source, false).expect("parse");
    let NodeKind::Command { words, .. } = &nodes.first()?.kind else {
        return None;
    };
    let NodeKind::Word { dequoted, .. } = &words.first()?.kind else {
        return None;
    };
    dequoted.clone()
}

#[test]
fn dequotes_static_words() {
    for (source, expected) in [
        ("sudo", "sudo"),
        ("'sudo'", "sudo"),
        ("\"su\"do", "sudo"),
        ("\\sudo", "sudo"),
        ("s'u'do", "sudo"),
        ("$'\\x73udo'", "sudo"),
        ("/usr/bin/sudo", "/usr/bin/sudo"),
    ] {
        assert_eq!(
            first_word_dequoted(source).as_deref(),
            Some(expected),
            "source: {source:?}"
        );
    }
}

#[test]
fn dynamic_words_have_no_static_value() {
    for source in [
        "$cmd",
        "${cmd}",
        "$(echo sudo)",
        "`echo sudo`",
        "a{b,c}",
        "<(sudo true)",
        "pre$(cmd)",
    ] {
        assert_eq!(first_word_dequoted(source), None, "source: {source:?}");
    }
}

#[test]
fn finds_command_nested_in_parameter_expansion() {
    assert_eq!(names("echo ${x:-$(sudo true)}"), ["echo", "sudo"]);
    assert_eq!(names("echo ${x:-`sudo true`}"), ["echo", "sudo"]);
    assert_eq!(names("echo ${arr[$(sudo true)]}"), ["echo", "sudo"]);
}

#[test]
fn finds_command_nested_in_arithmetic() {
    assert_eq!(names("echo $((1 + $(sudo true)))"), ["echo", "sudo"]);
    assert_eq!(names("echo $((1 + `sudo true`))"), ["echo", "sudo"]);
}

#[test]
fn finds_command_nested_in_conditional_term() {
    assert_eq!(names("[[ $(sudo true) == x ]]"), ["sudo"]);
    assert_eq!(names("[[ a$(sudo true) == x ]]"), ["sudo"]);
}

#[test]
fn finds_command_nested_in_unquoted_heredoc() {
    assert_eq!(names("cat <<EOF\n$(sudo true)\nEOF"), ["cat", "sudo"]);
    assert_eq!(names("cat <<-EOF\n\t$(sudo true)\n\tEOF"), ["cat", "sudo"]);
    // Quoted heredocs are literal: no expansion is performed.
    assert_eq!(names("cat <<'EOF'\n$(sudo true)\nEOF"), ["cat"]);
}

#[test]
fn finds_command_nested_in_command_and_process_substitutions() {
    assert_eq!(names("echo $(sudo true)"), ["echo", "sudo"]);
    assert_eq!(names("echo `sudo true`"), ["echo", "sudo"]);
    assert_eq!(names("cat <(sudo true)"), ["cat", "sudo"]);
    assert_eq!(names("echo $(echo $(sudo true))"), ["echo", "echo", "sudo"]);
}

#[test]
fn deeply_nested_substitutions_parse_in_linear_time() {
    // Regression guard for the former super-linear explosion: this used to
    // take minutes (and eventually OOM) at depth 200.
    let source = "$(".repeat(100) + "sudo true" + &")".repeat(100);
    let start = std::time::Instant::now();
    let nodes = rable::parse(&source, false).expect("parse deep nesting");
    assert!(!nodes.is_empty());
    assert!(
        start.elapsed() < std::time::Duration::from_millis(200),
        "deep nesting took {:?}",
        start.elapsed()
    );
}

#[test]
fn visit_stops_at_the_first_break() {
    let nodes = rable::parse("echo a; echo b; echo c", false).expect("parse");

    let count = |stop: bool| {
        let mut seen = 0;
        for node in &nodes {
            let flow = node.visit(&mut |_| {
                seen += 1;
                if stop && seen == 2 {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            });
            if flow.is_break() {
                break;
            }
        }
        seen
    };

    let full = count(false);
    let partial = count(true);
    assert_eq!(partial, 2);
    assert!(
        partial < full,
        "expected early stop, saw {partial} of {full}"
    );
}
