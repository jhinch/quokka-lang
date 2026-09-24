use crate::ast::*;
use crate::feature::{PreviewFeature, PreviewFeatureError, PreviewFeatureSet};
use crate::lex::{self, Token, TokenKind};
use crate::multi::{MultiResult, MultiResultBuilder};
use crate::source::span::span_between;
use miette::SourceSpan;
use std::fmt;
use tracing::instrument;

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum Error {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Lex(#[from] lex::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Syntax(#[from] SyntaxError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    MissingSeparator(#[from] MissingSeparatorError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    MissingTypeAnnotation(#[from] MissingTypeAnnotationError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Preview(#[from] PreviewFeatureError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    ChainedComparison(#[from] ChainedComparisonError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    CallOnValue(#[from] CallOnValueError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    ArrayIndexOnValue(#[from] ArrayIndexOnValueError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    FieldAccessOnValue(#[from] FieldAccessOnValueError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    AssignOnValue(#[from] AssignOnValueError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    UnnamedIdentifier(#[from] UnnamedIdentifierError),
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("Syntax error: found {unexpected} when expecting {expected}")]
#[diagnostic()]
pub struct SyntaxError {
    #[label("this token")]
    pub span: SourceSpan,
    pub unexpected: Unexpected,
    pub expected: Expected,
}

impl SyntaxError {
    /// Creates a syntax error for `token` not matching what the parser expected.
    ///
    /// Modeled on serde's `Error::invalid_type(unexp, exp)`: the found token and the
    /// expectation are each rendered by their own type, so a fallback branch can describe an
    /// expectation in words rather than listing every token kind it would have accepted.
    fn new(token: &Token<'_>, expected: impl Into<Expected>) -> Self {
        SyntaxError {
            span: token.span(),
            unexpected: Unexpected::from(token),
            expected: expected.into(),
        }
    }
}

/// What the parser found instead of what it expected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unexpected {
    /// A token whose kind alone describes it, such as a keyword or punctuation.
    Token(TokenKind),
    /// An identifier, along with its name.
    Ident(String),
    /// A number literal, along with its source text.
    Number(String),
    /// The end of the source.
    Eof,
}

impl From<&Token<'_>> for Unexpected {
    fn from(token: &Token<'_>) -> Self {
        match token.kind() {
            TokenKind::Ident => Unexpected::Ident(token.text().to_owned()),
            TokenKind::Number => Unexpected::Number(token.text().to_owned()),
            TokenKind::Eof => Unexpected::Eof,
            kind => Unexpected::Token(kind),
        }
    }
}

impl fmt::Display for Unexpected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unexpected::Token(kind) => write!(f, "{}", DescribeTokenKind(*kind)),
            Unexpected::Ident(name) => write!(f, "identifier `{name}`"),
            Unexpected::Number(text) => write!(f, "number `{text}`"),
            Unexpected::Eof => f.write_str("end of file"),
        }
    }
}

/// What the parser expected to find.
///
/// Short, specific expectations are best expressed as [`Expected::OneOf`] so the user sees
/// exactly which tokens would have been accepted. When that list would be long and made up of
/// low-level token kinds (the start of an expression, any operator), use
/// [`Expected::Description`] with a human phrase instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expected {
    /// One of the listed token kinds.
    OneOf(Vec<TokenKind>),
    /// A description of the expected syntax, such as "an expression".
    Description(&'static str),
}

impl From<TokenKind> for Expected {
    fn from(kind: TokenKind) -> Self {
        Expected::OneOf(vec![kind])
    }
}

impl<const N: usize> From<[TokenKind; N]> for Expected {
    fn from(kinds: [TokenKind; N]) -> Self {
        Expected::OneOf(kinds.to_vec())
    }
}

impl From<&'static str> for Expected {
    fn from(description: &'static str) -> Self {
        Expected::Description(description)
    }
}

impl fmt::Display for Expected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expected::Description(description) => f.write_str(description),
            Expected::OneOf(kinds) => match kinds.as_slice() {
                [] => f.write_str("nothing"),
                [kind] => write!(f, "{}", DescribeTokenKind(*kind)),
                [first, middle @ .., last] => {
                    write!(f, "one of {}", DescribeTokenKind(*first))?;
                    for kind in middle {
                        write!(f, ", {}", DescribeTokenKind(*kind))?;
                    }
                    write!(f, " or {}", DescribeTokenKind(*last))
                }
            },
        }
    }
}

/// Renders a token kind for a diagnostic: tokens with fixed text are quoted, while token kinds
/// that stand for a class of text are named.
struct DescribeTokenKind(TokenKind);

impl fmt::Display for DescribeTokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            TokenKind::Ident => f.write_str("an identifier"),
            TokenKind::Number => f.write_str("a number"),
            TokenKind::Comment => f.write_str("a comment"),
            TokenKind::Eof => f.write_str("end of file"),
            kind => write!(f, "`{kind}`"),
        }
    }
}

/// A separator (`,` or `;`) is missing between two adjacent list items or statements, or at the
/// end of a declaration or statement.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("Syntax error: missing `{separator}` {context}")]
#[diagnostic()]
pub struct MissingSeparatorError {
    /// A zero-width span where the separator should go, just after the preceding token.
    #[label(primary, "add `{separator}` here")]
    pub span: SourceSpan,
    /// The token that follows where the separator should be, if there is one worth pointing at.
    #[label("{next_label}")]
    pub next_span: Option<SourceSpan>,
    pub next_label: &'static str,
    pub separator: TokenKind,
    pub context: SeparatorContext,
    #[help]
    pub help: Option<&'static str>,
}

