use miette::SourceSpan;

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
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
