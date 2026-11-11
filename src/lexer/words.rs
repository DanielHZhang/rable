use std::ops::ControlFlow;

use crate::error::{RableError, Result};
use crate::token::{Token, TokenType};

use super::Lexer;
use super::word_builder::WordBuilder;
use super::word_shape::{assignment_value_start, is_identifier_prefix};
use crate::word_span::WordSpanKind;

impl Lexer {
    /// Reads a word token, handling quoting and expansions.
    ///
    /// Structured as three phases: character-by-character assembly via
    /// the `while let`/`match` loop, classification of the accumulated
    /// value (`classify_word`), and context-flag update for the next
    /// token (`advance_word_context`).
    pub(super) fn read_word_token(&mut self, start: usize, line: usize) -> Result<Token> {
        let mut wb = WordBuilder::new();
        while let Some(c) = self.peek_char() {
            match c {
                // Metacharacters end a word.
                ' ' | '\t' | '\n' | '|' | '&' | ';' | ')' => break,
                // Closing delimiter of a backtick fork ends the word.
                // The byte is consumed later by `exit_backtick_fork`.
                '`' if self.at_backtick_terminator() => break,
                '<' | '>' => {
                    if self.read_angle_bracket_word(&mut wb)?.is_break() {
                        break;
                    }
                }
                '(' => {
                    if self.read_open_paren_word(&mut wb)?.is_break() {
                        break;
                    }
                }
                '\'' | '"' | '\\' | '$' | '`' => {
                    self.read_word_special(&mut wb, c)?;
                }
                c if self.is_extglob_trigger(c) => {
                    self.read_extglob(&mut wb, c)?;
                }
                '[' if self.word_enters_bracket_subscript(&wb) => {
                    self.read_bracket_subscript(&mut wb)?;
                }
                _ => {
                    self.advance_char();
                    wb.push(c);
                }
            }
        }

        super::brace_expansion::detect_brace_expansions(&mut wb);

        if wb.is_empty() {
            return Err(RableError::parse("unexpected character", start, line));
        }

        let kind = self.classify_word(&wb);
        self.advance_word_context(kind);
        Ok(Token::with_spans(
            kind,
            wb.value,
            start..self.word_end(start),
            line,
            wb.spans,
        ))
    }

    /// End of the word token that started at `start`: the current
    /// position, less any line continuations the word reader consumed after
    /// its last character.
    fn word_end(&self, start: usize) -> usize {
        let mut source = &self.input[start..self.pos];
        while let Some(rest) = source.strip_suffix("\\\n") {
            source = rest;
        }
        start + source.len()
    }

    // -- word-assembly dispatch helpers --

    /// Handles a `<` or `>` encountered mid-word. If followed by `(` and
    /// the word so far is non-empty, reads a process substitution
    /// (`cat<(cmd)`) into `wb` and returns `Continue`. Otherwise the
    /// character is a metacharacter that terminates the word, and we
    /// return `Break`.
    fn read_angle_bracket_word(&mut self, wb: &mut WordBuilder) -> Result<ControlFlow<()>> {
        if !wb.is_empty() && self.peek_second_char() == Some('(') {
            self.read_process_sub_into(wb)?;
            Ok(ControlFlow::Continue(()))
        } else {
            Ok(ControlFlow::Break(()))
        }
    }

    /// Handles `(` encountered mid-word. Three outcomes:
    ///
    /// * preceded by `=` — array assignment `arr=(…)`; consume the
    ///   parenthesised element list and keep reading.
    /// * preceded by an extglob prefix (`@`, `?`, `+`, or `!`/`*` when
    ///   extglob is on) — extglob pattern `@(…)`, `*(…)`, etc.
    /// * otherwise `(` is a metacharacter that terminates the word.
    fn read_open_paren_word(&mut self, wb: &mut WordBuilder) -> Result<ControlFlow<()>> {
        if wb.ends_with('=') {
            self.advance_char();
            wb.push('(');
            self.read_array_elements(wb)?;
            Ok(ControlFlow::Continue(()))
        } else if wb.ends_with('@')
            || wb.ends_with('?')
            || wb.ends_with('+')
            || (wb.ends_with('!') && self.config.extglob)
            || (wb.ends_with('*') && self.config.extglob)
        {
            self.advance_char();
            wb.push('(');
            self.read_matched_parens(wb, 1)?;
            Ok(ControlFlow::Continue(()))
        } else {
            Ok(ControlFlow::Break(()))
        }
    }

    /// Returns true if `c` at the current position opens an extglob
    /// pattern: a prefix character (`@`, `?`, `+`, and `!`/`*` when
    /// `config.extglob` is enabled) followed immediately by `(`.
    fn is_extglob_trigger(&self, c: char) -> bool {
        if self.peek_second_char() != Some('(') {
            return false;
        }
        matches!(c, '@' | '?' | '+') || (matches!(c, '!' | '*') && self.config.extglob)
    }

