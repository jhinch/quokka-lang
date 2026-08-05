use crate::ast::*;
use crate::feature::{PreviewFeature, PreviewFeatureError, PreviewFeatureSet};
use crate::lex::{self, Token, TokenKind};
use crate::multi::{MultiResult, MultiResultBuilder};
use crate::source::span::span_between;
use miette::SourceSpan;

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
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("Syntax error: found {unexpected:?} when expecting {expected:?}")]
#[diagnostic()]
pub struct SyntaxError {
    #[label("this token")]
    pub span: SourceSpan,
    pub unexpected: TokenKind,
    pub expected: Vec<TokenKind>,
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

pub struct Parser<'de> {
    lexer: lex::Lexer<'de>,
    features: PreviewFeatureSet,
    result_builder: MultiResultBuilder,
}

impl<'de> From<&'de str> for Parser<'de> {
    fn from(value: &'de str) -> Self {
        Parser {
            lexer: lex::Lexer::from(value),
            features: PreviewFeatureSet::default(),
            result_builder: MultiResultBuilder::default(),
        }
    }
}

impl<'de> Parser<'de> {
    fn consume_token(&mut self) -> Result<Token<'de>, Error> {
        match self.lexer.next().transpose()? {
            Some(token) => Ok(token),
            None => unimplemented!("parsed beyond EOF"),
        }
    }

    fn peek_token(&mut self) -> Result<&Token<'de>, Error> {
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
            Err(Error::Syntax(SyntaxError {
                span: token.span(),
                unexpected: token.kind(),
                expected: vec![kind],
            }))
        }
    }

    fn is_proceeding_block(&self) -> bool {
        self.lexer
            .current()
            .is_some_and(|token| token.kind() == TokenKind::RightBrace)
    }
}

impl Parser<'_> {
    pub fn enable_feature(&mut self, feature: PreviewFeature) {
        self.features.enable(feature);
    }

    pub fn enable_features(&mut self, features: PreviewFeatureSet) {
        self.features.extend(features);
    }

    fn chomp(&mut self) -> Result<(), Error> {
        self.consume_token()?;
        Ok(())
    }
}

