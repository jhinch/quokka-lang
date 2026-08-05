use crate::ast;
use miette::SourceSpan;
use std::fmt;
use std::marker::PhantomData;

#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FunctionAttributes {
    #[serde(default, deserialize_with = "deserialize_unit_as_bool")]
    pub entrypoint: bool,
    #[serde(default, deserialize_with = "deserialize_unit_as_bool")]
    pub no_startup_code: bool,
}

impl TryFrom<&[ast::AttributeDecl]> for FunctionAttributes {
    type Error = Error;

    fn try_from(value: &[ast::AttributeDecl]) -> Result<Self, Self::Error> {
        <FunctionAttributes as serde::Deserialize>::deserialize(Deserializer::new(value))
    }
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum Error {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Unknown(#[from] UnknownAttributeError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Duplicate(#[from] DuplicateAttributeError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    MalformedValue(#[from] MalformedAttributeValueError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Malformed(#[from] MalformedAttributesError),
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("malformed attributes: {msg}")]
pub struct MalformedAttributesError {
    pub msg: String,
    #[label("these attributes")]
    pub span: Option<SourceSpan>,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("unknown attribute `{name}`")]
pub struct UnknownAttributeError {
    pub name: String,
    #[label("this attribute")]
    pub span: SourceSpan,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("attribute `{name}` cannot be defined multiple times")]
pub struct DuplicateAttributeError {
    pub name: String,
    #[label("previously defined here")]
    pub defined: Option<SourceSpan>,
    #[label("redefined here")]
    pub redefined: Option<SourceSpan>,
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("malformed attributes: {msg}")]
pub struct MalformedAttributeValueError {
    pub msg: String,
    #[label("these attributes")]
    pub span: SourceSpan,
}

impl serde::de::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Error::from(MalformedAttributesError {
            msg: msg.to_string(),
            span: None,
        })
    }

    fn duplicate_field(field: &'static str) -> Self {
        Error::from(DuplicateAttributeError {
            name: field.to_string(),
            defined: None,
            redefined: None,
        })
    }
}

pub struct Deserializer<'de> {
    attributes: &'de [ast::AttributeDecl],
    current: Option<&'de ast::AttributeDecl>,
    count: usize,
    _type: PhantomData<&'de ast::AttributeDecl>,
}

impl<'de> Deserializer<'de> {
    #[must_use]
    pub fn new(attributes: &'de [ast::AttributeDecl]) -> Self {
        Deserializer {
            attributes,
            current: None,
            count: 0,
            _type: PhantomData,
        }
    }
}

impl<'de> serde::Deserializer<'de> for Deserializer<'de> {
    type Error = Error;

    fn deserialize_any<V: serde::de::Visitor<'de>>(
        self,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        let attributes = self.attributes;
        visitor.visit_map(self).map_err(|error| match error {
            Error::Malformed(mut error) => {
                error.span = attributes.iter().map(|attr| attr.name.span).reduce(|a, b| {
                    SourceSpan::from(
                        a.offset().min(b.offset())
                            ..(a.offset() + a.len()).max(b.offset() + b.len()),
                    )
                });
                Error::from(error)
            }
            Error::Duplicate(mut error) => {
                if let Some(defined_index) = attributes
                    .iter()
                    .position(|decl| decl.name.data.value == error.name)
                {
                    error.defined = Some(attributes[defined_index].name.span);
                    error.redefined = attributes
                        .iter()
                        .skip(defined_index + 1)
                        .find(|decl| decl.name.data.value == error.name)
                        .map(|decl| decl.name.span);
                }
                Error::from(error)
            }
            error => error,
        })
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct enum identifier ignored_any
    }
}

impl<'de> serde::de::MapAccess<'de> for Deserializer<'de> {
    type Error = Error;

    fn next_key_seed<K: serde::de::DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Self::Error> {
        self.current = self.attributes.get(self.count);
        let current = match self.current {
            Some(current) => current,
            None => return Ok(None),
        };
        self.count += 1;
        seed.deserialize(AttrNameDeserializer::from(current))
            .map(Some)
            .map_err(Error::from)
    }

    fn next_value_seed<V: serde::de::DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, Self::Error> {
        let current = self
            .current
            .take()
            .expect("MapAccess::next_value called before next_key");
        seed.deserialize(AttrValueDeserializer::from(current))
            .map_err(Error::from)
    }
}

