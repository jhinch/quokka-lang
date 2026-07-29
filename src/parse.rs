use crate::ast::*;
use crate::feature::{PreviewFeature, PreviewFeatureError, PreviewFeatureSet};
use crate::lex::{self, Token, TokenKind};
use crate::multi::{MultiResult, MultiResultBuilder};
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
    pub fn parse(&mut self) -> MultiResult<Root> {
        match self.parse_root() {
            Ok(root) => self.result_builder.take().finish(root),
            Err(error) => self.result_builder.take().finish_with_error(error),
        }
    }

    pub fn parse_item_as_root(&mut self) -> MultiResult<Item> {
        self.parse_as_root(Parser::parse_item)
    }

    pub fn parse_statement_as_root(&mut self) -> MultiResult<Statement> {
        self.parse_as_root(Parser::parse_statement)
    }

    pub fn parse_expr_as_root(&mut self) -> MultiResult<Expr> {
        self.parse_as_root(Parser::parse_expr)
    }

    fn parse_as_root<A>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<A, Error>,
    ) -> MultiResult<A> {
        let item = match f(self) {
            Ok(item) => item,
            Err(error) => return self.result_builder.take().finish_with_error(error),
        };
        match self.expect(TokenKind::Eof) {
            Err(error) => self.result_builder.take().finish_with_error(error),
            Ok(_) => self.result_builder.take().finish(item),
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
        self.expect(TokenKind::Eof)?;
        Ok(Root { items })
    }

    fn parse_attributes(&mut self) -> Result<Vec<AttributeDecl>, Error> {
        let mut attributes = Vec::new();
        while self.peek_token()?.kind() == TokenKind::Hash {
            self.expect(TokenKind::Hash)?;
            self.expect(TokenKind::LeftBracket)?;
            let name = self.parse_identifier()?;
            if name.data.value == "preview_flag_example" {
                self.features.check_enabled(
                    PreviewFeature::ExamplePreview,
                    name.span,
                    &mut self.result_builder,
                );
            }
            attributes.push(AttributeDecl { name });
            self.expect(TokenKind::RightBracket)?;
        }
        Ok(attributes)
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
            let span = SourceSpan::from(start_span.offset()..(end_span.offset() + end_span.len()));
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
        self.expect(TokenKind::LeftBrace)?;

        let mut statements = Vec::new();
        while !matches!(
            self.peek_token()?.kind(),
            TokenKind::RightBrace | TokenKind::Eof
        ) {
            statements.push(self.parse_statement()?);
        }

        self.expect(TokenKind::RightBrace)?;

        Ok(Block { statements })
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
            span: SourceSpan::from(start_span.offset()..(end_span.offset() + end_span.len())),
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
                span: SourceSpan::from(start_span.offset()..(end_span.offset() + end_span.len())),
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
                    let name = SpannedIdentifier::new(lhs_token.text(), lhs_token.span());
                    ExprData::from(Place {
                        name,
                        field: Vec::new(),
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
                kind @ (TokenKind::Star | TokenKind::Bang | TokenKind::Minus) => {
                    let kind = match kind {
                        TokenKind::Bang => UnaryOpKind::Not,
                        TokenKind::Minus => UnaryOpKind::Neg,
                        TokenKind::Star => UnaryOpKind::Deref,
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
                            TokenKind::Star,
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
                span: SourceSpan::from(
                    lhs_start_span.offset()..(end_span.offset() + end_span.len()),
                ),
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
                // binary infix
                TokenKind::Equal => OpKind::Binary(BinaryOpKind::Assign),
                TokenKind::EqualEqual => OpKind::Binary(BinaryOpKind::Equal),
                TokenKind::BangEqual => OpKind::Binary(BinaryOpKind::NotEqual),
                TokenKind::Ampersand => OpKind::Binary(BinaryOpKind::BitAnd),
                TokenKind::AmpersandAmpersand => OpKind::Binary(BinaryOpKind::And),
                TokenKind::AmpersandEqual => OpKind::Binary(BinaryOpKind::BitAndAssign),
                TokenKind::Pipe => OpKind::Binary(BinaryOpKind::BitOr),
                TokenKind::PipePipe => OpKind::Binary(BinaryOpKind::Or),
                TokenKind::PipeEqual => OpKind::Binary(BinaryOpKind::BitOrAssign),
                TokenKind::Plus => OpKind::Binary(BinaryOpKind::Add),
                TokenKind::PlusEqual => OpKind::Binary(BinaryOpKind::AddAssign),
                TokenKind::Minus => OpKind::Binary(BinaryOpKind::Sub),
                TokenKind::MinusEqual => OpKind::Binary(BinaryOpKind::SubAssign),
                TokenKind::Carrot => OpKind::Binary(BinaryOpKind::Xor),
                TokenKind::CarrotEqual => OpKind::Binary(BinaryOpKind::XOrAssign),
                TokenKind::LessThan => OpKind::Binary(BinaryOpKind::Lt),
                TokenKind::LessThanEqual => OpKind::Binary(BinaryOpKind::Le),
                TokenKind::LessThanLessThan => OpKind::Binary(BinaryOpKind::ShiftLeft),
                TokenKind::LessThanLessThanEqual => OpKind::Binary(BinaryOpKind::ShiftLeftAssign),
                TokenKind::LessThanPercent => OpKind::Binary(BinaryOpKind::RotateLeft),
                TokenKind::LessThanPercentEqual => OpKind::Binary(BinaryOpKind::RotateLeftAssign),
                TokenKind::GreaterThan => OpKind::Binary(BinaryOpKind::Gt),
                TokenKind::GreaterThanEqual => OpKind::Binary(BinaryOpKind::Ge),
                TokenKind::GreaterThanGreaterThan => OpKind::Binary(BinaryOpKind::ShiftRight),
                TokenKind::GreaterThanGreaterThanEqual => {
                    OpKind::Binary(BinaryOpKind::ShiftRightAssign)
                }
                TokenKind::GreaterThanPercent => OpKind::Binary(BinaryOpKind::RotateRight),
                TokenKind::GreaterThanPercentEqual => {
                    OpKind::Binary(BinaryOpKind::RotateRightAssign)
                }
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
                            TokenKind::LessThanLessThanEqual,
                            TokenKind::LessThanPercent,
                            TokenKind::LessThanPercentEqual,
                            TokenKind::GreaterThan,
                            TokenKind::GreaterThanEqual,
                            TokenKind::GreaterThanGreaterThan,
                            TokenKind::GreaterThanGreaterThanEqual,
                            TokenKind::GreaterThanPercent,
                            TokenKind::GreaterThanPercentEqual,
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
                                let span = SourceSpan::from(
                                    op_token.span().offset()..(end_span.offset() + end_span.len()),
                                );
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
                                let span = SourceSpan::from(
                                    op_token.span().offset()..(end_span.offset() + end_span.len()),
                                );
                                self.features.check_enabled(
                                    PreviewFeature::Array,
                                    span,
                                    &mut self.result_builder,
                                );
                                match lhs.data {
                                    ExprData::Place(mut place) => {
                                        place.field.push(FieldPathPart::Index(index));
                                        ExprData::from(place)
                                    }
                                    _ => return Err(Error::from(ArrayIndexOnValueError { span })),
                                }
                            }
                            OpKind::FieldAccess => {
                                let token = self.expect(TokenKind::Ident)?;
                                let name = SpannedIdentifier::new(token.text(), token.span());
                                let span = SourceSpan::from(
                                    op_token.span().offset()
                                        ..(token.span().offset() + token.span().len()),
                                );
                                self.features.check_enabled(
                                    PreviewFeature::FieldAccess,
                                    span,
                                    &mut self.result_builder,
                                );
                                match lhs.data {
                                    ExprData::Place(mut place) => {
                                        place.field.push(FieldPathPart::Field(name));
                                        ExprData::from(place)
                                    }
                                    _ => return Err(Error::from(FieldAccessOnValueError { span })),
                                }
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
                            OpKind::Binary(kind) => {
                                if is_comparison(kind)
                                    && matches!(lhs.data, ExprData::Binary(BinaryOp { kind: left_kind, .. }) if is_comparison(left_kind))
                                {
                                    return Err(Error::ChainedComparison(ChainedComparisonError {
                                        span: op_token.span(),
                                    }));
                                }

                                if is_assign(kind) && !matches!(lhs.data, ExprData::Place(_)) {
                                    return Err(Error::AssignOnValue(AssignOnValueError {
                                        value_span: lhs.span,
                                        op_span: op_token.span(),
                                    }));
                                }

                                ExprData::from(BinaryOp {
                                    kind,
                                    left: Box::new(lhs),
                                    right: Box::new(rhs),
                                })
                            }
                            _ => unimplemented!("{kind:?} is not a binary infix op"),
                        }
                    }
                };
                let end_span = self.lexer.current().unwrap().span();
                Expr {
                    span: SourceSpan::from(
                        lhs_start_span.offset()..(end_span.offset() + end_span.len()),
                    ),
                    data,
                }
            }
        }
        Ok(lhs)
    }
}