impl Parser<'_> {
    pub fn parse(self) -> MultiResult<Root> {
        self.parse_as_root(Parser::parse_root)
    }

    pub fn parse_item_as_root(self) -> MultiResult<Item> {
        self.parse_as_root(Parser::parse_item)
    }

    pub fn parse_statement_as_root(self) -> MultiResult<Statement> {
        self.parse_as_root(Parser::parse_statement)
    }

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
        if name.data.value == "preview_flag_example" {
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
            TokenKind::Ident => AttributeValue::from(self.parse_identifier()?),
            TokenKind::Number => {
                let token = self.consume_token()?;
                let data = AttributeValueData::from(IntegerLiteral::from(token.try_parse_u32()?));
                AttributeValue {
                    span: token.span(),
                    data,
                }
            }
            unexpected => {
                return Err(Error::from(SyntaxError {
                    span: token.span(),
                    unexpected,
                    expected: vec![TokenKind::Ident, TokenKind::Number],
                }));
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
            unexpected => {
                return Err(Error::from(SyntaxError {
                    span: token.span(),
                    unexpected,
                    expected: vec![
                        TokenKind::Fn,
                        TokenKind::Global,
                        TokenKind::Const,
                        TokenKind::Struct,
                    ],
                }));
            }
        };
        Ok(Item { attributes, data })
    }

    fn parse_item_error(&mut self) -> Result<(), Error> {
        while self.lexer.peek().is_some()
            && !matches!(
                self.peek_token()?.kind(),
                TokenKind::Fn | TokenKind::Global | TokenKind::Struct | TokenKind::Eof
            )
        {
            self.chomp()?;
        }
        Ok(())
    }

    fn parse_identifier(&mut self) -> Result<SpannedIdentifier, Error> {
        let token = self.expect(TokenKind::Ident)?;
        Ok(SpannedIdentifier::new(token.text(), token.span()))
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
        self.expect(TokenKind::Colon)?;
        let ty = self.parse_ty_ref()?;
        self.expect(TokenKind::Semicolon)?;

        Ok(GlobalDecl { name, ty })
    }

    fn parse_const_decl(&mut self) -> Result<ConstDecl, Error> {
        self.expect(TokenKind::Const)?;
        let name = self.parse_identifier()?;
        self.expect(TokenKind::Colon)?;
        let ty = self.parse_ty_ref()?;
        self.expect(TokenKind::Equal)?;
        let value = self.parse_expr_or_block()?;
        self.expect(TokenKind::Semicolon)?;

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
                self.expect(TokenKind::Comma)?;
            }
        }
        self.expect(TokenKind::RightBrace)?;

        Ok(StructDecl { name, fields })
    }

    fn parse_field_decl(&mut self) -> Result<FieldDecl, Error> {
        let attributes = self.parse_attributes()?;
        let name = self.parse_identifier()?;
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
            self.features
                .check_enabled(PreviewFeature::Array, span, &mut self.result_builder);

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
        let ty = if self.peek_token()?.kind() == TokenKind::Colon {
            self.expect(TokenKind::Colon)?;
            Some(self.parse_ty_ref()?)
        } else {
            None
        };
        self.expect(TokenKind::Equal)?;
        let value = self.parse_expr_or_block()?;
        self.expect(TokenKind::Semicolon)?;

        Ok(LetSmt { name, ty, value })
    }

    fn parse_return(&mut self) -> Result<ReturnSmt, Error> {
        self.expect(TokenKind::Return)?;
        let value = if self.peek_token()?.kind() == TokenKind::Semicolon {
            self.expect(TokenKind::Semicolon)?;
            None
        } else {
            let value = self.parse_expr_or_block()?;
            self.expect(TokenKind::Semicolon)?;
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
            self.expect(TokenKind::Semicolon)?;
            Some(value)
        };
        Ok(BreakSmt { value })
    }

    fn parse_yield(&mut self) -> Result<YieldSmt, Error> {
        self.expect(TokenKind::Yield)?;
        let value = self.parse_expr_or_block()?;
        self.expect(TokenKind::Semicolon)?;
        Ok(YieldSmt { value })
    }

    fn parse_continue(&mut self) -> Result<ContinueSmt, Error> {
        self.expect(TokenKind::Continue)?;
        self.expect(TokenKind::Semicolon)?;
        Ok(ContinueSmt)
    }

    fn parse_expr_smt(&mut self) -> Result<ExprSmt, Error> {
        let value = self.parse_expr_or_block()?;
        if !self.is_proceeding_block() {
            self.expect(TokenKind::Semicolon)?;
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
        let lhs_token = self.consume_token()?;
        let lhs_start_span = lhs_token.span();
        let mut lhs = {
            let data = match lhs_token.kind() {
                // atoms
                TokenKind::Number => {
                    ExprData::from(IntegerLiteral::from(lhs_token.try_parse_u32()?))
                }
                TokenKind::Ident => {
                    let span = lhs_token.span();
                    let name = SpannedIdentifier::new(lhs_token.text(), span);
                    ExprData::from(Place {
                        name,
                        span,
                        projections: Vec::new(),
                    })
                }
                // group
                TokenKind::LeftParen => {
                    let expr = self.parse_expr_above(0)?;
                    self.expect(TokenKind::RightParen)?;
                    ExprData::from(Box::new(expr))
                }
                TokenKind::If => {
                    let condition = Box::new(self.parse_expr()?);
                    self.expect(TokenKind::Then)?;
                    let true_block = Box::new(self.parse_expr_or_block()?);
                    let false_block = if self.peek_token()?.kind() == TokenKind::Else {
                        self.chomp()?;
                        Some(Box::new(self.parse_expr_or_block()?))
                    } else {
                        None
                    };

                    self.features.check_enabled(
                        PreviewFeature::IfElse,
                        lhs_token.span(),
                        &mut self.result_builder,
                    );

                    ExprData::from(If {
                        condition,
                        true_block,
                        false_block,
                    })
                }
                TokenKind::Loop => {
                    let body = self.parse_body()?;
                    self.features.check_enabled(
                        PreviewFeature::Loop,
                        lhs_token.span(),
                        &mut self.result_builder,
                    );
                    ExprData::from(Loop { body })
                }
                TokenKind::Asm => {
                    self.features
                        .require_enabled(PreviewFeature::InlineAsm, lhs_token.span())?;
                    todo!("parse inline-asm");
                }
                // unary prefix expression
                kind @ (TokenKind::Bang | TokenKind::Minus) => {
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
                unexpected => {
                    return Err(Error::from(SyntaxError {
                        span: lhs_token.span(),
                        unexpected,
                        expected: vec![
                            TokenKind::Number,
                            TokenKind::Ident,
                            TokenKind::LeftParen,
                            TokenKind::Bang,
                            TokenKind::Minus,
                            TokenKind::If,
                            TokenKind::Loop,
                        ],
                    }));
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
                unexpected => {
                    let span = op_token.span();
                    if self.is_proceeding_block() {
                        break;
                    }
                    return Err(Error::from(SyntaxError {
                        span,
                        unexpected,
                        expected: vec![
                            TokenKind::Equal,
                            TokenKind::EqualEqual,
                            TokenKind::BangEqual,
                            TokenKind::Ampersand,
                            TokenKind::AmpersandAmpersand,
                            TokenKind::AmpersandEqual,
                            TokenKind::Pipe,
                            TokenKind::PipePipe,
                            TokenKind::PipeEqual,
                            TokenKind::Plus,
                            TokenKind::PlusEqual,
                            TokenKind::Minus,
                            TokenKind::MinusEqual,
                            TokenKind::Carrot,
                            TokenKind::CarrotEqual,
                            TokenKind::LessThan,
                            TokenKind::LessThanEqual,
                            TokenKind::LessThanLessThan,
                            TokenKind::LessThanPercent,
                            TokenKind::GreaterThan,
                            TokenKind::GreaterThanEqual,
                            TokenKind::GreaterThanGreaterThan,
                            TokenKind::GreaterThanPercent,
                        ],
                    }));
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
                                self.features.check_enabled(
                                    PreviewFeature::FunctionCall,
                                    span,
                                    &mut self.result_builder,
                                );
                                match lhs.data {
                                    ExprData::Place(callee) => ExprData::from(Call { callee }),
                                    _ => return Err(Error::from(CallOnValueError { span })),
                                }
                            }
                            OpKind::ArrayIndex => {
                                let index = Box::new(self.parse_expr()?);
                                let end_span = self.expect(TokenKind::RightBracket)?.span();
                                let span = span_between(op_token.span(), end_span);
                                self.features.check_enabled(
                                    PreviewFeature::Array,
                                    span,
                                    &mut self.result_builder,
                                );
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
                                let token = self.expect(TokenKind::Ident)?;
                                let name = SpannedIdentifier::new(token.text(), token.span());
                                let span = span_between(op_token.span(), token.span());
                                self.features.check_enabled(
                                    PreviewFeature::FieldAccess,
                                    span,
                                    &mut self.result_builder,
                                );
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
        OpKind::Unary(UnaryOpKind::Not | UnaryOpKind::Neg) => 18,
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
        OpKind::FunctionCall => 19,
        OpKind::FieldAccess | OpKind::ArrayIndex => 20,
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
