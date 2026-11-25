//! Loader diagnostics: the C++ loaders emit `LOG_DBERROR` /
//! `LOG_DBERRFIX` lines and fix up dirty rows in place. Rust
//! loaders here return those messages instead of logging (this
//! crate has no logger); the binary logs them at boot, and tests
//! can assert on them. Nothing is silently dropped or clamped
//! without a [`LoadReport`] entry.

/// Accumulated loader diagnostics, in row order.
#[derive(Clone, Debug, Default)]
pub struct LoadReport {
    pub errors: Vec<String>,
}

impl LoadReport {
    pub fn error(&mut self, message: String) {
        self.errors.push(message);
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.errors.len()
    }
}
