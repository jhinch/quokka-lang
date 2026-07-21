/// Similar to a Result except:
/// - when no value is present, there is 1 or more errors
/// - when a value is present, there is 0 or more errors
#[derive(Debug)]
pub struct MultiResult<A> {
    value: Option<A>,
    errors: MultiError,
}

impl<A> MultiResult<A> {
    pub fn into_result(self) -> Result<A, MultiError> {
        match self.value {
            Some(success) => Ok(success),
            None => Err(self.errors),
        }
    }

    pub fn into_parts(self) -> (Option<A>, MultiError) {
        (self.value, self.errors)
    }

    pub fn value(&self) -> Option<&A> {
        self.value.as_ref()
    }

    pub fn errors(&self) -> Option<&MultiError> {
        if self.errors.errors.is_empty() {
            None
        } else {
            Some(&self.errors)
        }
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.errors.is_empty()
    }
}

#[derive(Debug, Default)]
pub struct MultiResultBuilder {
    errors: Vec<Box<dyn miette::Diagnostic + Send + Sync + 'static>>,
}

impl MultiResultBuilder {
    pub fn take(&mut self) -> Self {
        std::mem::take(self)
    }

    pub fn finish<A>(self, value: A) -> MultiResult<A> {
        MultiResult {
            value: Some(value),
            errors: MultiError {
                errors: self.errors,
            },
        }
    }

    pub fn add_error(&mut self, error: impl miette::Diagnostic + Send + Sync + 'static) {
        self.errors.push(Box::new(error));
    }

    pub fn finish_with_error<A>(
        mut self,
        error: impl miette::Diagnostic + Send + Sync + 'static,
    ) -> MultiResult<A> {
        self.errors.push(Box::new(error));
        MultiResult {
            value: None,
            errors: MultiError {
                errors: self.errors,
            },
        }
    }
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("Multiple errors found")]
pub struct MultiError {
    #[related]
    pub errors: Vec<Box<dyn miette::Diagnostic + Send + Sync + 'static>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, thiserror::Error, miette::Diagnostic)]
    #[error("test error")]
    struct TestError;

    #[test]
    fn finish_produces_value_with_no_errors() {
        let builder = MultiResultBuilder::default();
        let result: MultiResult<i32> = builder.finish(42);
        assert_eq!(result.value(), Some(&42));
        assert!(!result.has_errors());
        assert!(result.errors().is_none());
    }

    #[test]
    fn finish_with_error_produces_no_value_with_one_error() {
        let builder = MultiResultBuilder::default();
        let result: MultiResult<i32> = builder.finish_with_error(TestError);
        assert!(result.value().is_none());
        assert!(result.has_errors());
        assert!(result.errors().is_some());
        assert_eq!(result.errors().unwrap().errors.len(), 1);
    }

    #[test]
    fn add_error_then_finish_gives_value_and_error() {
        let mut builder = MultiResultBuilder::default();
        builder.add_error(TestError);
        let result = builder.finish(99);
        assert_eq!(result.value(), Some(&99));
        assert!(result.has_errors());
        assert_eq!(result.errors().unwrap().errors.len(), 1);
    }

    #[test]
    fn multiple_add_error_accumulates() {
        let mut builder = MultiResultBuilder::default();
        builder.add_error(TestError);
        builder.add_error(TestError);
        let result: MultiResult<i32> = builder.finish_with_error(TestError);
        assert!(result.value().is_none());
        assert_eq!(result.errors().unwrap().errors.len(), 3);
    }

    #[test]
    fn take_resets_builder() {
        let mut builder = MultiResultBuilder::default();
        builder.add_error(TestError);
        let taken = builder.take();
        let result_original: MultiResult<i32> = builder.finish(1);
        assert!(!result_original.has_errors());
        let result_taken: MultiResult<i32> = taken.finish(2);
        assert!(result_taken.has_errors());
    }

    #[test]
    fn into_result_ok_when_value_present() {
        let result: MultiResult<i32> = MultiResultBuilder::default().finish(7);
        assert!(result.into_result().is_ok());
    }

    #[test]
    fn into_result_err_when_no_value() {
        let result: MultiResult<i32> = MultiResultBuilder::default().finish_with_error(TestError);
        assert!(result.into_result().is_err());
    }

    #[test]
    fn into_parts_returns_value_and_errors() {
        let mut builder = MultiResultBuilder::default();
        builder.add_error(TestError);
        let result = builder.finish(5);
        let (value, errors) = result.into_parts();
        assert_eq!(value, Some(5));
        assert_eq!(errors.errors.len(), 1);
    }

    #[test]
    fn errors_returns_none_when_no_errors() {
        let result: MultiResult<i32> = MultiResultBuilder::default().finish(0);
        assert!(result.errors().is_none());
    }
}
