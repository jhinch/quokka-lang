use crate::multi::MultiResultBuilder;
use miette::SourceSpan;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum PreviewFeature {
    /// example preview feature only used for tests. Provides no meaningful functionally
    Example,
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

    pub fn is_enabled(&self, feature: PreviewFeature) -> bool {
        self.features.contains(&feature)
    }
}