/// Where a separator was expected, used to describe a [`MissingSeparatorError`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeparatorContext {
    /// Between the items of an array expression.
    ArrayItems,
    /// Between the fields of a struct expression or struct declaration.
    StructFields,
    /// At the end of a declaration or statement.
    After(Terminated),
    /// Between the name and the type of a declaration or statement.
    BeforeType(Terminated),
}

impl SeparatorContext {
    fn next_label(&self) -> &'static str {
        match self {
            SeparatorContext::ArrayItems => "next item",
            SeparatorContext::StructFields => "next field",
            SeparatorContext::After(Terminated::Global(_) | Terminated::Const(_)) => {
                "next declaration"
            }
            SeparatorContext::After(_) => "next statement",
            SeparatorContext::BeforeType(_) => "type",
        }
    }

    /// Whether `kind` following the preceding item suggests the separator was left out, rather
    /// than something else being wrong.
    fn is_missing_before(&self, kind: TokenKind) -> bool {
        match self {
            SeparatorContext::ArrayItems => can_start_expr(kind),
            SeparatorContext::StructFields => {
                matches!(
                    kind,
                    TokenKind::Ident | TokenKind::Underscore | TokenKind::Hash
                )
            }
            SeparatorContext::After(_) => {
                matches!(kind, TokenKind::RightBrace | TokenKind::Eof)
                    || can_start_statement_or_item(kind)
            }
            SeparatorContext::BeforeType(_) => can_start_ty_ref(kind),
        }
    }
}

impl fmt::Display for SeparatorContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SeparatorContext::ArrayItems => f.write_str("between array items"),
            SeparatorContext::StructFields => f.write_str("between struct fields"),
            SeparatorContext::After(terminated) => write!(f, "after {terminated}"),
            SeparatorContext::BeforeType(terminated) => {
                write!(f, "before the type in {terminated}")
            }
        }
    }
}

/// A declaration or statement that must be terminated by a `;`.
///
/// Also names the declaration or statement that a type annotation belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Terminated {
    Global(Identifier),
    Const(Identifier),
    Let(Identifier),
    Return,
    Break,
    Yield,
    Continue,
    Expr,
}

impl fmt::Display for Terminated {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Terminated::Global(name) => write!(f, "`global {name}` declaration"),
            Terminated::Const(name) => write!(f, "`const {name}` declaration"),
            Terminated::Let(name) => write!(f, "`let {name}` statement"),
            Terminated::Return => f.write_str("`return` statement"),
            Terminated::Break => f.write_str("`break` statement"),
            Terminated::Yield => f.write_str("`yield` statement"),
            Terminated::Continue => f.write_str("`continue` statement"),
            Terminated::Expr => f.write_str("expression"),
        }
    }
}

/// A `global` or `const` declaration has no `: Type` after its name.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("Syntax error: missing type annotation in {declared}")]
#[diagnostic(help("`global` and `const` declarations must state their type, e.g. `: u8`"))]
pub struct MissingTypeAnnotationError {
    /// A zero-width span where the annotation should go, just after the declaration's name.
    #[label("add `: <type>` here")]
    pub span: SourceSpan,
    pub declared: Terminated,
}

/// Whether a token of `kind` can begin a type reference.
fn can_start_ty_ref(kind: TokenKind) -> bool {
    matches!(kind, TokenKind::Ident | TokenKind::LeftBracket)
}

/// Whether a token of `kind` can begin an expression.
fn can_start_expr(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Number
            | TokenKind::Ident
            | TokenKind::Underscore
            | TokenKind::LeftBracket
            | TokenKind::LeftParen
            | TokenKind::If
            | TokenKind::Loop
            | TokenKind::Bang
            | TokenKind::Minus
    )
}

/// Whether a token of `kind` can begin a statement or an item.
///
/// `{` is deliberately excluded: after an expression it more often means a missing `then` than
/// a missing `;` before a block.
fn can_start_statement_or_item(kind: TokenKind) -> bool {
    can_start_expr(kind)
        || matches!(
            kind,
            TokenKind::Let
                | TokenKind::Return
                | TokenKind::Break
                | TokenKind::Yield
                | TokenKind::Continue
                | TokenKind::Fn
                | TokenKind::Global
                | TokenKind::Const
                | TokenKind::Struct
                | TokenKind::Hash
        )
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("Syntax error: cannot chain comparisons")]
#[diagnostic()]
pub struct ChainedComparisonError {
    #[label("this comparison")]
    pub span: SourceSpan,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("Syntax error: callee is not a function")]
#[diagnostic()]
pub struct CallOnValueError {
    #[label("this call")]
    pub span: SourceSpan,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("Syntax error: array index requires array")]
#[diagnostic()]
pub struct ArrayIndexOnValueError {
    #[label("this index")]
    pub span: SourceSpan,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("Syntax error: field access requires struct")]
#[diagnostic()]
pub struct FieldAccessOnValueError {
    #[label("this field access")]
    pub span: SourceSpan,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("Syntax error: cannot assign to a value")]
#[diagnostic()]
pub struct AssignOnValueError {
    #[label("this value")]
    pub value_span: SourceSpan,
    #[label("this assignment")]
    pub op_span: SourceSpan,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic, Clone)]
#[error("Unnamed identifier not permitted here")]
#[diagnostic()]
pub struct UnnamedIdentifierError {
    #[label("this unnamed identifier")]
    pub span: SourceSpan,
}