fn prefix_binding_power(op: OpKind) -> u8 {
    match op {
        OpKind::Unary(UnaryOpKind::Not | UnaryOpKind::Neg | UnaryOpKind::Deref) => 19,
        _ => unimplemented!("{op:?} is not a prefix op"),
    }
}

fn postfix_binding_power(op: OpKind) -> Option<u8> {
    Some(match op {
        OpKind::FunctionCall => 20,
        OpKind::FieldAccess | OpKind::ArrayIndex => 21,
        _ => return None,
    })
}

fn infix_binding_power(op: OpKind) -> (u8, u8) {
    match op {
        OpKind::Binary(
            BinaryOpKind::Assign
            | BinaryOpKind::AddAssign
            | BinaryOpKind::RotateRightAssign
            | BinaryOpKind::ShiftRightAssign
            | BinaryOpKind::RotateLeftAssign
            | BinaryOpKind::ShiftLeftAssign
            | BinaryOpKind::XOrAssign
            | BinaryOpKind::SubAssign
            | BinaryOpKind::BitOrAssign
            | BinaryOpKind::BitAndAssign,
        ) => (2, 1),
        OpKind::Binary(BinaryOpKind::Or) => (3, 4),
        OpKind::Binary(BinaryOpKind::And) => (5, 6),
        OpKind::Binary(
            BinaryOpKind::Equal
            | BinaryOpKind::NotEqual
            | BinaryOpKind::Lt
            | BinaryOpKind::Gt
            | BinaryOpKind::Le
            | BinaryOpKind::Ge,
        ) => (7, 8),
        OpKind::Binary(BinaryOpKind::BitOr) => (9, 10),
        OpKind::Binary(BinaryOpKind::Xor) => (11, 12),
        OpKind::Binary(BinaryOpKind::BitAnd) => (13, 14),
        OpKind::Binary(
            BinaryOpKind::ShiftLeft
            | BinaryOpKind::ShiftRight
            | BinaryOpKind::RotateLeft
            | BinaryOpKind::RotateRight,
        ) => (15, 16),
        OpKind::Binary(BinaryOpKind::Add | BinaryOpKind::Sub) => (17, 18),
        _ => unimplemented!("{op:?} is not an infix op"),
    }
}