    /// Returns true if a `[` at the current position should open a
    /// bracket-subscript absorption (`arr[i]=val`, `arr[i]` at command
    /// start, or regex character classes inside `[[ ]]`).
    ///
    /// In any other position a bare `[` is just an ordinary character
    /// so the word reader falls through to the `_` arm.
    fn word_enters_bracket_subscript(&self, wb: &WordBuilder) -> bool {
        (self.ctx.command_start && is_identifier_prefix(&wb.value))
            || (self.ctx.cond_expr && !wb.is_empty() && !wb.ends_with('['))
    }

    // -- word finalisation --

    /// Classifies an assembled word value into a `TokenType`.
    ///
    /// Reserved-word classification requires BOTH `command_start` and
    /// `reserved_words_ok` (issue #37): once a simple command has
    /// consumed an `AssignmentWord`, bash stops recognising reserved
    /// words in that same command even though we're still at command
    /// position. Additionally, `]]` is only a reserved word inside a
    /// `[[ ]]` conditional (issue #35); elsewhere it's an ordinary
    /// word so the parser doesn't mistake it for a terminator.
    fn classify_word(&self, wb: &WordBuilder) -> TokenType {
        let kind = if self.ctx.command_start && self.ctx.reserved_words_ok {
            TokenType::reserved_word(&wb.value).unwrap_or_else(|| {
                if assignment_value_start(&wb.value).is_some() {
                    TokenType::AssignmentWord
                } else {
                    TokenType::Word
                }
            })
        } else if assignment_value_start(&wb.value).is_some() {
            TokenType::AssignmentWord
        } else {
            TokenType::Word
        };
        if kind == TokenType::DoubleRightBracket && !self.ctx.cond_expr {
            TokenType::Word
        } else {
            kind
        }
    }

    /// Updates `command_start` / `reserved_words_ok` for the token
    /// that follows the word just emitted.
    ///
    /// `AssignmentWord` keeps `command_start=true` (assignments chain
    /// and the eventual command word is still at command position)
    /// but clears `reserved_words_ok` — no more reserved words in
    /// this simple command.
    ///
    /// Other words follow the existing rule: `command_start` re-arms
    /// only for keywords that start a new command (`then`, `else`,
    /// `elif`, `do`, `;`), and `reserved_words_ok` tracks
    /// `command_start` directly.
    fn advance_word_context(&mut self, kind: TokenType) {
        if kind == TokenType::AssignmentWord {
            self.ctx.command_start = true;
            self.ctx.reserved_words_ok = false;
        } else {
            self.ctx.command_start = kind.starts_command()
                || matches!(
                    kind,
                    TokenType::Then
                        | TokenType::Else
                        | TokenType::Elif
                        | TokenType::Do
                        | TokenType::Semi
                );
            self.ctx.reserved_words_ok = self.ctx.command_start;
        }
    }

    /// Reads a quoted string, escape, dollar expansion, or backtick within a word.
    pub(super) fn read_word_special(&mut self, wb: &mut WordBuilder, c: char) -> Result<()> {
        match c {
            '\'' => {
                let start = wb.span_start();
                self.advance_char();
                wb.push('\'');
                self.read_single_quoted(wb)?;
                wb.record(start, WordSpanKind::SingleQuoted);
            }
            '"' => {
                let start = wb.span_start();
                self.advance_char();
                wb.push('"');
                self.read_double_quoted(wb)?;
                wb.record(start, WordSpanKind::DoubleQuoted);
            }
            '\\' => {
                self.advance_char();
                if self.peek_char() == Some('\n') {
                    self.advance_char(); // line continuation — no span
                } else {
                    let start = wb.span_start();
                    wb.push('\\');
                    if let Some(next) = self.advance_char() {
                        wb.push(next);
                    } else {
                        // Trailing \ at EOF — bash keeps it as literal \\
                        wb.push('\\');
                    }
                    wb.record(start, WordSpanKind::Escape);
                }
            }
            '$' => {
                self.read_dollar(wb)?;
            }
            '`' => {
                let start = wb.span_start();
                self.advance_char();
                wb.push('`');
                self.read_backtick(wb)?;
                wb.record(start, WordSpanKind::Backtick);
            }
            _ => {}
        }
        Ok(())
    }

    /// Reads an extglob pattern `@(...)`, `?(...)`, etc.
    pub(super) fn read_extglob(&mut self, wb: &mut WordBuilder, prefix: char) -> Result<()> {
        let start = wb.span_start();
        self.advance_char();
        wb.push(prefix);
        self.advance_char();
        wb.push('(');
        self.read_matched_parens(wb, 1)?;
        wb.record(start, WordSpanKind::Extglob(prefix));
        Ok(())
    }

