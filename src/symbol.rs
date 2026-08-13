use lasso::{MemoryLimits, Spur, ThreadedRodeo};
use std::fmt;
use std::sync::OnceLock;

fn symbol_table() -> &'static ThreadedRodeo {
    static SYMBOL_TABLE: OnceLock<ThreadedRodeo> = OnceLock::new();
    SYMBOL_TABLE.get_or_init(ThreadedRodeo::new)
}

pub fn set_memory_limit(n: usize) {
    symbol_table().set_memory_limits(MemoryLimits::for_memory_usage(n));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(into = "&str")]
pub struct Symbol(Spur);

impl Default for Symbol {
    fn default() -> Self {
        Symbol::from_static("")
    }
}

impl Symbol {
    #[must_use]
    pub fn new(value: &str) -> Symbol {
        Symbol(symbol_table().get_or_intern(value))
    }

    #[must_use]
    pub fn from_static(value: &'static str) -> Symbol {
        Symbol(symbol_table().get_or_intern_static(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &'static str {
        symbol_table().resolve(&self.0)
    }
}

impl<T: AsRef<str>> From<T> for Symbol {
    fn from(value: T) -> Self {
        Symbol::new(value.as_ref())
    }
}

impl From<Symbol> for &'static str {
    fn from(value: Symbol) -> Self {
        value.as_str()
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
