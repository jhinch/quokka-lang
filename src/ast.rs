use crate::source::Spanned;
use crate::symbol::Symbol;
use miette::SourceSpan;
use std::fmt;
use std::num::NonZeroU16;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Root {
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AttributeDecl {
    #[serde(with = "crate::source::span")]
    pub span: SourceSpan,
    pub name: SpannedIdentifier,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<AttributeValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AttributeValue {
    #[serde(with = "crate::source::span")]
    pub span: SourceSpan,
    pub data: AttributeValueData,
}

impl From<SpannedIdentifier> for AttributeValue {
    fn from(value: SpannedIdentifier) -> Self {
        AttributeValue {
            span: value.span,
            data: AttributeValueData::from(value.data),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum AttributeValueData {
    Ident(Identifier),
    Integer(IntegerLiteral),
}

impl From<Identifier> for AttributeValueData {
    fn from(value: Identifier) -> Self {
        AttributeValueData::Ident(value)
    }
}

impl From<IntegerLiteral> for AttributeValueData {
    fn from(value: IntegerLiteral) -> Self {
        AttributeValueData::Integer(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Item {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attributes: Vec<AttributeDecl>,
    pub data: ItemData,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, derive_more::From)]
pub enum ItemData {
    FnDecl(FnDecl),
    StructDecl(StructDecl),
    GlobalDecl(GlobalDecl),
    ConstDecl(ConstDecl),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FnDecl {
    pub signature: FnSig,
    pub body: Block,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FnSig {
    pub name: SpannedIdentifier,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ty: Option<TyRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StructDecl {
    pub name: SpannedIdentifier,
    pub fields: Vec<FieldDecl>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FieldDecl {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attributes: Vec<AttributeDecl>,
    pub name: SpannedIdentifier,
    pub ty: TyRef,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct GlobalDecl {
    pub name: SpannedIdentifier,
    pub ty: TyRef,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ConstDecl {
    pub name: SpannedIdentifier,
    pub ty: TyRef,
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TyRef {
    #[serde(with = "crate::source::span")]
    pub span: SourceSpan,
    pub data: TyRefData,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum TyRefData {
    Simple(SpannedIdentifier),
    Array(Box<TyRef>, NonZeroU16),
}

impl TyRef {
    #[must_use]
    pub fn new(name: SpannedIdentifier) -> Self {
        TyRef {
            span: name.span,
            data: TyRefData::Simple(name),
        }
    }

    #[must_use]
    pub fn array(span: SourceSpan, item: TyRef, count: NonZeroU16) -> Self {
        TyRef {
            span,
            data: TyRefData::Array(Box::new(item), count),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Block {
    #[serde(with = "crate::source::span")]
    pub span: SourceSpan,
    pub statements: Vec<Statement>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(transparent)]
pub struct Identifier {
    pub value: Symbol,
}

impl Identifier {
    #[must_use]
    pub fn unnamed() -> Self {
        Identifier {
            value: Symbol::from_static("_"),
        }
    }
}

impl Identifier {
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.value.as_str()
    }
}

pub type SpannedIdentifier = Spanned<Identifier>;

impl fmt::Display for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value)
    }
}

impl From<Symbol> for Identifier {
    fn from(value: Symbol) -> Self {
        Identifier { value }
    }
}

impl From<String> for Identifier {
    fn from(value: String) -> Self {
        Identifier {
            value: Symbol::from(value),
        }
    }
}

impl From<&str> for Identifier {
    fn from(value: &str) -> Self {
        Identifier {
            value: Symbol::from(value),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Statement {
    #[serde(with = "crate::source::span")]
    pub span: SourceSpan,
    pub data: StatementData,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, derive_more::From)]
pub enum StatementData {
    Expr(ExprSmt),
    Let(LetSmt),
    Return(ReturnSmt),
    Break(BreakSmt),
    Continue(ContinueSmt),
    Yield(YieldSmt),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ExprSmt {
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LetSmt {
    pub name: SpannedIdentifier,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ty: Option<TyRef>,
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ReturnSmt {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BreakSmt {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct YieldSmt {
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ContinueSmt;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Expr {
    #[serde(with = "crate::source::span")]
    pub span: SourceSpan,
    pub data: ExprData,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, derive_more::From)]
pub enum ExprData {
    Place(Place),
    Integer(IntegerLiteral),
    Struct(StructExpr),
    Array(ArrayExpr),
    Repeat(RepeatExpr),
    Unary(UnaryOp),
    Logical(LogicalOp),
    Arithmetic(ArithmeticOp),
    Comparison(ComparisonOp),
    Assign(AssignOp),
    Block(Block),
    If(If),
    Loop(Loop),
    Call(Call),
    Paren(Box<Expr>),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Place {
    pub name: SpannedIdentifier,
    #[serde(with = "crate::source::span")]
    pub span: SourceSpan,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projections: Vec<PlaceProjection>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum PlaceProjection {
    Field(SpannedIdentifier),
    Index(Box<Expr>),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct IntegerLiteral {
    pub value: u32,
}

impl From<u32> for IntegerLiteral {
    fn from(value: u32) -> Self {
        IntegerLiteral { value }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StructExpr {
    pub name: SpannedIdentifier,
    pub fields: Vec<StructExprField>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StructExprField {
    pub name: SpannedIdentifier,
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ArrayExpr {
    pub items: Vec<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RepeatExpr {
    pub item: Box<Expr>,
    pub count: NonZeroU16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, derive_more::From)]
pub enum OpKind {
    Unary(UnaryOpKind),
    Logical(LogicalOpKind),
    Arithmetic(ArithmeticOpKind),
    Comparison(ComparisonOpKind),
    Assign(AssignOpKind),
    FieldAccess,
    ArrayIndex,
    FunctionCall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum UnaryOpKind {
    Not,
    Neg,
    ShiftLeft,
    ShiftRight,
    RotateLeft,
    RotateRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum LogicalOpKind {
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum ArithmeticOpKind {
    BitAnd,
    BitOr,
    BitXor,
    Add,
    Sub,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum ComparisonOpKind {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum AssignOpKind {
    Assign,
    AddAssign,
    SubAssign,
    BitAndAssign,
    BitOrAssign,
    BitXorAssign,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct UnaryOp {
    pub kind: UnaryOpKind,
    pub expr: Box<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LogicalOp {
    pub kind: LogicalOpKind,
    pub left: Box<Expr>,
    pub right: Box<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ArithmeticOp {
    pub kind: ArithmeticOpKind,
    pub left: Box<Expr>,
    pub right: Box<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ComparisonOp {
    pub kind: ComparisonOpKind,
    pub left: Box<Expr>,
    pub right: Box<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AssignOp {
    pub kind: AssignOpKind,
    pub left: Box<Place>,
    pub right: Box<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct If {
    pub condition: Box<Expr>,
    pub true_block: Box<Expr>,
    pub false_block: Option<Box<Expr>>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Loop {
    pub body: Block,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Call {
    pub callee: Place,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Spanned;
    use miette::{SourceOffset, SourceSpan};
    use std::assert_matches;

    fn dummy_span() -> SourceSpan {
        SourceSpan::new(SourceOffset::from(0), 0)
    }

    fn ident(name: &str) -> SpannedIdentifier {
        Spanned::new(Identifier::from(name), dummy_span())
    }

    #[test]
    fn identifier_display_outputs_value() {
        let id = Identifier::from("hello");
        assert_eq!(format!("{id}"), "hello");
    }

    #[test]
    fn identifier_display_empty() {
        let id = Identifier::from("");
        assert_eq!(format!("{id}"), "");
    }

    #[test]
    fn identifier_from_string() {
        let id = Identifier::from(String::from("foo"));
        assert_eq!(id.value.as_str(), "foo");
    }

    #[test]
    fn identifier_from_str() {
        let id = Identifier::from("bar");
        assert_eq!(id.value.as_str(), "bar");
    }

    #[test]
    fn integer_literal_from_u32() {
        let lit = IntegerLiteral::from(99);
        assert_eq!(lit.value, 99);
    }

    #[test]
    fn integer_literal_from_zero() {
        let lit = IntegerLiteral::from(0);
        assert_eq!(lit.value, 0);
    }

    #[test]
    fn integer_literal_from_max() {
        let lit = IntegerLiteral::from(u32::MAX);
        assert_eq!(lit.value, u32::MAX);
    }

    #[test]
    fn ty_ref_new_is_scalar() {
        let ty = TyRef::new(ident("u32"));
        assert_matches!(ty.data, TyRefData::Simple(name) if &*name.data.as_str() == "u32");
    }

    #[test]
    fn ty_ref_array() {
        let count = NonZeroU16::new(8).unwrap();
        let ty = TyRef::array(dummy_span(), TyRef::new(ident("u8")), count);
        assert_matches!(ty.data, TyRefData::Array(item, count) if matches!(&item.data, TyRefData::Simple(name) if name.data.as_str() == "u8") && count.get() == 8);
    }
}
