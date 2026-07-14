use crate::source::Spanned;
use std::borrow::Borrow;
use std::fmt;
use std::num::NonZeroU32;

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
    pub name: SpannedIdentifier,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<NonZeroU32>,
}

impl TyRef {
    pub const fn new(name: SpannedIdentifier) -> Self {
        TyRef { name, count: None }
    }

    pub const fn array(name: SpannedIdentifier, count: NonZeroU32) -> Self {
        TyRef {
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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, derive_more::From)]
pub enum Statement {
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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, derive_more::From)]
pub enum Expr {
    Place(Place),
    Integer(SpannedIntegerLiteral),
    Unary(UnaryOp),
    Binary(BinaryOp),
    Block(Block),
    If(If),
    Loop(Loop),
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
    Index(u32),
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

pub type SpannedIntegerLiteral = Spanned<IntegerLiteral>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, derive_more::From)]
pub enum OpKind {
    Unary(UnaryOpKind),
    Binary(BinaryOpKind),
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