#[derive(Debug, thiserror::Error)]
enum AttrNameError {
    #[error(transparent)]
    Unknown(#[from] UnknownAttributeError),
    #[error(transparent)]
    Malformed(#[from] MalformedAttributesError),
}

impl From<AttrNameError> for Error {
    fn from(error: AttrNameError) -> Self {
        match error {
            AttrNameError::Unknown(error) => Error::from(error),
            AttrNameError::Malformed(error) => Error::from(error),
        }
    }
}

impl serde::de::Error for AttrNameError {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        AttrNameError::from(MalformedAttributesError {
            msg: msg.to_string(),
            span: None,
        })
    }

    fn unknown_variant(variant: &str, _: &'static [&'static str]) -> Self {
        AttrNameError::from(UnknownAttributeError {
            name: variant.to_string(),
            span: SourceSpan::from(0..1),
        })
    }

    fn unknown_field(field: &str, _: &'static [&'static str]) -> Self {
        AttrNameError::from(UnknownAttributeError {
            name: field.to_string(),
            span: SourceSpan::from(0..1),
        })
    }
}

#[derive(derive_more::From)]
struct AttrNameDeserializer<'de> {
    attribute: &'de ast::AttributeDecl,
}

impl<'de> serde::Deserializer<'de> for AttrNameDeserializer<'de> {
    type Error = AttrNameError;

    fn deserialize_any<V: serde::de::Visitor<'de>>(
        self,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        visitor
            .visit_borrowed_str::<Self::Error>(self.attribute.name.data.value.as_str())
            .map_err(|error| match error {
                AttrNameError::Unknown(mut error) => {
                    error.span = self.attribute.name.span;
                    AttrNameError::from(error)
                }
                error => error,
            })
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct enum identifier ignored_any
    }
}

#[derive(Debug, thiserror::Error)]
enum AttrValueError {
    #[error(transparent)]
    MalformedValue(#[from] MalformedAttributeValueError),
}

impl From<AttrValueError> for Error {
    fn from(AttrValueError::MalformedValue(error): AttrValueError) -> Self {
        Error::from(error)
    }
}

impl serde::de::Error for AttrValueError {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        AttrValueError::from(MalformedAttributeValueError {
            msg: msg.to_string(),
            span: SourceSpan::from(0..1),
        })
    }
}

#[derive(derive_more::From)]
struct AttrValueDeserializer<'de> {
    attribute: &'de ast::AttributeDecl,
}

impl<'de> serde::Deserializer<'de> for AttrValueDeserializer<'de> {
    type Error = AttrValueError;

    fn deserialize_any<V: serde::de::Visitor<'de>>(
        self,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        visitor
            .visit_unit::<Self::Error>()
            .map_err(|AttrValueError::MalformedValue(mut error)| {
                error.span = self.attribute.name.span;
                AttrValueError::from(error)
            })
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct enum identifier ignored_any
    }
}

/// Custom deserialize function which returns when unit specified
fn deserialize_unit_as_bool<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<bool, D::Error> {
    <() as serde::Deserialize<'de>>::deserialize(deserializer)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::SpannedIdentifier;
    use serde::Deserialize;

    #[test]
    fn entrypoint_absent() {
        let actual = FunctionAttributes::deserialize(Deserializer::new(&[])).unwrap();
        assert_eq!(actual.entrypoint, false);
    }

    #[test]
    fn entrypoint_present() {
        let attributes = vec![ast::AttributeDecl {
            name: SpannedIdentifier::new("entrypoint", SourceSpan::from(0..10)),
        }];
        let actual =
            FunctionAttributes::deserialize(Deserializer::new(attributes.as_slice())).unwrap();
        assert_eq!(actual.entrypoint, true);
    }

    #[test]
    fn no_startup_code_absent() {
        let actual = FunctionAttributes::deserialize(Deserializer::new(&[])).unwrap();
        assert_eq!(actual.no_startup_code, false);
    }

    #[test]
    fn no_startup_code_present() {
        let attributes = vec![ast::AttributeDecl {
            name: SpannedIdentifier::new("no_startup_code", SourceSpan::from(0..10)),
        }];
        let actual =
            FunctionAttributes::deserialize(Deserializer::new(attributes.as_slice())).unwrap();
        assert_eq!(actual.no_startup_code, true);
    }
}
