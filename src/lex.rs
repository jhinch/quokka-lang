use miette::{SourceOffset, SourceSpan};
use std::borrow::Cow;
use std::num::{IntErrorKind, NonZeroU32};
use std::str::Chars;

#[derive(Debug, Clone)]
pub struct Token<'de> {
    text: &'de str,
    offset: usize,
    kind: TokenKind,
}

impl Token<'_> {
    pub fn span(&self) -> SourceSpan {
        SourceSpan::new(SourceOffset::from(self.offset), self.text.len())
    }

    pub fn kind(&self) -> TokenKind {
        self.kind
    }

    pub fn text(&self) -> &str {
        self.text
    }

    pub fn try_parse_u32(&self) -> Result<u32, Error> {
        let text = if self.text.contains('_') {
            Cow::Owned(self.text.replace('_', ""))
        } else {
            Cow::Borrowed(self.text)
        };
        match &text[0..2.min(text.len())] {
            "0x" => u32::from_str_radix(&text[2..], 16),
            "0o" => u32::from_str_radix(&text[2..], 8),
            "0b" => u32::from_str_radix(&text[2..], 2),
            _ => u32::from_str_radix(&text, 10),
        }
        .map_err(|source| {
            Error::from(InvalidNumberError {
                span: self.span(),
                kind: source.kind().clone(),
            })
        })
    }

    pub fn try_parse_non_zero_u32(&self) -> Result<NonZeroU32, Error> {
        let value = self.try_parse_u32()?;
        NonZeroU32::try_from(value).map_err(|_| {
            Error::from(InvalidNumberError {
                span: self.span(),
                kind: IntErrorKind::Zero,
            })
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    // separators
    LeftParen,
    RightParen,
    LeftBrace,
    RightBrace,
    LeftBracket,
    RightBracket,
    Comma,
    Semicolon,
    Colon,
    // operators
    Equal,
    EqualEqual,
    Star,
    Ampersand,
    AmpersandAmpersand,
    Pipe,
    PipePipe,
    Bang,
    BangEqual,
    Hash,
    Dot,
    Carrot,
    Plus,
    Minus,
    MinusGreaterThan,
    LessThan,
    LessThanLessThan,
    LessThanEqual,
    LessThanPercent,
    GreaterThan,
    GreaterThanGreaterThan,
    GreaterThanEqual,
    GreaterThanPercent,
    PlusEqual,
    MinusEqual,
    AmpersandEqual,
    PipeEqual,
    CarrotEqual,
    LessThanLessThanEqual,
    GreaterThanGreaterThanEqual,
    LessThanPercentEqual,
    GreaterThanPercentEqual,
    // literals
    Number,
    // identifiers
    Ident,
    // keywords
    Fn,
    Let,
    Loop,
    Break,
    Yield,
    Return,
    Continue,
    If,
    Then,
    Else,
    Struct,
    Global,
    Const,
    Asm,
    // other
    Eof,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic, Clone)]
pub enum Error {
    #[error("Unexpected EOF")]
    #[diagnostic()]
    UnexpectedEof,
    #[error(transparent)]
    #[diagnostic(transparent)]
    UnexpectedChar(#[from] UnexpectedCharError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    InvalidNumber(#[from] InvalidNumberError),
}

#[derive(Debug, thiserror::Error, miette::Diagnostic, Clone)]
#[error("Unexpected character {unexpected:?}")]
#[diagnostic()]
pub struct UnexpectedCharError {
    #[label("this character")]
    pub span: SourceSpan,
    pub unexpected: char,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic, Clone)]
#[error("Invalid number. {kind:?}")]
#[diagnostic()]
pub struct InvalidNumberError {
    #[label("this number")]
    pub span: SourceSpan,
    pub kind: IntErrorKind,
}

pub struct Lexer<'de> {
    src: &'de str,
    remaining: Chars<'de>,
    offset: usize,
    eof: bool,
    peeked: Option<Result<Token<'de>, Error>>,
    current: Option<Token<'de>>,
}