pub struct Parser<'de> {
    lexer: lex::Lexer<'de>,
    features: PreviewFeatureSet,
    result_builder: MultiResultBuilder,
    /// the token kind of the last non-comment token consumed via [`Parser::consume_token`].
    ///
    /// This is used rather than `lexer.current()` to handle the case where comment tokens
    /// are present between non-comment tokens.
    last_token_kind: Option<TokenKind>,
    /// the span of the last non-comment token consumed via [`Parser::consume_token`].
    last_token_span: Option<SourceSpan>,
}

impl<'de> From<&'de str> for Parser<'de> {
    fn from(value: &'de str) -> Self {
        Parser {
            lexer: lex::Lexer::from(value),
            features: PreviewFeatureSet::default(),
            result_builder: MultiResultBuilder::default(),
            last_token_kind: None,
            last_token_span: None,
        }
    }
}

impl<'de> Parser<'de> {
    fn consume_token(&mut self) -> Result<Token<'de>, Error> {
        self.consume_comments()?;
        match self.lexer.next().transpose()? {
            Some(token) => {
                self.last_token_kind = Some(token.kind());
                self.last_token_span = Some(token.span());
                Ok(token)
            }
            None => unimplemented!("parsed beyond EOF"),
        }
    }

    fn peek_token(&mut self) -> Result<&Token<'de>, Error> {
        self.consume_comments()?;
        match self.lexer.peek() {
            Some(Ok(token)) => Ok(token),
            Some(Err(e)) => Err(Error::Lex(e.clone())),
            None => unimplemented!("parsed beyond EOF"),
        }
    }

    fn expect(&mut self, kind: TokenKind) -> Result<Token<'de>, Error> {
        let token = self.peek_token()?;
        if token.kind() == kind {
            self.consume_token()
        } else {
            Err(Error::Syntax(SyntaxError::new(token, kind)))
        }
    }

    /// Expects `separator` after the item or statement just parsed.
    ///
    /// When it's absent and the next token suggests it was simply left out (see
    /// [`SeparatorContext::is_missing_before`]), this reports a [`MissingSeparatorError`] pointing
    /// at where the separator belongs. Otherwise it falls back to a [`SyntaxError`] listing
    /// everything that would have been accepted in `expected`.
    fn expect_separator(
        &mut self,
        separator: TokenKind,
        context: SeparatorContext,
        expected: impl Into<Expected>,
        help: Option<&'static str>,
    ) -> Result<Token<'de>, Error> {
        let token = self.peek_token()?;
        let kind = token.kind();
        if kind == separator {
            return self.consume_token();
        }
        if !context.is_missing_before(kind) {
            return Err(Error::from(SyntaxError::new(token, expected)));
        }
        let next_span = match kind {
            TokenKind::RightBrace | TokenKind::Eof => None,
            _ => Some(token.span()),
        };
        Err(Error::from(MissingSeparatorError {
            span: self.span_after_last_token(),
            next_span,
            next_label: context.next_label(),
            separator,
            context,
            help,
        }))
    }

    /// Expects the `;` that ends a declaration or statement.
    fn expect_terminator(&mut self, terminated: Terminated) -> Result<Token<'de>, Error> {
        self.expect_separator(
            TokenKind::Semicolon,
            SeparatorContext::After(terminated),
            TokenKind::Semicolon,
            None,
        )
    }

    /// Expects the `:` that starts the type annotation of `declared`, right after its name.
    ///
    /// When the next token is one of `after_type`, i.e. what would follow the whole annotation,
    /// the annotation was left out entirely and this reports a [`MissingTypeAnnotationError`].
    /// When the next token looks like a type, only the `:` was left out and this reports a
    /// [`MissingSeparatorError`]. Otherwise it falls back to a [`SyntaxError`].
    fn expect_type_annotation(
        &mut self,
        declared: Terminated,
        after_type: &[TokenKind],
    ) -> Result<Token<'de>, Error> {
        if after_type.contains(&self.peek_token()?.kind()) {
            return Err(Error::from(MissingTypeAnnotationError {
                span: self.span_after_last_token(),
                declared,
            }));
        }
        self.expect_separator(
            TokenKind::Colon,
            SeparatorContext::BeforeType(declared),
            TokenKind::Colon,
            None,
        )
    }

    /// A zero-width span just after the last consumed token.
    fn span_after_last_token(&self) -> SourceSpan {
        let previous_end = self
            .last_token_span
            .map(|span| span.offset() + span.len())
            .unwrap_or_default();
        SourceSpan::new(previous_end.into(), 0)
    }

    fn is_proceeding_block(&self) -> bool {
        self.last_token_kind == Some(TokenKind::RightBrace)
    }
}

impl Parser<'_> {
    pub fn enable_feature(&mut self, feature: PreviewFeature) {
        self.features.enable(feature);
    }

    pub fn enable_features(&mut self, features: PreviewFeatureSet) {
        self.features.extend(features);
    }

    fn consume_comments(&mut self) -> Result<(), Error> {
        loop {
            break match self.lexer.peek() {
                Some(Ok(token)) if token.kind() == TokenKind::Comment => {
                    self.lexer.next().transpose()?;
                    continue;
                }
                Some(Err(e)) => Err(Error::Lex(e.clone())),
                _ => Ok(()),
            };
        }
    }

    fn chomp(&mut self) -> Result<(), Error> {
        self.consume_token()?;
        Ok(())
    }
}

