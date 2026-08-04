use crate::ast::{Identifier, SpannedIdentifier};
use crate::ty::TyDisplay;
use miette::SourceSpan;

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("cannot find type `{name}`")]
#[diagnostic()]
pub struct TypeNotFoundError {
    pub name: Identifier,
    #[label("this type")]
    pub span: SourceSpan,
}

impl From<SpannedIdentifier> for TypeNotFoundError {
    fn from(value: SpannedIdentifier) -> Self {
        TypeNotFoundError {
            name: value.data,
            span: value.span,
        }
    }
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("`continue` outside of a loop")]
#[diagnostic()]
pub struct ContinueOutsideLoopError {
    #[label("`continue` not allowed here")]
    pub span: SourceSpan,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("`break` outside of a loop")]
#[diagnostic()]
pub struct BreakOutsideLoopError {
    #[label("`break` not allowed here")]
    pub span: SourceSpan,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("`return` outside of a function")]
#[diagnostic()]
pub struct ReturnOutsideFnError {
    #[label("`return` not allowed here")]
    pub span: SourceSpan,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("`yield` outside of a block")]
#[diagnostic()]
pub struct YieldOutsideBlockError {
    #[label("`yield` not allowed here")]
    pub span: SourceSpan,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("`yield` missing in expression block")]
#[diagnostic()]
pub struct YieldMissingError {
    #[label("this block")]
    pub span: SourceSpan,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("name `{name}` defined multiple times in scope")]
#[diagnostic()]
pub struct RedefinedError {
    pub name: Identifier,
    #[label("previously defined here")]
    pub defined: SourceSpan,
    #[label("redefined here")]
    pub redefined: SourceSpan,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("name `{name}` not defined in scope")]
#[diagnostic()]
pub struct NotDefinedError {
    pub name: Identifier,
    #[label("not defined")]
    pub span: SourceSpan,
}

impl From<SpannedIdentifier> for NotDefinedError {
    fn from(value: SpannedIdentifier) -> Self {
        NotDefinedError {
            name: value.data,
            span: value.span,
        }
    }
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("`{ty1}` is not equal to `{ty2}`")]
#[diagnostic()]
pub struct TyMismatchError {
    #[label("has type `{ty1}`")]
    pub ty1_span: SourceSpan,
    pub ty1: TyDisplay<'static>,
    #[label("has type `{ty2}`")]
    pub ty2_span: SourceSpan,
    pub ty2: TyDisplay<'static>,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("`{ty}` is not an integer type")]
#[diagnostic()]
pub struct NotIntError {
    #[label("has type `{ty}`")]
    pub span: SourceSpan,
    pub ty: TyDisplay<'static>,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("`{ty}` is not a signed integer type")]
#[diagnostic()]
pub struct NotSignedIntError {
    #[label("has type `{ty}`")]
    pub span: SourceSpan,
    pub ty: TyDisplay<'static>,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("`{ty}` is not a struct type")]
#[diagnostic()]
pub struct NotStructError {
    #[label("has type `{ty}`")]
    pub span: SourceSpan,
    pub ty: TyDisplay<'static>,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("field `{name}` defined multiple times on `{ty}`")]
#[diagnostic()]
pub struct FieldRedefinedError {
    pub name: Identifier,
    pub ty: Identifier,
    #[label("previously defined here")]
    pub defined: SourceSpan,
    #[label("redefined here")]
    pub redefined: SourceSpan,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("field `{field}` is not defined on `{ty}`")]
#[diagnostic()]
pub struct FieldNotDefinedError {
    #[label("`{ty}` is defined here")]
    pub struct_span: SourceSpan,
    pub ty: TyDisplay<'static>,
    #[label("not defined")]
    pub field_span: SourceSpan,
    pub field: Identifier,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("`{ty}` is not an array type")]
#[diagnostic()]
pub struct NotArrayError {
    #[label("has type `{ty}`")]
    pub span: SourceSpan,
    pub ty: TyDisplay<'static>,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("`{ty}` is not a function type")]
#[diagnostic()]
pub struct NotFnPtrError {
    #[label("has type `{ty}`")]
    pub span: SourceSpan,
    pub ty: TyDisplay<'static>,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("cannot assign a function")]
#[diagnostic()]
pub struct FnPtrNotAssignableError {
    #[label("this is a function")]
    pub span: SourceSpan,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("unreachable code")]
#[diagnostic()]
pub struct UnreachableError {
    #[label("unreachable")]
    pub span: SourceSpan,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("type cannot be determined by inference")]
#[diagnostic()]
pub struct TypeUnresolvedError {
    #[label("needs type annotation")]
    pub span: SourceSpan,
}
