use miette::SourceSpan;
use serde::ser::SerializeTuple;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Spanned<T> {
    pub data: T,
    pub span: SourceSpan,
}

impl<T> Spanned<T> {
    pub fn new(value: impl Into<T>, span: SourceSpan) -> Self {
        Spanned {
            data: value.into(),
            span,
        }
    }
}

impl<T: serde::Serialize> serde::Serialize for Spanned<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        struct SourceSpanSer(SourceSpan);
        impl serde::Serialize for SourceSpanSer {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                span::serialize(&self.0, serializer)
            }
        }
        let mut ser = serializer.serialize_tuple(2)?;
        ser.serialize_element(&self.data)?;
        ser.serialize_element(&SourceSpanSer(self.span))?;
        ser.end()
    }
}

impl<'de, T: serde::Deserialize<'de>> serde::Deserialize<'de> for Spanned<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error>
where {
        struct SourceSpanDe(SourceSpan);
        impl<'de> serde::Deserialize<'de> for SourceSpanDe {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error>
where {
                Ok(SourceSpanDe(span::deserialize(deserializer)?))
            }
        }
        let (data, SourceSpanDe(span)) =
            <(T, SourceSpanDe) as serde::Deserialize>::deserialize(deserializer)?;
        Ok(Spanned { data, span })
    }
}

pub mod span {
    use miette::{SourceOffset, SourceSpan};
    use serde::ser::SerializeTuple;

    #[must_use]
    pub fn span_between(start: SourceSpan, end: SourceSpan) -> SourceSpan {
        SourceSpan::from(
            start.offset().min(end.offset())
                ..(start.offset() + start.len()).max(end.offset() + end.len()),
        )
    }

    pub fn serialize<S: serde::Serializer>(
        value: &SourceSpan,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut ser = serializer.serialize_tuple(2)?;
        ser.serialize_element(&value.offset())?;
        ser.serialize_element(&value.len())?;
        ser.end()
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<SourceSpan, D::Error> {
        let (offset, len) = <(usize, usize) as serde::Deserialize>::deserialize(deserializer)?;
        Ok(SourceSpan::new(SourceOffset::from(offset), len))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miette::SourceOffset;

    #[test]
    fn new_stores_value_and_span() {
        let span = SourceSpan::new(SourceOffset::from(5), 10);
        let s: Spanned<u32> = Spanned::new(42u32, span);
        assert_eq!(s.data, 42);
        assert_eq!(s.span.offset(), 5);
        assert_eq!(s.span.len(), 10);
    }

    #[test]
    fn new_converts_value_via_into() {
        let span = SourceSpan::new(SourceOffset::from(0), 3);
        let s: Spanned<String> = Spanned::new("hi", span);
        assert_eq!(s.data, "hi");
    }

    #[test]
    fn serde_round_trip() {
        let span = SourceSpan::new(SourceOffset::from(7), 4);
        let original: Spanned<u32> = Spanned::new(99u32, span);
        let serialized = ron::to_string(&original).unwrap();
        let deserialized: Spanned<u32> = ron::from_str(&serialized).unwrap();
        assert_eq!(deserialized.data, original.data);
        assert_eq!(deserialized.span.offset(), original.span.offset());
        assert_eq!(deserialized.span.len(), original.span.len());
    }

    #[test]
    fn span_between_spans_from_start_of_first_to_end_of_second() {
        let start = SourceSpan::new(SourceOffset::from(3), 4);
        let end = SourceSpan::new(SourceOffset::from(10), 5);
        let result = span::span_between(start, end);
        assert_eq!(result.offset(), 3);
        assert_eq!(result.len(), 12);
    }

    #[test]
    fn span_between_with_overlapping_spans() {
        let start = SourceSpan::new(SourceOffset::from(0), 10);
        let end = SourceSpan::new(SourceOffset::from(5), 3);
        let result = span::span_between(start, end);
        assert_eq!(result.offset(), 0);
        assert_eq!(result.len(), 10);
    }

    #[test]
    fn equality_considers_value_and_span() {
        let span_a = SourceSpan::new(SourceOffset::from(0), 1);
        let span_b = SourceSpan::new(SourceOffset::from(1), 1);
        let a: Spanned<u32> = Spanned::new(1u32, span_a);
        let b: Spanned<u32> = Spanned::new(1u32, span_b);
        let c: Spanned<u32> = Spanned::new(1u32, span_a);
        assert_ne!(a, b);
        assert_eq!(a, c);
    }
}
