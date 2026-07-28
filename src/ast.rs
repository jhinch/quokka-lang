use crate::source::Spanned;
use miette::SourceSpan;
use std::borrow::Borrow;
use std::fmt;
use std::num::NonZeroU16;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Root {
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AttributeDecl {
    pub name: SpannedIdentifier,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Item {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attributes: Vec<AttributeDecl>,
    pub data: ItemData,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, derive_more::From)]
pub enum ItemData {
    FnDecl(FnDecl),
    StructDecl(StructDecl),
    GlobalDecl(GlobalDecl),
    ConstDecl(ConstDecl),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FnDecl {
    pub signature: FnSig,
    pub body: Block,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FnSig {
    pub name: SpannedIdentifier,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ty: Option<TyRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StructDecl {
    pub name: SpannedIdentifier,
    pub fields: Vec<FieldDecl>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FieldDecl {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attributes: Vec<AttributeDecl>,
    pub name: SpannedIdentifier,
    pub ty: TyRef,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GlobalDecl {
    pub name: SpannedIdentifier,
    pub ty: TyRef,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ConstDecl {
    pub name: SpannedIdentifier,
    pub ty: TyRef,
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TyRef {
    #[serde(with = "crate::source::span")]
    pub span: SourceSpan,
    pub name: SpannedIdentifier,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<NonZeroU16>,
}

impl TyRef {
    #[must_use]
    pub const fn new(name: SpannedIdentifier) -> Self {
        TyRef {
            span: name.span,
            name,
            count: None,
        }
    }

    #[must_use]
    pub const fn array(span: SourceSpan, name: SpannedIdentifier, count: NonZeroU16) -> Self {
        TyRef {
            span,
            name,
            count: Some(count),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Block {
    pub statements: Vec<Statement>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Identifier {
    pub value: String,
}

pub type SpannedIdentifier = Spanned<Identifier>;

impl fmt::Display for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value)
    }
}

impl From<String> for Identifier {
    fn from(value: String) -> Self {
        Identifier { value }
    }
}

impl From<&str> for Identifier {
    fn from(value: &str) -> Self {
        Identifier {
            value: String::from(value),
        }
    }
}

impl From<Identifier> for String {
    fn from(value: Identifier) -> Self {
        value.value
    }
}

impl Borrow<str> for Identifier {
    fn borrow(&self) -> &str {
        &self.value
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Statement {
    #[serde(with = "crate::source::span")]
    pub span: SourceSpan,
    pub data: StatementData,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, derive_more::From)]
pub enum StatementData {
    Expr(ExprSmt),
    Let(LetSmt),
    Return(ReturnSmt),
    Break(BreakSmt),
    Continue(ContinueSmt),
    Yield(YieldSmt),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExprSmt {
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LetSmt {
    pub name: SpannedIdentifier,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ty: Option<TyRef>,
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReturnSmt {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BreakSmt {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct YieldSmt {
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ContinueSmt;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Expr {
    #[serde(with = "crate::source::span")]
    pub span: SourceSpan,
    pub data: ExprData,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, derive_more::From)]
pub enum ExprData {
    Place(Place),
    Integer(IntegerLiteral),
    Unary(UnaryOp),
    Binary(BinaryOp),
    Block(Block),
    If(If),
    Loop(Loop),
    Call(Call),
    Paren(Box<Expr>),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Place {
    pub name: SpannedIdentifier,
    pub field: Vec<FieldPathPart>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FieldPathPart {
    Field(SpannedIdentifier),
    Index(Box<Expr>),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IntegerLiteral {
    pub value: u32,
}

impl From<u32> for IntegerLiteral {
    fn from(value: u32) -> Self {
        IntegerLiteral { value }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, derive_more::From)]
pub enum OpKind {
    Unary(UnaryOpKind),
    Binary(BinaryOpKind),
    FieldAccess,
    ArrayIndex,
    FunctionCall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum UnaryOpKind {
    Not,
    Neg,
    Deref,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BinaryOpKind {
    Assign,
    Equal,
    NotEqual,
    And,
    Or,
    BitAnd,
    BitOr,
    Xor,
    Lt,
    Le,
    Gt,
    Ge,
    RotateLeft,
    RotateRight,
    ShiftLeft,
    ShiftRight,
    Add,
    Sub,
    AddAssign,
    SubAssign,
    BitAndAssign,
    BitOrAssign,
    XOrAssign,
    ShiftLeftAssign,
    ShiftRightAssign,
    RotateLeftAssign,
    RotateRightAssign,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct UnaryOp {
    pub kind: UnaryOpKind,
    pub expr: Box<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BinaryOp {
    pub kind: BinaryOpKind,
    pub left: Box<Expr>,
    pub right: Box<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct If {
    pub condition: Box<Expr>,
    pub true_block: Box<Expr>,
    pub false_block: Option<Box<Expr>>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Loop {
    pub body: Block,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Call {
    pub callee: Place,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Spanned;
    use miette::{SourceOffset, SourceSpan};
    use std::borrow::Borrow;

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
        assert_eq!(id.value, "foo");
    }

    #[test]
    fn identifier_from_str() {
        let id = Identifier::from("bar");
        assert_eq!(id.value, "bar");
    }

    #[test]
    fn string_from_identifier() {
        let id = Identifier::from("baz");
        let s = String::from(id);
        assert_eq!(s, "baz");
    }

    #[test]
    fn identifier_borrow_str() {
        let id = Identifier::from("qux");
        let borrowed: &str = id.borrow();
        assert_eq!(borrowed, "qux");
    }

    #[test]
    fn identifier_borrow_allows_hashmap_str_lookup() {
        use std::collections::HashMap;
        let mut map: HashMap<Identifier, i32> = HashMap::new();
        map.insert(Identifier::from("key"), 42);
        assert_eq!(map.get("key"), Some(&42));
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
        assert_eq!(ty.name.data.value, "u32");
        assert!(ty.count.is_none());
    }

    #[test]
    fn ty_ref_array_has_count() {
        let count = NonZeroU16::new(8).unwrap();
        let ty = TyRef::array(dummy_span(), ident("u8"), count);
        assert_eq!(ty.name.data.value, "u8");
        assert_eq!(ty.count, Some(count));
    }

    #[test]
    fn ty_ref_array_count_one() {
        let count = NonZeroU16::new(1).unwrap();
        let ty = TyRef::array(dummy_span(), ident("u8"), count);
        assert_eq!(ty.count.unwrap().get(), 1);
    }
}
