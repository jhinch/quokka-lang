use miette::{SourceOffset, SourceSpan};
use serde::ser::SerializeTuple;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Spanned<T> {
    pub value: T,
    pub span: SourceSpan,
}

impl<T> Spanned<T> {
    pub fn new(value: impl Into<T>, span: SourceSpan) -> Self {
        Spanned {
            value: value.into(),
            span,
        }
    }
}

impl<'de, T: serde::Deserialize<'de>> serde::Deserialize<'de> for Spanned<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let (value, offset, len) =
            <(T, usize, usize) as serde::Deserialize<'de>>::deserialize(deserializer)?;
        Ok(Spanned {
            value,
            span: SourceSpan::new(SourceOffset::from(offset), len),
        })
    }
}

impl<T: serde::Serialize> serde::Serialize for Spanned<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut seq = serializer.serialize_tuple(3)?;
        seq.serialize_element(&self.value)?;
        seq.serialize_element(&self.span.offset())?;
        seq.serialize_element(&self.span.len())?;
        seq.end()
    }
}
