use crate::analyze::infer::InferredTyDisplay;
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
#[error("recursive type `{name}` detected")]
#[diagnostic()]
pub struct TypeRecursiveError {
    pub name: Identifier,
    #[label("this type")]
    pub span: SourceSpan,
}

impl From<SpannedIdentifier> for TypeRecursiveError {
    fn from(value: SpannedIdentifier) -> Self {
        TypeRecursiveError {
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
    #[label("expected `yield` before this closing brace")]
    pub span: SourceSpan,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("cannot `yield` from a unit block")]
#[diagnostic()]
pub struct YieldUnexpectedError {
    #[label("value yielded here")]
    pub yield_span: SourceSpan,
    #[label("block does not produce a value")]
    pub block_span: SourceSpan,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("`return` missing in block")]
#[diagnostic()]
pub struct ReturnMissingError {
    #[label("expected `return` before this closing brace")]
    pub span: SourceSpan,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("`return` missing a value")]
#[diagnostic()]
pub struct ReturnExprMissingError {
    #[label("missing value here")]
    pub return_span: SourceSpan,
    #[label("return type declared")]
    pub function_span: SourceSpan,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("unexpected `return` value")]
#[diagnostic()]
pub struct ReturnExprUnexpectedError {
    #[label("value returned here")]
    pub return_span: SourceSpan,
    #[label("no return type declared")]
    pub function_span: SourceSpan,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("cannot `break` with value from a unit loop")]
#[diagnostic()]
pub struct BreakExprUnexpectedError {
    #[label("break value here")]
    pub break_span: SourceSpan,
    #[label("loop does not produce a value")]
    pub loop_span: SourceSpan,
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
    pub ty1: InferredTyDisplay<'static>,
    #[label("has type `{ty2}`")]
    pub ty2_span: SourceSpan,
    pub ty2: InferredTyDisplay<'static>,
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
#[error("cannot assign to a constant")]
#[diagnostic()]
pub struct ConstNotAssignableError {
    #[label("this is a constant")]
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

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("if expression are not support in constant declarations")]
#[diagnostic()]
pub struct IfNotSupportInConstDeclError {
    #[label("not permitted")]
    pub span: SourceSpan,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("function calls are not support in constant declarations")]
#[diagnostic()]
pub struct CallNotSupportInConstDeclError {
    #[label("not permitted")]
    pub span: SourceSpan,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("loops are not support in constant declarations")]
#[diagnostic()]
pub struct LoopNotSupportInConstDeclError {
    #[label("not permitted")]
    pub span: SourceSpan,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("cannot reference global `{name}` in constant declaration")]
#[diagnostic()]
pub struct GlobalReferencedInConstDeclError {
    pub name: Identifier,
    #[label("not permitted")]
    pub span: SourceSpan,
}

impl From<SpannedIdentifier> for GlobalReferencedInConstDeclError {
    fn from(value: SpannedIdentifier) -> Self {
        GlobalReferencedInConstDeclError {
            name: value.data,
            span: value.span,
        }
    }
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("type `{ty}` too large to fit in a local variable. {bit_size} bits > 16 bits")]
#[diagnostic()]
pub struct TyTooLargeForLocalVariableError {
    pub ty: TyDisplay<'static>,
    pub bit_size: u32,
    #[label("too large")]
    pub span: SourceSpan,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("const `{name}` has a cyclic dependency")]
#[diagnostic()]
pub struct CyclicConstDeclError {
    pub name: Identifier,
    #[label("this constant")]
    pub span: SourceSpan,
    #[label(collection, "require `{name}` to be initialized")]
    pub dependencies: Vec<SourceSpan>,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("field `{name}` defined multiple times in struct expression")]
#[diagnostic()]
pub struct DuplicateFieldExprError {
    pub name: Identifier,
    #[label("previously defined here")]
    pub defined: SourceSpan,
    #[label("redefined here")]
    pub redefined: SourceSpan,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("field `{name}` is missing in struct expression")]
#[diagnostic()]
pub struct MissingFieldExprError {
    pub name: Identifier,
    #[label("defined here")]
    pub defined: SourceSpan,
    #[label("missing here")]
    pub missing: SourceSpan,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("empty array expressions are not allowed")]
#[diagnostic()]
pub struct EmptyArrayExprError {
    #[label("this array is empty")]
    pub span: SourceSpan,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("array expression is too large")]
#[diagnostic()]
pub struct OversizedArrayExprError {
    #[label("too many items")]
    pub span: SourceSpan,
}