    /// Reads a bracket subscript `[...]` inside a word.
    ///
    /// Called only when the word so far is a plain identifier and we are at
    /// command-start position — i.e. an `arr[idx]=value` assignment or an
    /// `arr[idx]` command invocation. In this position bash's parser is
    /// permissive about whitespace and metacharacters inside `[...]`, so
    /// we absorb them into the word verbatim until the matching `]`.
    pub(super) fn read_bracket_subscript(&mut self, wb: &mut WordBuilder) -> Result<()> {
        let start = wb.span_start();
        self.advance_char(); // consume [
        wb.push('[');
        let mut depth = 1;
        while let Some(c) = self.peek_char() {
            match c {
                '[' => {
                    depth += 1;
                    self.advance_char();
                    wb.push(c);
                }
                ']' => {
                    depth -= 1;
                    self.advance_char();
                    wb.push(c);
                    if depth == 0 {
                        wb.record(start, WordSpanKind::BracketSubscript);
                        return Ok(());
                    }
                }
                '\'' => {
                    self.advance_char();
                    wb.push('\'');
                    self.read_single_quoted(wb)?;
                }
                '"' => {
                    self.advance_char();
                    wb.push('"');
                    self.read_double_quoted(wb)?;
                }
                '\\' => {
                    self.advance_char();
                    wb.push('\\');
                    if let Some(nc) = self.advance_char() {
                        wb.push(nc);
                    }
                }
                '$' => {
                    self.read_dollar(wb)?;
                }
                _ => {
                    self.advance_char();
                    wb.push(c);
                }
            }
        }
        Ok(())
    }

    /// Reads a process substitution into an existing word value.
    pub(super) fn read_process_sub_into(&mut self, wb: &mut WordBuilder) -> Result<()> {
        let start = wb.span_start();
        let dir = self.advance_char().unwrap_or('<');
        wb.push(dir);
        self.advance_char(); // (
        wb.push('(');
        self.read_paren_body_forked(wb)?;
        wb.record(start, WordSpanKind::ProcessSub(dir));
        Ok(())
    }

    /// Reads a process substitution `<(...)` or `>(...)` as a word token.
    /// Continues reading word characters after the closing `)`.
    pub(super) fn read_process_sub_word(&mut self, start: usize, line: usize) -> Result<Token> {
        let mut wb = WordBuilder::new();
        let span_start = wb.span_start();
        // Read < or >
        let dir = self.advance_char().unwrap_or('<');
        wb.push(dir);
        // Read (
        self.advance_char();
        wb.push('(');
        // Read until matching ) via fork-and-merge
        self.read_paren_body_forked(&mut wb)?;
        wb.record(span_start, WordSpanKind::ProcessSub(dir));
        // Continue reading word chars after the process substitution
        self.continue_word(&mut wb)?;
        self.ctx.command_start = false;
        self.ctx.reserved_words_ok = false;
        Ok(Token::with_spans(
            TokenType::Word,
            wb.value,
            start..self.word_end(start),
            line,
            wb.spans,
        ))
    }

    /// Continue reading word characters after a special construct.
    fn continue_word(&mut self, wb: &mut WordBuilder) -> Result<()> {
        while let Some(c) = self.peek_char() {
            match c {
                ' ' | '\t' | '\n' | '|' | '&' | ';' | ')' | '(' => break,
                '<' | '>' => {
                    if self.peek_second_char() == Some('(') {
                        self.read_process_sub_into(wb)?;
                    } else {
                        break;
                    }
                }
                '\'' | '"' | '\\' | '$' | '`' => {
                    self.read_word_special(wb, c)?;
                }
                _ => {
                    self.advance_char();
                    wb.push(c);
                }
            }
        }
        Ok(())
    }

    /// Reads array elements from `(...)`, producing normalized content.
    ///
    /// Parses individual elements separated by whitespace directly from
    /// the input stream, stripping comments. Uses the lexer's existing
    /// word-reading facilities for quoting and expansion handling.
    /// The opening `(` must already be consumed and pushed to `wb`.
    fn read_array_elements(&mut self, wb: &mut WordBuilder) -> Result<()> {
        let mut need_space = false;
        loop {
            // Skip whitespace between elements
            while matches!(self.peek_char(), Some(' ' | '\t' | '\n' | '\r')) {
                self.advance_char();
            }
            match self.peek_char() {
                Some(')') => {
                    self.advance_char();
                    wb.push(')');
                    return Ok(());
                }
                Some('#') => {
                    // Comment — skip to end of line
                    while matches!(self.peek_char(), Some(c) if c != '\n') {
                        self.advance_char();
                    }
                }
                Some(_) => {
                    if need_space {
                        wb.push(' ');
                    }
                    self.read_array_element(wb)?;
                    need_space = true;
                }
                None => {
                    return Err(RableError::matched_pair(
                        "unterminated array",
                        self.pos,
                        self.line,
                    ));
                }
            }
        }
    }

    /// Reads a single array element using the same word-reading logic
    /// as `read_word_token`, but with `)` and whitespace as terminators
    /// instead of the standard shell metacharacters.
    fn read_array_element(&mut self, wb: &mut WordBuilder) -> Result<()> {
        while let Some(c) = self.peek_char() {
            match c {
                ' ' | '\t' | '\n' | '\r' | ')' => break,
                '<' | '>' => {
                    if self.peek_second_char() == Some('(') {
                        self.read_process_sub_into(wb)?;
                    } else {
                        self.advance_char();
                        wb.push(c);
                    }
                }
                '\'' | '"' | '\\' | '$' | '`' => {
                    self.read_word_special(wb, c)?;
                }
                _ => {
                    self.advance_char();
                    wb.push(c);
                }
            }
        }
        Ok(())
    }
}