#[must_use]
pub fn is_comparison(op: BinaryOpKind) -> bool {
    matches!(
        op,
        BinaryOpKind::Equal
            | BinaryOpKind::NotEqual
            | BinaryOpKind::Lt
            | BinaryOpKind::Le
            | BinaryOpKind::Gt
            | BinaryOpKind::Ge
    )
}

#[must_use]
pub fn is_assign(op: BinaryOpKind) -> bool {
    matches!(
        op,
        BinaryOpKind::Assign
            | BinaryOpKind::AddAssign
            | BinaryOpKind::SubAssign
            | BinaryOpKind::BitAndAssign
            | BinaryOpKind::BitOrAssign
            | BinaryOpKind::XOrAssign
            | BinaryOpKind::ShiftLeftAssign
            | BinaryOpKind::ShiftRightAssign
            | BinaryOpKind::RotateLeftAssign
            | BinaryOpKind::RotateRightAssign
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comparison_ops_are_comparisons() {
        for op in [
            BinaryOpKind::Equal,
            BinaryOpKind::NotEqual,
            BinaryOpKind::Lt,
            BinaryOpKind::Le,
            BinaryOpKind::Gt,
            BinaryOpKind::Ge,
        ] {
            assert!(is_comparison(op), "{op:?} should be a comparison");
        }
    }

    #[test]
    fn non_comparison_ops_are_not_comparisons() {
        for op in [
            BinaryOpKind::Add,
            BinaryOpKind::Sub,
            BinaryOpKind::And,
            BinaryOpKind::Or,
            BinaryOpKind::BitAnd,
            BinaryOpKind::BitOr,
            BinaryOpKind::Xor,
            BinaryOpKind::Assign,
            BinaryOpKind::ShiftLeft,
            BinaryOpKind::ShiftRight,
            BinaryOpKind::RotateLeft,
            BinaryOpKind::RotateRight,
        ] {
            assert!(!is_comparison(op), "{op:?} should not be a comparison");
        }
    }
}
