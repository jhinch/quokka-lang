use crate::multi::MultiResultBuilder;
use miette::SourceSpan;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum PreviewFeature {
    /// example preview feature only used for tests. Provides no meaningful functionally
    ExamplePreview,
    /// enable use of array types and index access expressions on local variables at runtime.
    LocalArray,
    /// enable use of field access expressions on local variables at runtime.
    LocalFieldAccess,
    /// enable ASM block expressions
    InlineAsm,
    /// enable custom entrypoint attribute support
    CustomEntrypoint,
    /// enable no startup code augmentation of entrypoint
    NoStartupCode,
    /// enable use of the sub-byte unsigned integer types (u2, u4)
    NarrowInt,
    /// enable use of `loop` expressions inside constant declarations
    ConstLoop,
    /// enable Aggressive Dead Code Elimination (ADCE) optimization pass on MIR
    Adce,
    /// enable Sparse Conditional Constant Propagation (SCCP) optimization pass on MIR
    Sccp,
    /// enable prune (nop and basic block) optimization pass on MIR
    Prune,
    /// disable tree shake (functions and globals) optimization pass on MIR
    NoTreeShake,
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

impl PreviewFeature {
    /// preview features that opt in to an optimization pass on MIR.
    #[must_use]
    pub fn optimization_features() -> &'static [PreviewFeature] {
        &[
            PreviewFeature::Adce,
            PreviewFeature::Sccp,
            PreviewFeature::Prune,
        ]
    }

    /// preview features that opt out of a default MIR optimization behavior.
    #[must_use]
    pub fn optimization_opt_out_features() -> &'static [PreviewFeature] {
        &[PreviewFeature::NoTreeShake]
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

#[derive(Clone, Default)]
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
        let set: PreviewFeatureSet = [
            PreviewFeature::ExamplePreview,
            PreviewFeature::NoStartupCode,
        ]
        .into_iter()
        .collect();
        assert!(set.is_enabled(PreviewFeature::ExamplePreview));
        assert!(set.is_enabled(PreviewFeature::NoStartupCode));
        assert!(!set.is_enabled(PreviewFeature::CustomEntrypoint));
    }

    #[test]
    fn extend_adds_features_without_duplicates() {
        let mut set = PreviewFeatureSet::default();
        set.enable(PreviewFeature::ExamplePreview);
        set.extend([
            PreviewFeature::ExamplePreview,
            PreviewFeature::NoStartupCode,
        ]);
        assert_eq!(set.features.len(), 2);
        assert!(set.is_enabled(PreviewFeature::ExamplePreview));
        assert!(set.is_enabled(PreviewFeature::NoStartupCode));
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

    #[test]
    fn optimization_features_and_opt_out_features_are_disjoint() {
        for feature in PreviewFeature::optimization_features() {
            assert!(!PreviewFeature::optimization_opt_out_features().contains(feature));
        }
    }
}