impl Parser<'_> {
    #[instrument(skip_all)]
    pub fn parse(self) -> MultiResult<Root> {
        self.parse_as_root(Parser::parse_root)
    }

    #[instrument(skip_all)]
    pub fn parse_item_as_root(self) -> MultiResult<Item> {
        self.parse_as_root(Parser::parse_item)
    }

    #[instrument(skip_all)]
    pub fn parse_statement_as_root(self) -> MultiResult<Statement> {
        self.parse_as_root(Parser::parse_statement)
    }

    #[instrument(skip_all)]
    pub fn parse_expr_as_root(self) -> MultiResult<Expr> {
        self.parse_as_root(Parser::parse_expr)
    }

    fn parse_as_root<A>(mut self, f: impl FnOnce(&mut Self) -> Result<A, Error>) -> MultiResult<A> {
        let item = match f(&mut self) {
            Ok(item) => item,
            Err(error) => return self.result_builder.finish_with_error(error),
        };
        match self.expect(TokenKind::Eof) {
            Err(error) => self.result_builder.finish_with_error(error),
            Ok(_) => self.result_builder.finish(item),
        }
    }

    fn parse_root(&mut self) -> Result<Root, Error> {
        let mut items = Vec::new();
        while self.lexer.peek().is_some() && self.peek_token()?.kind() != TokenKind::Eof {
            match self.parse_item() {
                Ok(item) => {
                    items.push(item);
                }
                Err(Error::Lex(e)) => return Err(Error::Lex(e)),
                Err(e) => {
                    self.result_builder.add_error(e);
                    self.parse_item_error()?;
                }
            }
        }
        Ok(Root { items })
    }

    fn parse_attributes(&mut self) -> Result<Vec<AttributeDecl>, Error> {
        let mut attributes = Vec::new();
        while self.peek_token()?.kind() == TokenKind::Hash {
            attributes.push(self.parse_attribute()?);
        }
        Ok(attributes)
    }

    fn parse_attribute(&mut self) -> Result<AttributeDecl, Error> {
        let start_span = self.expect(TokenKind::Hash)?.span();
        self.expect(TokenKind::LeftBracket)?;
        let name = self.parse_identifier()?;
        if name.data.as_str() == "preview_flag_example" {
            self.features.check_enabled(
                PreviewFeature::ExamplePreview,
                name.span,
                &mut self.result_builder,
            );
        }
        let value = if self.peek_token()?.kind() == TokenKind::Equal {
            self.chomp()?;
            Some(self.parse_attribute_value()?)
        } else {
            None
        };
        let end_span = self.expect(TokenKind::RightBracket)?.span();
        Ok(AttributeDecl {
            name,
            span: span_between(start_span, end_span),
            value,
        })
    }

    fn parse_attribute_value(&mut self) -> Result<AttributeValue, Error> {
        let token = self.peek_token()?;
        Ok(match token.kind() {
            TokenKind::Ident | TokenKind::Underscore => {
                AttributeValue::from(self.parse_identifier()?)
            }
            TokenKind::Number => {
                let token = self.consume_token()?;
                let data = AttributeValueData::from(IntegerLiteral::from(token.try_parse_u32()?));
                AttributeValue {
                    span: token.span(),
                    data,
                }
            }
            _ => {
                return Err(Error::from(SyntaxError::new(
                    token,
                    [TokenKind::Ident, TokenKind::Number],
                )));
            }
        })
    }

    fn parse_item(&mut self) -> Result<Item, Error> {
        let attributes = self.parse_attributes()?;
        let token = self.peek_token()?;
        let data = match token.kind() {
            TokenKind::Fn => ItemData::from(self.parse_fn_decl()?),
            TokenKind::Global => ItemData::from(self.parse_global_decl()?),
            TokenKind::Const => ItemData::from(self.parse_const_decl()?),
            TokenKind::Struct => ItemData::from(self.parse_struct_decl()?),
            _ => {
                return Err(Error::from(SyntaxError::new(
                    token,
                    [
                        TokenKind::Fn,
                        TokenKind::Global,
                        TokenKind::Const,
                        TokenKind::Struct,
                    ],
                )));
            }
        };
        Ok(Item { attributes, data })
    }

    fn parse_item_error(&mut self) -> Result<(), Error> {
        while self.lexer.peek().is_some()
            && !matches!(
                self.peek_token()?.kind(),
                TokenKind::Fn
                    | TokenKind::Global
                    | TokenKind::Const
                    | TokenKind::Struct
                    | TokenKind::Eof
            )
        {
            self.chomp()?;
        }
        Ok(())
    }

    fn parse_identifier(&mut self) -> Result<SpannedIdentifier, Error> {
        let identifier = self.parse_identifier_or_unnamed()?;
        if identifier.data == Identifier::unnamed() {
            self.result_builder.add_error(UnnamedIdentifierError {
                span: identifier.span,
            });
        }
        Ok(identifier)
    }

    fn parse_identifier_or_unnamed(&mut self) -> Result<SpannedIdentifier, Error> {
        if self.peek_token()?.kind() == TokenKind::Underscore {
            let token = self.consume_token()?;
            Ok(SpannedIdentifier::new(Identifier::unnamed(), token.span()))
        } else {
            let token = self.expect(TokenKind::Ident)?;
            if let Err(error) = token.check_unused_reserved() {
                self.result_builder.add_error(error);
            }
            Ok(SpannedIdentifier::new(token.text(), token.span()))
        }
    }

    fn parse_fn_decl(&mut self) -> Result<FnDecl, Error> {
        self.expect(TokenKind::Fn)?;
        let signature = self.parse_fn_sig()?;
        let body = self.parse_body()?;

        Ok(FnDecl { signature, body })
    }

    fn parse_fn_sig(&mut self) -> Result<FnSig, Error> {
        let name = self.parse_identifier()?;
        // no parameters supported atm
        self.expect(TokenKind::LeftParen)?;
        self.expect(TokenKind::RightParen)?;
        let ty = if self.peek_token()?.kind() == TokenKind::MinusGreaterThan {
            self.expect(TokenKind::MinusGreaterThan)?;
            Some(self.parse_ty_ref()?)
        } else {
            None
        };

        Ok(FnSig { name, ty })
    }

    fn parse_global_decl(&mut self) -> Result<GlobalDecl, Error> {
        self.expect(TokenKind::Global)?;
        let name = self.parse_identifier()?;
        self.expect_type_annotation(
            Terminated::Global(name.data.clone()),
            &[TokenKind::Semicolon],
        )?;
        let ty = self.parse_ty_ref()?;
        self.expect_terminator(Terminated::Global(name.data.clone()))?;

        Ok(GlobalDecl { name, ty })
    }

    fn parse_const_decl(&mut self) -> Result<ConstDecl, Error> {
        self.expect(TokenKind::Const)?;
        let name = self.parse_identifier()?;
        self.expect_type_annotation(
            Terminated::Const(name.data.clone()),
            &[TokenKind::Equal, TokenKind::Semicolon],
        )?;
        let ty = self.parse_ty_ref()?;
        self.expect(TokenKind::Equal)?;
        let value = self.parse_expr_or_block()?;
        self.expect_terminator(Terminated::Const(name.data.clone()))?;

        Ok(ConstDecl { name, ty, value })
    }

    fn parse_struct_decl(&mut self) -> Result<StructDecl, Error> {
        self.expect(TokenKind::Struct)?;
        let name = self.parse_identifier()?;

        self.expect(TokenKind::LeftBrace)?;
        let mut fields = Vec::new();
        while self.peek_token()?.kind() != TokenKind::RightBrace {
            fields.push(self.parse_field_decl()?);
            if self.peek_token()?.kind() != TokenKind::RightBrace {
                self.expect_separator(
                    TokenKind::Comma,
                    SeparatorContext::StructFields,
                    [TokenKind::Comma, TokenKind::RightBrace],
                    None,
                )?;
            }
        }
        self.expect(TokenKind::RightBrace)?;

        Ok(StructDecl { name, fields })
    }

    fn parse_field_decl(&mut self) -> Result<FieldDecl, Error> {
        let attributes = self.parse_attributes()?;
        let name = self.parse_identifier_or_unnamed()?;
        self.expect(TokenKind::Colon)?;
        let ty = self.parse_ty_ref()?;

        Ok(FieldDecl {
            attributes,
            name,
            ty,
        })
    }

    fn parse_ty_ref(&mut self) -> Result<TyRef, Error> {
        Ok(if self.peek_token()?.kind() == TokenKind::LeftBracket {
            // fixed size array
            let start_span = self.peek_token()?.span();
            self.expect(TokenKind::LeftBracket)?;
            let name = self.parse_ty_ref()?;
            self.expect(TokenKind::Semicolon)?;
            let count = self.expect(TokenKind::Number)?.try_parse_non_zero_u16()?;
            self.expect(TokenKind::RightBracket)?;
            let end_span = self.lexer.current().unwrap().span();
            let span = span_between(start_span, end_span);

            TyRef::array(span, name, count)
        } else {
            // regular type is equivalent to an array of size 1
            let name = self.parse_identifier()?;
            TyRef::new(name)
        })
    }

    fn parse_body(&mut self) -> Result<Block, Error> {
        let start_span = self.expect(TokenKind::LeftBrace)?.span();

        let mut statements = Vec::new();
        while !matches!(
            self.peek_token()?.kind(),
            TokenKind::RightBrace | TokenKind::Eof
        ) {
            statements.push(self.parse_statement()?);
        }

        let end_span = self.expect(TokenKind::RightBrace)?.span();

        Ok(Block {
            span: span_between(start_span, end_span),
            statements,
        })
    }

    fn parse_statement(&mut self) -> Result<Statement, Error> {
        let token = self.peek_token()?;
        let start_span = token.span();
        let data = match token.kind() {
            TokenKind::Let => StatementData::from(self.parse_local_decl()?),
            TokenKind::Return => StatementData::from(self.parse_return()?),
            TokenKind::Break => StatementData::from(self.parse_break()?),
            TokenKind::Yield => StatementData::from(self.parse_yield()?),
            TokenKind::Continue => StatementData::from(self.parse_continue()?),
            _ => StatementData::from(self.parse_expr_smt()?),
        };
        let end_span = self.lexer.current().unwrap().span();
        Ok(Statement {
            span: span_between(start_span, end_span),
            data,
        })
    }

    fn parse_local_decl(&mut self) -> Result<LetSmt, Error> {
        self.expect(TokenKind::Let)?;
        let name = self.parse_identifier()?;
        let kind = self.peek_token()?.kind();
        let ty = if kind == TokenKind::Colon || can_start_ty_ref(kind) {
            // the type is optional, so there is no annotation to be missing, only its `:`
            self.expect_type_annotation(Terminated::Let(name.data.clone()), &[])?;
            Some(self.parse_ty_ref()?)
        } else {
            None
        };
        self.expect(TokenKind::Equal)?;
        let value = self.parse_expr_or_block()?;
        self.expect_terminator(Terminated::Let(name.data.clone()))?;

        Ok(LetSmt { name, ty, value })
    }

    fn parse_return(&mut self) -> Result<ReturnSmt, Error> {
        self.expect(TokenKind::Return)?;
        let value = if self.peek_token()?.kind() == TokenKind::Semicolon {
            self.expect(TokenKind::Semicolon)?;
            None
        } else {
            let value = self.parse_expr_or_block()?;
            self.expect_terminator(Terminated::Return)?;
            Some(value)
        };
        Ok(ReturnSmt { value })
    }

    fn parse_break(&mut self) -> Result<BreakSmt, Error> {
        self.expect(TokenKind::Break)?;
        let value = if self.peek_token()?.kind() == TokenKind::Semicolon {
            self.expect(TokenKind::Semicolon)?;
            None
        } else {
            let value = self.parse_expr_or_block()?;
            self.expect_terminator(Terminated::Break)?;
            Some(value)
        };
        Ok(BreakSmt { value })
    }

    fn parse_yield(&mut self) -> Result<YieldSmt, Error> {
        self.expect(TokenKind::Yield)?;
        let value = self.parse_expr_or_block()?;
        self.expect_terminator(Terminated::Yield)?;
        Ok(YieldSmt { value })
    }

    fn parse_continue(&mut self) -> Result<ContinueSmt, Error> {
        self.expect(TokenKind::Continue)?;
        self.expect_terminator(Terminated::Continue)?;
        Ok(ContinueSmt)
    }

    fn parse_expr_smt(&mut self) -> Result<ExprSmt, Error> {
        let value = self.parse_expr_or_block()?;
        if !self.is_proceeding_block() {
            self.expect_terminator(Terminated::Expr)?;
        }
        Ok(ExprSmt { value })
    }

    fn parse_expr_or_block(&mut self) -> Result<Expr, Error> {
        let peeked_token = self.peek_token()?;
        Ok(if peeked_token.kind() == TokenKind::LeftBrace {
            let start_span = peeked_token.span();
            let block = self.parse_body()?;
            let end_span = self.lexer.current().unwrap().span();
            Expr {
                span: span_between(start_span, end_span),
                data: ExprData::Block(block),
            }
        } else {
            self.parse_expr_above(0)?
        })
    }

    fn parse_expr(&mut self) -> Result<Expr, Error> {
        self.parse_expr_above(0)
    }

    fn parse_expr_above(&mut self, min_bp: u8) -> Result<Expr, Error> {
        let lhs_token = self.peek_token()?;
        let lhs_start_span = lhs_token.span();
        let mut lhs = {
            let data = match lhs_token.kind() {
                // atoms
                TokenKind::Number => {
                    let lhs_token = self.consume_token()?;
                    ExprData::from(IntegerLiteral::from(lhs_token.try_parse_u32()?))
                }
                TokenKind::Ident | TokenKind::Underscore => {
                    let name = self.parse_identifier()?;
                    if self.peek_token()?.kind() == TokenKind::LeftBrace {
                        self.expect(TokenKind::LeftBrace)?;
                        let mut fields = Vec::new();
                        while self.peek_token()?.kind() != TokenKind::RightBrace {
                            let name = self.parse_identifier()?;
                            self.expect(TokenKind::Colon)?;
                            let value = self.parse_expr_above(0)?;
                            fields.push(StructExprField { name, value });
                            if self.peek_token()?.kind() != TokenKind::RightBrace {
                                self.expect_separator(
                                    TokenKind::Comma,
                                    SeparatorContext::StructFields,
                                    [TokenKind::Comma, TokenKind::RightBrace],
                                    None,
                                )?;
                            }
                        }
                        self.expect(TokenKind::RightBrace)?;
                        ExprData::from(StructExpr { name, fields })
                    } else {
                        ExprData::from(Place {
                            span: name.span,
                            name,
                            projections: Vec::new(),
                        })
                    }
                }
                TokenKind::LeftBracket => {
                    self.chomp()?;
                    let first = self.parse_expr_above(0)?;
                    if self.peek_token()?.kind() == TokenKind::Semicolon {
                        self.expect(TokenKind::Semicolon)?;
                        let count = self.expect(TokenKind::Number)?.try_parse_non_zero_u16()?;
                        self.expect(TokenKind::RightBracket)?;
                        ExprData::from(RepeatExpr {
                            item: Box::new(first),
                            count,
                        })
                    } else {
                        let mut items = vec![first];
                        while self.peek_token()?.kind() != TokenKind::RightBracket {
                            if items.len() == 1 {
                                // a repeat expression is still possible after the first item
                                let help = (self.peek_token()?.kind() == TokenKind::Number)
                                    .then_some("to repeat an item, use `;` instead: `[0; 4]`");
                                self.expect_separator(
                                    TokenKind::Comma,
                                    SeparatorContext::ArrayItems,
                                    [
                                        TokenKind::Comma,
                                        TokenKind::Semicolon,
                                        TokenKind::RightBracket,
                                    ],
                                    help,
                                )?;
                            } else {
                                self.expect_separator(
                                    TokenKind::Comma,
                                    SeparatorContext::ArrayItems,
                                    [TokenKind::Comma, TokenKind::RightBracket],
                                    None,
                                )?;
                            }
                            if self.peek_token()?.kind() == TokenKind::RightBracket {
                                break;
                            }
                            let item = self.parse_expr_above(0)?;
                            items.push(item);
                        }
                        self.expect(TokenKind::RightBracket)?;
                        ExprData::from(ArrayExpr { items })
                    }
                }
                // group
                TokenKind::LeftParen => {
                    self.chomp()?;
                    let expr = self.parse_expr_above(0)?;
                    self.expect(TokenKind::RightParen)?;
                    ExprData::from(Box::new(expr))
                }
                TokenKind::If => {
                    self.chomp()?;
                    let condition = Box::new(self.parse_expr()?);
                    self.expect(TokenKind::Then)?;
                    let true_block = Box::new(self.parse_expr_or_block()?);
                    let false_block = if self.peek_token()?.kind() == TokenKind::Else {
                        self.chomp()?;
                        Some(Box::new(self.parse_expr_or_block()?))
                    } else {
                        None
                    };

                    ExprData::from(If {
                        condition,
                        true_block,
                        false_block,
                    })
                }
                TokenKind::Loop => {
                    self.chomp()?;
                    let body = self.parse_body()?;
                    ExprData::from(Loop { body })
                }
                // unary prefix expression
                kind @ (TokenKind::Bang | TokenKind::Minus) => {
                    self.chomp()?;
                    let kind = match kind {
                        TokenKind::Bang => UnaryOpKind::Not,
                        TokenKind::Minus => UnaryOpKind::Neg,
                        _ => unreachable!(
                            "{kind:?} is matched as a unary prefix expression but has no op kind"
                        ),
                    };
                    let bp = prefix_binding_power(OpKind::Unary(kind));
                    let expr = Box::new(self.parse_expr_above(bp)?);
                    ExprData::from(UnaryOp { kind, expr })
                }
                _ => {
                    return Err(Error::from(SyntaxError::new(lhs_token, "an expression")));
                }
            };
            let end_span = self.lexer.current().unwrap().span();
            Expr {
                span: span_between(lhs_start_span, end_span),
                data,
            }
        };

        loop {
            let op_token = self.peek_token()?;
            let kind = match op_token.kind() {
                TokenKind::Eof
                | TokenKind::RightParen
                | TokenKind::Semicolon
                | TokenKind::Comma
                | TokenKind::RightBrace
                | TokenKind::RightBracket
                | TokenKind::Then
                | TokenKind::Else => break,
                // postfix
                TokenKind::LeftParen => OpKind::FunctionCall,
                TokenKind::LeftBracket => OpKind::ArrayIndex,
                TokenKind::Dot => OpKind::FieldAccess,
                TokenKind::LessThanLessThan => OpKind::Unary(UnaryOpKind::ShiftLeft),
                TokenKind::LessThanPercent => OpKind::Unary(UnaryOpKind::RotateLeft),
                TokenKind::GreaterThanGreaterThan => OpKind::Unary(UnaryOpKind::ShiftRight),
                TokenKind::GreaterThanPercent => OpKind::Unary(UnaryOpKind::RotateRight),
                TokenKind::As => OpKind::BitCast,
                // infix
                TokenKind::Equal => OpKind::Assign(AssignOpKind::Assign),
                TokenKind::EqualEqual => OpKind::Comparison(ComparisonOpKind::Eq),
                TokenKind::BangEqual => OpKind::Comparison(ComparisonOpKind::Ne),
                TokenKind::Ampersand => OpKind::Arithmetic(ArithmeticOpKind::BitAnd),
                TokenKind::AmpersandAmpersand => OpKind::Logical(LogicalOpKind::And),
                TokenKind::AmpersandEqual => OpKind::Assign(AssignOpKind::BitAndAssign),
                TokenKind::Pipe => OpKind::Arithmetic(ArithmeticOpKind::BitOr),
                TokenKind::PipePipe => OpKind::Logical(LogicalOpKind::Or),
                TokenKind::PipeEqual => OpKind::Assign(AssignOpKind::BitOrAssign),
                TokenKind::Plus => OpKind::Arithmetic(ArithmeticOpKind::Add),
                TokenKind::PlusEqual => OpKind::Assign(AssignOpKind::AddAssign),
                TokenKind::Minus => OpKind::Arithmetic(ArithmeticOpKind::Sub),
                TokenKind::MinusEqual => OpKind::Assign(AssignOpKind::SubAssign),
                TokenKind::Carrot => OpKind::Arithmetic(ArithmeticOpKind::BitXor),
                TokenKind::CarrotEqual => OpKind::Assign(AssignOpKind::BitXorAssign),
                TokenKind::LessThan => OpKind::Comparison(ComparisonOpKind::Lt),
                TokenKind::LessThanEqual => OpKind::Comparison(ComparisonOpKind::Le),
                TokenKind::GreaterThan => OpKind::Comparison(ComparisonOpKind::Gt),
                TokenKind::GreaterThanEqual => OpKind::Comparison(ComparisonOpKind::Ge),
                // Anything that could start another expression, statement or item ends this
                // expression, leaving the enclosing construct to report the missing separator.
                kind if can_start_statement_or_item(kind) => break,
                _ => {
                    let error = SyntaxError::new(op_token, "an operator or end of expression");
                    if self.is_proceeding_block() {
                        break;
                    }
                    return Err(Error::from(error));
                }
            };

            lhs = {
                let data = match postfix_binding_power(kind) {
                    Some(bp) => {
                        // postfix handling
                        if bp < min_bp {
                            break;
                        }
                        let op_token = self.consume_token()?;
                        match kind {
                            OpKind::FunctionCall => {
                                let end_span = self.expect(TokenKind::RightParen)?.span();
                                let span = span_between(op_token.span(), end_span);
                                match lhs.data {
                                    ExprData::Place(callee) => ExprData::from(Call { callee }),
                                    _ => return Err(Error::from(CallOnValueError { span })),
                                }
                            }
                            OpKind::ArrayIndex => {
                                let index = Box::new(self.parse_expr()?);
                                let end_span = self.expect(TokenKind::RightBracket)?.span();
                                let span = span_between(op_token.span(), end_span);
                                match lhs.data {
                                    ExprData::Place(mut place) => {
                                        place.span = span_between(place.span, end_span);
                                        place.projections.push(PlaceProjection::Index(index));
                                        ExprData::from(place)
                                    }
                                    _ => return Err(Error::from(ArrayIndexOnValueError { span })),
                                }
                            }
                            OpKind::FieldAccess => {
                                let name = self.parse_identifier()?;
                                let span = span_between(op_token.span(), name.span);
                                match lhs.data {
                                    ExprData::Place(mut place) => {
                                        place.span = span_between(place.span, span);
                                        place.projections.push(PlaceProjection::Field(name));
                                        ExprData::from(place)
                                    }
                                    _ => return Err(Error::from(FieldAccessOnValueError { span })),
                                }
                            }
                            OpKind::Unary(kind) => ExprData::from(UnaryOp {
                                kind,
                                expr: Box::new(lhs),
                            }),
                            OpKind::BitCast => {
                                let ty = self.parse_ty_ref()?;
                                ExprData::from(BitCast {
                                    expr: Box::new(lhs),
                                    ty,
                                })
                            }
                            _ => unimplemented!("{kind:?} is not a postfix op"),
                        }
                    }
                    None => {
                        // infix handling
                        let (lhs_bp, rhs_bp) = infix_binding_power(kind);
                        if lhs_bp < min_bp {
                            break;
                        }
                        let op_token = self.consume_token()?;

                        let rhs = self.parse_expr_above(rhs_bp)?;

                        match kind {
                            OpKind::Logical(kind) => ExprData::from(LogicalOp {
                                kind,
                                left: Box::new(lhs),
                                right: Box::new(rhs),
                            }),
                            OpKind::Arithmetic(kind) => ExprData::from(ArithmeticOp {
                                kind,
                                left: Box::new(lhs),
                                right: Box::new(rhs),
                            }),
                            OpKind::Comparison(kind) => {
                                if matches!(lhs.data, ExprData::Comparison(_)) {
                                    return Err(Error::ChainedComparison(ChainedComparisonError {
                                        span: op_token.span(),
                                    }));
                                }

                                ExprData::from(ComparisonOp {
                                    kind,
                                    left: Box::new(lhs),
                                    right: Box::new(rhs),
                                })
                            }
                            OpKind::Assign(kind) => {
                                if let ExprData::Place(place) = lhs.data {
                                    ExprData::from(AssignOp {
                                        kind,
                                        left: Box::new(place),
                                        right: Box::new(rhs),
                                    })
                                } else {
                                    return Err(Error::AssignOnValue(AssignOnValueError {
                                        value_span: lhs.span,
                                        op_span: op_token.span(),
                                    }));
                                }
                            }
                            _ => unimplemented!("{kind:?} is not a binary infix op"),
                        }
                    }
                };
                let end_span = self.lexer.current().unwrap().span();
                Expr {
                    span: span_between(lhs_start_span, end_span),
                    data,
                }
            }
        }
        Ok(lhs)
    }
}