impl<'de> Lexer<'de> {
    pub fn peek(&mut self) -> Option<&Result<Token<'de>, Error>> {
        if self.peeked.is_none() {
            self.peeked = self.advance();
        }
        self.peeked.as_ref()
    }

    pub fn current(&self) -> Option<&Token<'de>> {
        self.current.as_ref()
    }

    fn token(&self, kind: TokenKind, offset: usize, len: usize) -> Token<'de> {
        Token {
            offset,
            kind,
            text: &self.src[offset..(offset + len)],
        }
    }

    fn choose_if_equal_next(
        &mut self,
        true_kind: TokenKind,
        false_kind: TokenKind,
        offset: usize,
        len: usize,
    ) -> Token<'de> {
        let (kind, len) = if self.remaining.as_str().starts_with("=") {
            let additional_len = self.remaining.next().unwrap().len_utf8();
            self.offset += additional_len;
            (true_kind, len + additional_len)
        } else {
            (false_kind, len)
        };
        self.token(kind, offset, len)
    }

    fn advance(&mut self) -> Option<Result<Token<'de>, Error>> {
        Some(loop {
            let offset = self.offset;
            let c = match self.remaining.next() {
                Some(c) => c,
                None if self.eof => return None,
                None => {
                    self.eof = true;
                    break Ok(self.token(TokenKind::Eof, self.offset, 0));
                }
            };
            self.offset += c.len_utf8();
            break Ok(match c {
                '(' => self.token(TokenKind::LeftParen, offset, c.len_utf8()),
                ')' => self.token(TokenKind::RightParen, offset, c.len_utf8()),
                '{' => self.token(TokenKind::LeftBrace, offset, c.len_utf8()),
                '}' => self.token(TokenKind::RightBrace, offset, c.len_utf8()),
                '[' => self.token(TokenKind::LeftBracket, offset, c.len_utf8()),
                ']' => self.token(TokenKind::RightBracket, offset, c.len_utf8()),
                ',' => self.token(TokenKind::Comma, offset, c.len_utf8()),
                ';' => self.token(TokenKind::Semicolon, offset, c.len_utf8()),
                ':' => self.token(TokenKind::Colon, offset, c.len_utf8()),
                '*' => self.token(TokenKind::Star, offset, c.len_utf8()),
                '#' => self.token(TokenKind::Hash, offset, c.len_utf8()),
                '.' => self.token(TokenKind::Dot, offset, c.len_utf8()),
                '=' => self.choose_if_equal_next(
                    TokenKind::EqualEqual,
                    TokenKind::Equal,
                    offset,
                    c.len_utf8(),
                ),
                '!' => self.choose_if_equal_next(
                    TokenKind::BangEqual,
                    TokenKind::Bang,
                    offset,
                    c.len_utf8(),
                ),
                '+' => self.choose_if_equal_next(
                    TokenKind::PlusEqual,
                    TokenKind::Plus,
                    offset,
                    c.len_utf8(),
                ),
                '^' => self.choose_if_equal_next(
                    TokenKind::CarrotEqual,
                    TokenKind::Carrot,
                    offset,
                    c.len_utf8(),
                ),
                '-' => {
                    if self.remaining.as_str().starts_with(">") {
                        let additional_len = self.remaining.next().unwrap().len_utf8();
                        self.offset += additional_len;
                        let len = c.len_utf8() + additional_len;
                        self.token(TokenKind::MinusGreaterThan, offset, len)
                    } else {
                        self.choose_if_equal_next(
                            TokenKind::MinusEqual,
                            TokenKind::Minus,
                            offset,
                            c.len_utf8(),
                        )
                    }
                }
                '&' => {
                    if self.remaining.as_str().starts_with("&") {
                        let additional_len = self.remaining.next().unwrap().len_utf8();
                        self.offset += additional_len;
                        let len = c.len_utf8() + additional_len;
                        self.token(TokenKind::AmpersandAmpersand, offset, len)
                    } else {
                        self.choose_if_equal_next(
                            TokenKind::AmpersandEqual,
                            TokenKind::Ampersand,
                            offset,
                            c.len_utf8(),
                        )
                    }
                }
                '|' => {
                    if self.remaining.as_str().starts_with("|") {
                        let additional_len = self.remaining.next().unwrap().len_utf8();
                        self.offset += additional_len;
                        let len = c.len_utf8() + additional_len;
                        self.token(TokenKind::PipePipe, offset, len)
                    } else {
                        self.choose_if_equal_next(
                            TokenKind::PipeEqual,
                            TokenKind::Pipe,
                            offset,
                            c.len_utf8(),
                        )
                    }
                }
                '<' => {
                    if self.remaining.as_str().starts_with("<") {
                        let additional_len = self.remaining.next().unwrap().len_utf8();
                        self.offset += additional_len;
                        let len = c.len_utf8() + additional_len;
                        self.choose_if_equal_next(
                            TokenKind::LessThanLessThanEqual,
                            TokenKind::LessThanLessThan,
                            offset,
                            len,
                        )
                    } else if self.remaining.as_str().starts_with("%") {
                        let additional_len = self.remaining.next().unwrap().len_utf8();
                        self.offset += additional_len;
                        let len = c.len_utf8() + additional_len;
                        self.choose_if_equal_next(
                            TokenKind::LessThanPercentEqual,
                            TokenKind::LessThanPercent,
                            offset,
                            len,
                        )
                    } else {
                        self.choose_if_equal_next(
                            TokenKind::LessThanEqual,
                            TokenKind::LessThan,
                            offset,
                            c.len_utf8(),
                        )
                    }
                }
                '>' => {
                    if self.remaining.as_str().starts_with(">") {
                        let additional_len = self.remaining.next().unwrap().len_utf8();
                        self.offset += additional_len;
                        let len = c.len_utf8() + additional_len;
                        self.choose_if_equal_next(
                            TokenKind::GreaterThanGreaterThanEqual,
                            TokenKind::GreaterThanGreaterThan,
                            offset,
                            len,
                        )
                    } else if self.remaining.as_str().starts_with("%") {
                        let additional_len = self.remaining.next().unwrap().len_utf8();
                        self.offset += additional_len;
                        let len = c.len_utf8() + additional_len;
                        self.choose_if_equal_next(
                            TokenKind::GreaterThanPercentEqual,
                            TokenKind::GreaterThanPercent,
                            offset,
                            len,
                        )
                    } else {
                        self.choose_if_equal_next(
                            TokenKind::GreaterThanEqual,
                            TokenKind::GreaterThan,
                            offset,
                            c.len_utf8(),
                        )
                    }
                }
                '/' if self.remaining.as_str().starts_with("/") => {
                    // comment, skip until the end of the line
                    let skip_len = self
                        .remaining
                        .as_str()
                        .find('\n')
                        .unwrap_or(self.remaining.as_str().len());
                    self.offset += skip_len;
                    self.remaining = self.remaining.as_str()[skip_len..].chars();
                    continue;
                }
                '0' => {
                    let additional_len = match self.remaining.clone().next() {
                        Some('x') => {
                            // hexidecimal
                            self.remaining.as_str()[1..]
                                .find(
                                    |c: char| !matches!(c, '0'..='9' | 'a'..='f' | 'A'..='F' | '_'),
                                )
                                .unwrap_or(self.remaining.as_str().len())
                                + 1
                        }
                        Some('o') => {
                            // octal
                            self.remaining.as_str()[1..]
                                .find(|c: char| !matches!(c, '0'..='7' | '_'))
                                .unwrap_or(self.remaining.as_str().len())
                                + 1
                        }
                        Some('b') => {
                            // binary
                            self.remaining.as_str()[1..]
                                .find(|c: char| !matches!(c, '0'..='1' | '_'))
                                .unwrap_or(self.remaining.as_str().len())
                                + 1
                        }
                        _ => {
                            // treat all other scenarios as a decimal number
                            self.remaining
                                .as_str()
                                .find(|c: char| !matches!(c, '0'..='9' | '_'))
                                .unwrap_or(self.remaining.as_str().len())
                        }
                    };
                    self.offset += additional_len;
                    self.remaining = self.remaining.as_str()[additional_len..].chars();
                    self.token(TokenKind::Number, offset, c.len_utf8() + additional_len)
                }
                '1'..='9' => {
                    // decimal number
                    let additional_len = self
                        .remaining
                        .as_str()
                        .find(|c: char| !matches!(c, '0'..='9' | '_'))
                        .unwrap_or(self.remaining.as_str().len());
                    self.offset += additional_len;
                    self.remaining = self.remaining.as_str()[additional_len..].chars();
                    self.token(TokenKind::Number, offset, c.len_utf8() + additional_len)
                }
                c if c.is_whitespace() => continue,
                c if unicode_ident::is_xid_start(c) || c == '_' => {
                    let additional_len = self
                        .remaining
                        .as_str()
                        .find(|c: char| !unicode_ident::is_xid_continue(c))
                        .unwrap_or(self.remaining.as_str().len());
                    self.offset += additional_len;
                    self.remaining = self.remaining.as_str()[additional_len..].chars();
                    let len = c.len_utf8() + additional_len;
                    match &self.src[offset..(offset + len)] {
                        "fn" => self.token(TokenKind::Fn, offset, len),
                        "return" => self.token(TokenKind::Return, offset, len),
                        "break" => self.token(TokenKind::Break, offset, len),
                        "yield" => self.token(TokenKind::Yield, offset, len),
                        "continue" => self.token(TokenKind::Continue, offset, len),
                        "if" => self.token(TokenKind::If, offset, len),
                        "then" => self.token(TokenKind::Then, offset, len),
                        "else" => self.token(TokenKind::Else, offset, len),
                        "loop" => self.token(TokenKind::Loop, offset, len),
                        "let" => self.token(TokenKind::Let, offset, len),
                        "struct" => self.token(TokenKind::Struct, offset, len),
                        "global" => self.token(TokenKind::Global, offset, len),
                        "const" => self.token(TokenKind::Const, offset, len),
                        "asm" => self.token(TokenKind::Asm, offset, len),
                        _ => self.token(TokenKind::Ident, offset, len),
                    }
                }
                unexpected => {
                    break Err(Error::from(UnexpectedCharError {
                        span: SourceSpan::new(SourceOffset::from(offset), c.len_utf8()),
                        unexpected,
                    }));
                }
            });
        })
    }
}

impl<'de> From<&'de str> for Lexer<'de> {
    fn from(src: &'de str) -> Self {
        Lexer {
            src,
            remaining: src.chars(),
            offset: 0,
            eof: false,
            peeked: None,
            current: None,
        }
    }
}

impl<'de> Iterator for Lexer<'de> {
    type Item = Result<Token<'de>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let result = if let Some(peeked) = self.peeked.take() {
            Some(peeked)
        } else {
            self.advance()
        };

        self.current = None;

        match result {
            None => None,
            Some(Err(err)) => Some(Err(err)),
            Some(Ok(token)) => {
                self.current = Some(token.clone());
                Some(Ok(token))
            }
        }
    }
}
