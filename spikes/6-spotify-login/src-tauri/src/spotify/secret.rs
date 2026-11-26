//! A string that must never be printed.

/// Holds a cookie or token. It has no `Display` or `Serialize`, and `Debug` is
/// redacted, so it only leaves through [`Secret::expose`].
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_hides_the_value() {
        let shown = format!("{:?}", Some(Secret::new("FAKE-VALUE")));
        assert_eq!(shown, "Some(Secret(***))");
        assert_eq!(Secret::new("FAKE-VALUE").expose(), "FAKE-VALUE");
    }
}
