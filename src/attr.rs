use crate::ast;
use crate::source::span::span_between;
use miette::SourceSpan;
use serde::de::value::{StrDeserializer, U32Deserializer, UnitDeserializer};
use std::fmt;
use std::marker::PhantomData;

#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FnDeclAttributes {
    #[serde(default)]
    pub entrypoint: bool,
    #[serde(default)]
    pub no_startup_code: bool,
    #[serde(default)]
    pub inline: InlineKind,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InlineKind {
    #[default]
    Auto,
    Never,
    Always,
    Expect,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConstDeclAttributes {
    #[serde(default)]
    section: RomSectionKind,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RomSectionKind {
    #[default]
    Auto,
    RomX,
    Rom0,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalDeclAttributes {
    #[serde(default)]
    section: RamSectionKind,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RamSectionKind {
    #[default]
    Auto,
    WRam0,
    VRam,
    HRam,
    WRamX,
    SRam,
    Oam,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructDeclAttributes {}

#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldDeclAttributes {}

macro_rules! try_from_attributes_impl {
    ($name:ident) => {
        impl TryFrom<&[ast::AttributeDecl]> for $name {
            type Error = Error;

            fn try_from(value: &[ast::AttributeDecl]) -> Result<Self, Self::Error> {
                <$name as serde::Deserialize>::deserialize(Deserializer::new(value))
            }
        }
    };
}

try_from_attributes_impl!(FnDeclAttributes);
try_from_attributes_impl!(ConstDeclAttributes);
try_from_attributes_impl!(GlobalDeclAttributes);
try_from_attributes_impl!(StructDeclAttributes);
try_from_attributes_impl!(FieldDeclAttributes);

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
                error.span = attributes.iter().map(|attr| attr.span).reduce(span_between);
                Error::from(error)
            }
            Error::Duplicate(mut error) => {
                if let Some(defined_index) = attributes
                    .iter()
                    .position(|decl| decl.name.data.value == error.name)
                {
                    error.defined = Some(attributes[defined_index].span);
                    error.redefined = attributes
                        .iter()
                        .skip(defined_index + 1)
                        .find(|decl| decl.name.data.value == error.name)
                        .map(|decl| decl.span);
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

impl AttrValueDeserializer<'_> {
    fn populate_span_on_error(
        &self,
        AttrValueError::MalformedValue(mut error): AttrValueError,
    ) -> AttrValueError {
        error.span = self
            .attribute
            .value
            .as_ref()
            .map(|value| value.span)
            .unwrap_or(self.attribute.span);
        AttrValueError::from(error)
    }
}

impl<'de> serde::Deserializer<'de> for AttrValueDeserializer<'de> {
    type Error = AttrValueError;

    fn deserialize_any<V: serde::de::Visitor<'de>>(
        self,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        let result = match self.attribute.value.as_ref().map(|value| &value.data) {
            None => visitor.visit_unit::<Self::Error>(),
            Some(ast::AttributeValueData::Ident(ident)) => {
                visitor.visit_borrowed_str(ident.value.as_str())
            }
            Some(ast::AttributeValueData::Integer(int)) => visitor.visit_u32(int.value),
        };
        result.map_err(|error| self.populate_span_on_error(error))
    }

    fn deserialize_bool<V: serde::de::Visitor<'de>>(
        self,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        if self.attribute.value.is_some() {
            self.deserialize_any(visitor)
        } else {
            visitor
                .visit_bool(true)
                .map_err(|error| self.populate_span_on_error(error))
        }
    }

    fn deserialize_enum<V: serde::de::Visitor<'de>>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        visitor.visit_enum(self)
    }

    serde::forward_to_deserialize_any! {
        i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct identifier ignored_any
    }
}

impl<'de> serde::de::EnumAccess<'de> for AttrValueDeserializer<'de> {
    type Error = AttrValueError;
    type Variant = Self;

    fn variant_seed<V: serde::de::DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> Result<(V::Value, Self::Variant), Self::Error> {
        let value = match self.attribute.value.as_ref().map(|value| &value.data) {
            None => seed.deserialize(UnitDeserializer::<AttrValueError>::new()),
            Some(ast::AttributeValueData::Ident(ident)) => {
                seed.deserialize(StrDeserializer::new(ident.value.as_str()))
            }
            Some(ast::AttributeValueData::Integer(int)) => {
                seed.deserialize(U32Deserializer::new(int.value))
            }
        }?;
        Ok((value, self))
    }
}

