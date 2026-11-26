/// A token or id that must never be printed. It has no `Serialize` impl on purpose,
/// so it cannot be returned to the webview by accident.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The raw value. Only for request building and the credential store.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn masked(&self) -> String {
        format!("present (len={})", self.0.len())
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.masked())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_and_masked_show_only_presence_and_length() {
        let secret = Secret::new("2-FAKE-token-value");
        assert_eq!(secret.masked(), "present (len=18)");
        assert_eq!(format!("{secret:?}"), "present (len=18)");
        assert_eq!(format!("{:?}", Some(secret)), "Some(present (len=18))");
    }
}
