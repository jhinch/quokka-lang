use crate::ast::*;
use crate::feature::{PreviewFeature, PreviewFeatureError, PreviewFeatureSet};
use crate::lex::{self, Token, TokenKind};
use crate::multi::{MultiResult, MultiResultBuilder};
use miette::{SourceOffset, SourceSpan};

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
    ChainedComparison(#[from] ChainedComparisonError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Preview(#[from] PreviewFeatureError),
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
        self.features.enable(feature)
    }

    pub fn enable_features(&mut self, features: PreviewFeatureSet) {
        self.features.extend(features)
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
                    continue;
                }
                Err(Error::Lex(e)) => return Err(Error::Lex(e)),
                Err(e) => {
                    self.result_builder.add_error(e);
                    self.parse_item_error()?;
                }
            };
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
            if name.value.value == "preview_flag_example" {
                self.features.check_enabled(
                    PreviewFeature::Example,
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
                    expected: vec![TokenKind::Fn, TokenKind::Global, TokenKind::Const, TokenKind::Struct],
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
        let value = self.parse_expr()?;
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
        let (name, count) = if self.peek_token()?.kind() == TokenKind::LeftBracket {
            // fixed size array
            let span = self.peek_token()?.span();
            self.features
                .check_enabled(PreviewFeature::Array, span, &mut self.result_builder);
            self.expect(TokenKind::LeftBracket)?;
            let name = self.parse_identifier()?;
            self.expect(TokenKind::Semicolon)?;
            let count = self.expect(TokenKind::Number)?.try_parse_non_zero_u32()?;
            self.expect(TokenKind::RightBracket)?;
            (name, Some(count))
        } else {
            // regular type is equivalent to an array of size 1
            let name = self.parse_identifier()?;
            (name, None)
        };
        Ok(TyRef { name, count })
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
        Ok(match token.kind() {
            TokenKind::Let => Statement::from(self.parse_local_decl()?),
            TokenKind::Return => Statement::from(self.parse_return()?),
            TokenKind::Break => Statement::from(self.parse_break()?),
            TokenKind::Yield => Statement::from(self.parse_yield()?),
            TokenKind::Continue => Statement::from(self.parse_continue()?),
            _ => Statement::from(self.parse_expr_smt()?),
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
        self.expect(lex::TokenKind::Equal)?;
        let value = self.parse_expr()?;
        self.expect(TokenKind::Semicolon)?;

        Ok(LetSmt { name, ty, value })
    }

    fn parse_return(&mut self) -> Result<ReturnSmt, Error> {
        self.expect(TokenKind::Return)?;
        let value = if self.peek_token()?.kind() == TokenKind::Semicolon {
            self.expect(TokenKind::Semicolon)?;
            None
        } else {
            let value = self.parse_expr()?;
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
            let value = self.parse_expr()?;
            self.expect(TokenKind::Semicolon)?;
            Some(value)
        };
        Ok(BreakSmt { value })
    }

    fn parse_yield(&mut self) -> Result<YieldSmt, Error> {
        self.expect(TokenKind::Yield)?;
        let value = self.parse_expr()?;
        self.expect(TokenKind::Semicolon)?;
        Ok(YieldSmt { value })
    }

    fn parse_continue(&mut self) -> Result<ContinueSmt, Error> {
        self.expect(TokenKind::Continue)?;
        self.expect(TokenKind::Semicolon)?;
        Ok(ContinueSmt)
    }

    fn parse_expr_smt(&mut self) -> Result<ExprSmt, Error> {
        let value = self.parse_expr()?;
        if !self.is_proceeding_block() || self.peek_token()?.kind() == TokenKind::Semicolon {
            self.expect(TokenKind::Semicolon)?;
        }
        Ok(ExprSmt { value })
    }

    fn parse_expr_or_block(&mut self) -> Result<Expr, Error> {
        Ok(if self.peek_token()?.kind() == TokenKind::LeftBrace {
            Expr::from(self.parse_body()?)
        } else {
            self.parse_expr_above(0)?
        })
    }

    fn parse_expr(&mut self) -> Result<Expr, Error> {
        self.parse_expr_above(0)
    }

    fn parse_expr_above(&mut self, min_bp: u8) -> Result<Expr, Error> {
        let lhs_token = self.consume_token()?;
        let mut lhs = match lhs_token.kind() {
            // atoms
            TokenKind::Number => Expr::from(SpannedIntegerLiteral::new(
                lhs_token.try_parse_u32()?,
                lhs_token.span(),
            )),
            TokenKind::Ident => {
                let name = SpannedIdentifier::new(lhs_token.text(), lhs_token.span());
                if self.peek_token()?.kind() == TokenKind::LeftParen {
                    let start_span = self.expect(TokenKind::LeftParen)?.span();
                    let end_span = self.expect(TokenKind::RightParen)?.span();
                    let span = SourceSpan::new(
                        SourceOffset::from(start_span.offset()),
                        end_span.offset() + end_span.len() - start_span.offset(),
                    );
                    self.features.check_enabled(
                        PreviewFeature::FunctionCall,
                        span,
                        &mut self.result_builder,
                    );
                    Expr::from(Call { name })
                } else {
                    let mut field = Vec::new();
                    loop {
                        field.push(match self.peek_token()?.kind() {
                            TokenKind::Dot => {
                                // field
                                let span = self.expect(TokenKind::Dot)?.span();
                                self.features.check_enabled(
                                    PreviewFeature::FieldAccess,
                                    span,
                                    &mut self.result_builder,
                                );
                                let field_name = self.parse_identifier()?;
                                FieldPathPart::Field(field_name)
                            }
                            TokenKind::LeftBracket => {
                                // index
                                let start_span = self.expect(TokenKind::LeftBracket)?.span();
                                let index = self.expect(TokenKind::Number)?.try_parse_u32()?;
                                let end_span = self.expect(TokenKind::RightBracket)?.span();
                                let span = SourceSpan::new(
                                    SourceOffset::from(start_span.offset()),
                                    end_span.offset() + end_span.len() - start_span.offset(),
                                );
                                self.features.check_enabled(
                                    PreviewFeature::Array,
                                    span,
                                    &mut self.result_builder,
                                );
                                FieldPathPart::Index(index)
                            }
                            _ => break,
                        });
                    }
                    Expr::from(Place { name, field })
                }
            }
            // group
            TokenKind::LeftParen => {
                let expr = self.parse_expr_above(0)?;
                self.expect(TokenKind::RightParen)?;
                Expr::from(Box::new(expr))
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

                Expr::from(If {
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
                Expr::from(Loop { body })
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
                    _ => unreachable!(),
                };
                let bp = prefix_binding_power(OpKind::Unary(kind));
                let expr = Box::new(self.parse_expr_above(bp)?);
                Expr::from(UnaryOp { kind, expr })
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

        loop {
            let op_token = self.peek_token()?;
            let kind = match op_token.kind() {
                TokenKind::Eof
                | TokenKind::RightParen
                | TokenKind::Semicolon
                | TokenKind::RightBrace
                | TokenKind::Then
                | TokenKind::Else => break,
                // binary infix
                TokenKind::Equal => BinaryOpKind::Assign,
                TokenKind::EqualEqual => BinaryOpKind::Equal,
                TokenKind::BangEqual => BinaryOpKind::NotEqual,
                TokenKind::Ampersand => BinaryOpKind::BitAnd,
                TokenKind::AmpersandAmpersand => BinaryOpKind::And,
                TokenKind::AmpersandEqual => BinaryOpKind::BitAndAssign,
                TokenKind::Pipe => BinaryOpKind::BitOr,
                TokenKind::PipePipe => BinaryOpKind::Or,
                TokenKind::PipeEqual => BinaryOpKind::BitOrAssign,
                TokenKind::Plus => BinaryOpKind::Add,
                TokenKind::PlusEqual => BinaryOpKind::AddAssign,
                TokenKind::Minus => BinaryOpKind::Sub,
                TokenKind::MinusEqual => BinaryOpKind::SubAssign,
                TokenKind::Carrot => BinaryOpKind::Xor,
                TokenKind::CarrotEqual => BinaryOpKind::XOrAssign,
                TokenKind::LessThan => BinaryOpKind::Lt,
                TokenKind::LessThanEqual => BinaryOpKind::Le,
                TokenKind::LessThanLessThan => BinaryOpKind::ShiftLeft,
                TokenKind::LessThanLessThanEqual => BinaryOpKind::ShiftLeftAssign,
                TokenKind::LessThanPercent => BinaryOpKind::RotateLeft,
                TokenKind::LessThanPercentEqual => BinaryOpKind::RotateLeftAssign,
                TokenKind::GreaterThan => BinaryOpKind::Gt,
                TokenKind::GreaterThanEqual => BinaryOpKind::Ge,
                TokenKind::GreaterThanGreaterThan => BinaryOpKind::ShiftRight,
                TokenKind::GreaterThanGreaterThanEqual => BinaryOpKind::ShiftRightAssign,
                TokenKind::GreaterThanPercent => BinaryOpKind::RotateRight,
                TokenKind::GreaterThanPercentEqual => BinaryOpKind::RotateRightAssign,
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
            let (lhs_bp, rhs_bp) = infix_binding_power(OpKind::Binary(kind));
            if lhs_bp < min_bp {
                break;
            }
            if is_comparison(kind)
                && matches!(lhs, Expr::Binary(BinaryOp { kind: left_kind, .. }) if is_comparison(left_kind))
            {
                return Err(Error::ChainedComparison(ChainedComparisonError {
                    span: op_token.span(),
                }));
            }
            self.chomp()?;
            let rhs = self.parse_expr_above(rhs_bp)?;
            lhs = Expr::from(BinaryOp {
                kind,
                left: Box::new(lhs),
                right: Box::new(rhs),
            });
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