impl<'de> serde::de::VariantAccess<'de> for AttrValueDeserializer<'de> {
    type Error = AttrValueError;

    fn unit_variant(self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn newtype_variant_seed<T: serde::de::DeserializeSeed<'de>>(
        self,
        seed: T,
    ) -> Result<T::Value, Self::Error> {
        seed.deserialize(UnitDeserializer::new())
    }

    fn tuple_variant<V: serde::de::Visitor<'de>>(
        self,
        _: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        visitor.visit_unit()
    }

    fn struct_variant<V: serde::de::Visitor<'de>>(
        self,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        visitor.visit_unit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::SpannedIdentifier;
    use serde::Deserialize;

    #[test]
    fn entrypoint_absent() {
        let actual = FnDeclAttributes::deserialize(Deserializer::new(&[])).unwrap();
        assert_eq!(actual.entrypoint, false);
    }

    #[test]
    fn entrypoint_present() {
        let attributes = vec![ast::AttributeDecl {
            name: SpannedIdentifier::new("entrypoint", SourceSpan::from(2..12)),
            span: SourceSpan::from(0..13),
            value: None,
        }];
        let actual =
            FnDeclAttributes::deserialize(Deserializer::new(attributes.as_slice())).unwrap();
        assert_eq!(actual.entrypoint, true);
    }

    #[test]
    fn no_startup_code_absent() {
        let actual = FnDeclAttributes::deserialize(Deserializer::new(&[])).unwrap();
        assert_eq!(actual.no_startup_code, false);
    }

    #[test]
    fn no_startup_code_present() {
        let attributes = vec![ast::AttributeDecl {
            name: SpannedIdentifier::new("no_startup_code", SourceSpan::from(2..17)),
            span: SourceSpan::from(0..18),
            value: None,
        }];
        let actual =
            FnDeclAttributes::deserialize(Deserializer::new(attributes.as_slice())).unwrap();
        assert_eq!(actual.no_startup_code, true);
    }

    #[test]
    fn inline_absent_defaults_to_auto() {
        let actual = FnDeclAttributes::deserialize(Deserializer::new(&[])).unwrap();
        assert_eq!(actual.inline, InlineKind::Auto);
    }

    #[test]
    fn inline_with_ident_value() {
        let attributes = vec![ast::AttributeDecl {
            name: SpannedIdentifier::new("inline", SourceSpan::from(2..8)),
            span: SourceSpan::from(0..20),
            value: Some(ast::AttributeValue {
                span: SourceSpan::from(11..16),
                data: ast::AttributeValueData::Ident(ast::Identifier::from("never")),
            }),
        }];
        let actual =
            FnDeclAttributes::deserialize(Deserializer::new(attributes.as_slice())).unwrap();
        assert_eq!(actual.inline, InlineKind::Never);
    }

    #[test]
    fn inline_with_unknown_variant_is_malformed() {
        let attributes = vec![ast::AttributeDecl {
            name: SpannedIdentifier::new("inline", SourceSpan::from(2..8)),
            span: SourceSpan::from(0..25),
            value: Some(ast::AttributeValue {
                span: SourceSpan::from(11..24),
                data: ast::AttributeValueData::Ident(ast::Identifier::from("not_a_variant")),
            }),
        }];
        let err =
            FnDeclAttributes::deserialize(Deserializer::new(attributes.as_slice())).unwrap_err();
        match err {
            Error::MalformedValue(inner) => assert!(inner.msg.contains("not_a_variant")),
            other => panic!("expected Error::MalformedValue, got {other:?}"),
        }
    }

    #[test]
    fn unknown_attribute_reports_name_and_span() {
        let attributes = vec![ast::AttributeDecl {
            name: SpannedIdentifier::new("bogus", SourceSpan::from(2..7)),
            span: SourceSpan::from(0..8),
            value: None,
        }];
        let err =
            FnDeclAttributes::deserialize(Deserializer::new(attributes.as_slice())).unwrap_err();
        match err {
            Error::Unknown(inner) => {
                assert_eq!(inner.name, "bogus");
                assert_eq!(inner.span.offset(), 2);
                assert_eq!(inner.span.len(), 5);
            }
            other => panic!("expected Error::Unknown, got {other:?}"),
        }
    }

    #[test]
    fn duplicate_attribute_reports_both_spans() {
        let attributes = vec![
            ast::AttributeDecl {
                name: SpannedIdentifier::new("entrypoint", SourceSpan::from(2..12)),
                span: SourceSpan::from(0..13),
                value: None,
            },
            ast::AttributeDecl {
                name: SpannedIdentifier::new("entrypoint", SourceSpan::from(20..30)),
                span: SourceSpan::from(18..31),
                value: None,
            },
        ];
        let err =
            FnDeclAttributes::deserialize(Deserializer::new(attributes.as_slice())).unwrap_err();
        match err {
            Error::Duplicate(inner) => {
                assert_eq!(inner.name, "entrypoint");
                assert_eq!(inner.defined.map(|s| s.offset()), Some(0));
                assert_eq!(inner.redefined.map(|s| s.offset()), Some(18));
            }
            other => panic!("expected Error::Duplicate, got {other:?}"),
        }
    }

    #[test]
    fn try_from_attribute_decls_delegates_to_deserializer() {
        let attributes = vec![ast::AttributeDecl {
            name: SpannedIdentifier::new("entrypoint", SourceSpan::from(2..12)),
            span: SourceSpan::from(0..13),
            value: None,
        }];
        let actual = FnDeclAttributes::try_from(attributes.as_slice()).unwrap();
        assert!(actual.entrypoint);
    }

    #[test]
    fn try_from_attribute_decls_propagates_error() {
        let attributes = vec![ast::AttributeDecl {
            name: SpannedIdentifier::new("bogus", SourceSpan::from(2..7)),
            span: SourceSpan::from(0..8),
            value: None,
        }];
        let result = FnDeclAttributes::try_from(attributes.as_slice());
        assert!(matches!(result, Err(Error::Unknown(_))));
    }
}