fn prefix_binding_power(op: OpKind) -> u8 {
    match op {
        OpKind::Unary(UnaryOpKind::Not | UnaryOpKind::Neg) => 19,
        _ => unimplemented!("{op:?} is not a prefix op"),
    }
}

fn postfix_binding_power(op: OpKind) -> Option<u8> {
    Some(match op {
        OpKind::Unary(
            UnaryOpKind::ShiftLeft
            | UnaryOpKind::ShiftRight
            | UnaryOpKind::RotateLeft
            | UnaryOpKind::RotateRight,
        ) => 15,
        OpKind::BitCast => 18,
        OpKind::FunctionCall => 20,
        OpKind::FieldAccess | OpKind::ArrayIndex => 21,
        _ => return None,
    })
}

fn infix_binding_power(op: OpKind) -> (u8, u8) {
    match op {
        OpKind::Assign(
            AssignOpKind::Assign
            | AssignOpKind::AddAssign
            | AssignOpKind::BitXorAssign
            | AssignOpKind::SubAssign
            | AssignOpKind::BitOrAssign
            | AssignOpKind::BitAndAssign,
        ) => (2, 1),
        OpKind::Logical(LogicalOpKind::Or) => (3, 4),
        OpKind::Logical(LogicalOpKind::And) => (5, 6),
        OpKind::Comparison(
            ComparisonOpKind::Eq
            | ComparisonOpKind::Ne
            | ComparisonOpKind::Lt
            | ComparisonOpKind::Gt
            | ComparisonOpKind::Le
            | ComparisonOpKind::Ge,
        ) => (7, 8),
        OpKind::Arithmetic(ArithmeticOpKind::BitOr) => (9, 10),
        OpKind::Arithmetic(ArithmeticOpKind::BitXor) => (11, 12),
        OpKind::Arithmetic(ArithmeticOpKind::BitAnd) => (13, 14),
        OpKind::Arithmetic(ArithmeticOpKind::Add | ArithmeticOpKind::Sub) => (16, 17),
        _ => unimplemented!("{op:?} is not an infix op"),
    }
}
