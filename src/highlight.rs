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
                    | lex::TokenKind::Then
                    | lex::TokenKind::Else
                    | lex::TokenKind::Struct
                    | lex::TokenKind::Let
                    | lex::TokenKind::Const
                    | lex::TokenKind::Global
                    | lex::TokenKind::Yield
                    | lex::TokenKind::Continue
                    | lex::TokenKind::Asm => {
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
                    lex::TokenKind::LeftParen
                    | lex::TokenKind::RightParen
                    | lex::TokenKind::LeftBrace
                    | lex::TokenKind::RightBrace
                    | lex::TokenKind::LeftBracket
                    | lex::TokenKind::RightBracket
                    | lex::TokenKind::Comma
                    | lex::TokenKind::Semicolon
                    | lex::TokenKind::Colon
                    | lex::TokenKind::Equal
                    | lex::TokenKind::EqualEqual
                    | lex::TokenKind::Ampersand
                    | lex::TokenKind::AmpersandAmpersand
                    | lex::TokenKind::Pipe
                    | lex::TokenKind::PipePipe
                    | lex::TokenKind::Bang
                    | lex::TokenKind::BangEqual
                    | lex::TokenKind::Hash
                    | lex::TokenKind::Dot
                    | lex::TokenKind::Carrot
                    | lex::TokenKind::Plus
                    | lex::TokenKind::Minus
                    | lex::TokenKind::MinusGreaterThan
                    | lex::TokenKind::LessThan
                    | lex::TokenKind::LessThanLessThan
                    | lex::TokenKind::LessThanEqual
                    | lex::TokenKind::LessThanPercent
                    | lex::TokenKind::GreaterThan
                    | lex::TokenKind::GreaterThanGreaterThan
                    | lex::TokenKind::GreaterThanEqual
                    | lex::TokenKind::GreaterThanPercent
                    | lex::TokenKind::PlusEqual
                    | lex::TokenKind::MinusEqual
                    | lex::TokenKind::AmpersandEqual
                    | lex::TokenKind::PipeEqual
                    | lex::TokenKind::CarrotEqual
                    | lex::TokenKind::Ident
                    | lex::TokenKind::Underscore
                    | lex::TokenKind::Eof => {}
                },
                Err(_) => break,
            }
        }
        parts.push(owo_colors::Style::default().style(&line[offset..]));
        parts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn highlight(line: &str) -> Vec<owo_colors::Styled<&str>> {
        QuokkaHighlighterState.highlight_line(line)
    }

    fn plain() -> owo_colors::Style {
        owo_colors::Style::default()
    }

    #[test]
    fn line_with_no_keywords_or_numbers_is_a_single_plain_part() {
        let parts = highlight("x + y");
        assert_eq!(parts.len(), 1);
        assert_eq!(*parts[0].inner(), "x + y");
        assert_eq!(parts[0].style, plain());
    }

    #[test]
    fn keyword_is_split_out_and_styled_red() {
        let parts = highlight("fn main");
        // plain "" before, red "fn", plain " main" after
        assert_eq!(parts.len(), 3);
        assert_eq!(*parts[0].inner(), "");
        assert_eq!(*parts[1].inner(), "fn");
        assert_eq!(parts[1].style, plain().red());
        assert_eq!(*parts[2].inner(), " main");
        assert_eq!(parts[2].style, plain());
    }

    #[test]
    fn number_is_split_out_and_styled_blue() {
        let parts = highlight("let x = 42");
        // plain "" before "let", red "let", plain " x = " before number, blue "42", plain "" after
        assert_eq!(parts.len(), 5);
        assert_eq!(*parts[1].inner(), "let");
        assert_eq!(parts[1].style, plain().red());
        assert_eq!(*parts[3].inner(), "42");
        assert_eq!(parts[3].style, plain().blue());
    }

    #[test]
    fn non_highlighted_token_kinds_are_left_unstyled() {
        // `main` is an identifier, not a keyword, so it should not be split out.
        let parts = highlight("main()");
        assert_eq!(parts.len(), 1);
        assert_eq!(*parts[0].inner(), "main()");
        assert_eq!(parts[0].style, plain());
    }

    #[test]
    fn lexer_error_stops_highlighting_but_keeps_source_intact() {
        // An unterminated/invalid token should not cause a panic, and the raw
        // remainder of the line must still be emitted verbatim.
        let parts = highlight("fn `");
        let rendered = parts
            .iter()
            .map(|part| *part.inner())
            .collect::<Vec<_>>()
            .concat();
        assert_eq!(rendered, "fn `");
    }
}
