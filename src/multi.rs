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
