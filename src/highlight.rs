use crate::lex;
use miette::SpanContents;
use miette::highlighters::{BlankHighlighter, Highlighter, HighlighterState};
pub struct QuokkaHighlighter;

impl Highlighter for QuokkaHighlighter {
    fn start_highlighter_state<'h>(
        &'h self,
        source: &dyn SpanContents<'_>,
    ) -> Box<dyn HighlighterState + 'h> {
        match source.language() {
            None | Some("quokka") => Box::new(QuokkaHighlighterState),
            Some(_) => BlankHighlighter.start_highlighter_state(source),
        }
    }
}

pub struct QuokkaHighlighterState;

impl HighlighterState for QuokkaHighlighterState {
    fn highlight_line<'s>(&mut self, line: &'s str) -> Vec<owo_colors::Styled<&'s str>> {
        let mut parts = Vec::new();
        let mut offset = 0;
        for token in lex::Lexer::from(line) {
            match token {
                Ok(token) => match token.kind() {
                    lex::TokenKind::Fn
                    | lex::TokenKind::Loop
                    | lex::TokenKind::Return
                    | lex::TokenKind::Break
                    | lex::TokenKind::If
                    | lex::TokenKind::Else
                    | lex::TokenKind::Struct
                    | lex::TokenKind::Let
                    | lex::TokenKind::Const
                    | lex::TokenKind::Global => {
                        parts.push(
                            owo_colors::Style::default()
                                .style(&line[offset..token.span().offset()]),
                        );
                        let next_offset = token.span().offset() + token.span().len();
                        parts.push(
                            owo_colors::Style::default()
                                .red()
                                .style(&line[token.span().offset()..next_offset]),
                        );
                        offset = next_offset;
                    }
                    lex::TokenKind::Number => {
                        parts.push(
                            owo_colors::Style::default()
                                .style(&line[offset..token.span().offset()]),
                        );
                        let next_offset = token.span().offset() + token.span().len();
                        parts.push(
                            owo_colors::Style::default()
                                .blue()
                                .style(&line[token.span().offset()..next_offset]),
                        );
                        offset = next_offset;
                    }
                    _ => {}
                },
                Err(_) => break,
            }
        }
        parts.push(owo_colors::Style::default().style(&line[offset..]));
        parts
    }
}
