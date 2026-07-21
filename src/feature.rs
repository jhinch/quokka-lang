use crate::multi::MultiResultBuilder;
use miette::SourceSpan;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum PreviewFeature {
    /// example preview feature only used for tests. Provides no meaningful functionally
    ExamplePreview,
    /// enable use of arrays in types and expressions
    Array,
    /// enable use of fields in expressions
    FieldAccess,
    /// enable function calls in expressions
    FunctionCall,
    /// enable loop block expressions
    Loop,
    /// enable if else expressions
    IfElse,
    /// enable ASM block expressions
    InlineAsm,
    /// enable custom entrypoint attribute support
    CustomEntrypoint,
    /// enable no startup code augmentation of entrypoint
    NoStartupCode,
}

impl fmt::Display for PreviewFeature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            clap::ValueEnum::to_possible_value(self).unwrap().get_name()
        )
    }
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("Preview feature error: use --preview {feature}")]
#[diagnostic()]
pub struct PreviewFeatureError {
    #[label("requires preview feature")]
    pub span: SourceSpan,
    pub feature: PreviewFeature,
}

#[derive(Default)]
pub struct PreviewFeatureSet {
    features: Vec<PreviewFeature>,
}

impl FromIterator<PreviewFeature> for PreviewFeatureSet {
    fn from_iter<T: IntoIterator<Item = PreviewFeature>>(iter: T) -> Self {
        let mut features = PreviewFeatureSet::default();
        features.extend(iter);
        features
    }
}

impl IntoIterator for PreviewFeatureSet {
    type Item = PreviewFeature;
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.features.into_iter()
    }
}

impl Extend<PreviewFeature> for PreviewFeatureSet {
    fn extend<T: IntoIterator<Item = PreviewFeature>>(&mut self, iter: T) {
        for feature in iter {
            self.enable(feature);
        }
    }
}

impl PreviewFeatureSet {
    pub fn enable(&mut self, feature: PreviewFeature) {
        if !self.features.contains(&feature) {
            self.features.push(feature);
        }
    }

    pub fn require_enabled(
        &self,
        feature: PreviewFeature,
        span: SourceSpan,
    ) -> Result<(), PreviewFeatureError> {
        if self.features.contains(&feature) {
            Ok(())
        } else {
            Err(PreviewFeatureError { span, feature })
        }
    }

    pub fn check_enabled(
        &self,
        feature: PreviewFeature,
        span: SourceSpan,
        result_builder: &mut MultiResultBuilder,
    ) {
        if !self.features.contains(&feature) {
            result_builder.add_error(PreviewFeatureError { span, feature });
        }
    }

    #[must_use]
    pub fn is_enabled(&self, feature: PreviewFeature) -> bool {
        self.features.contains(&feature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::multi::MultiResultBuilder;
    use miette::SourceSpan;

    fn dummy_span() -> SourceSpan {
        SourceSpan::new(0.into(), 0)
    }

    #[test]
    fn enable_adds_feature() {
        let mut set = PreviewFeatureSet::default();
        assert!(!set.is_enabled(PreviewFeature::ExamplePreview));
        set.enable(PreviewFeature::ExamplePreview);
        assert!(set.is_enabled(PreviewFeature::ExamplePreview));
    }

    #[test]
    fn enable_is_idempotent() {
        let mut set = PreviewFeatureSet::default();
        set.enable(PreviewFeature::ExamplePreview);
        set.enable(PreviewFeature::ExamplePreview);
        assert_eq!(
            set.features.len(),
            1,
            "internal list should not grow beyond one entry"
        );
    }

    #[test]
    fn enable_does_not_affect_other_features() {
        let mut set = PreviewFeatureSet::default();
        set.enable(PreviewFeature::ExamplePreview);
        for feature in <PreviewFeature as clap::ValueEnum>::value_variants()
            .iter()
            .copied()
        {
            if feature != PreviewFeature::ExamplePreview {
                assert!(!set.is_enabled(feature));
            }
        }
    }

    #[test]
    fn require_enabled_ok_when_feature_is_present() {
        let mut set = PreviewFeatureSet::default();
        set.enable(PreviewFeature::ExamplePreview);
        assert!(
            set.require_enabled(PreviewFeature::ExamplePreview, dummy_span())
                .is_ok()
        );
    }

    #[test]
    fn require_enabled_err_when_feature_is_absent() {
        let set = PreviewFeatureSet::default();
        let result = set.require_enabled(PreviewFeature::ExamplePreview, dummy_span());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().feature, PreviewFeature::ExamplePreview);
    }

    #[test]
    fn check_enabled_adds_no_error_when_feature_present() {
        let mut set = PreviewFeatureSet::default();
        set.enable(PreviewFeature::ExamplePreview);
        let mut builder = MultiResultBuilder::default();
        set.check_enabled(PreviewFeature::ExamplePreview, dummy_span(), &mut builder);
        let result = builder.finish(());
        assert!(!result.has_errors());
    }

    #[test]
    fn check_enabled_adds_error_when_feature_absent() {
        let set = PreviewFeatureSet::default();
        let mut builder = MultiResultBuilder::default();
        set.check_enabled(PreviewFeature::ExamplePreview, dummy_span(), &mut builder);
        let result = builder.finish(());
        assert!(result.has_errors());
    }

    #[test]
    fn from_iter_enables_all_features() {
        let set: PreviewFeatureSet = [PreviewFeature::Array, PreviewFeature::FieldAccess]
            .into_iter()
            .collect();
        assert!(set.is_enabled(PreviewFeature::Array));
        assert!(set.is_enabled(PreviewFeature::FieldAccess));
        assert!(!set.is_enabled(PreviewFeature::Loop));
    }

    #[test]
    fn extend_adds_features_without_duplicates() {
        let mut set = PreviewFeatureSet::default();
        set.enable(PreviewFeature::Array);
        set.extend([PreviewFeature::Array, PreviewFeature::Loop]);
        assert_eq!(set.features.len(), 2);
        assert!(set.is_enabled(PreviewFeature::Array));
        assert!(set.is_enabled(PreviewFeature::Loop));
    }

    #[test]
    fn display_uses_kebab_case_name() {
        assert_eq!(
            PreviewFeature::ExamplePreview.to_string(),
            "example-preview"
        );
    }

    #[test]
    fn into_iter_empty_set_yields_nothing() {
        let set = PreviewFeatureSet::default();
        assert_eq!(set.into_iter().count(), 0);
    }
}
