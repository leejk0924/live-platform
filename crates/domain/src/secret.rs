use std::fmt;

/// Wraps a value that must never reach logs or responses. It has no `Display`
/// and no serde impls; reading it takes an explicit [`Secret::expose`].
#[derive(Clone)]
pub struct Secret<T>(T);

impl<T> Secret<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> &T {
        &self.0
    }
}

impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([REDACTED])")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_shows_the_value() {
        let secret = Secret::new(String::from("hunter2"));
        assert_eq!(format!("{secret:?}"), "Secret([REDACTED])");
        assert!(!format!("{secret:#?}").contains("hunter2"));
        assert!(!format!("{:?}", Some(&secret)).contains("hunter2"));
        assert_eq!(secret.expose(), "hunter2");
    }
}
