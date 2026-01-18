//! Lightweight input validation helpers. Keep logic minimal and deterministic.

use crate::CoreError;
use crate::Slug;

/// Validate an original URL. We keep this intentionally light to avoid heavy
/// parsing crates: ensure http/https scheme and a reasonable length.
pub fn validate_original_url(s: &str) -> Result<(), CoreError> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(CoreError::InvalidUrl("empty".into()));
    }
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err(CoreError::InvalidUrl("must start with http:// or https://".into()));
    }
    if trimmed.len() > 2048 {
        return Err(CoreError::InvalidUrl("too long".into()));
    }
    Ok(())
}

/// Validate a custom slug string using the same rules as `Slug::new`.
pub fn validate_custom_slug(s: &str) -> Result<Slug, CoreError> {
    Slug::new(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_validation_basic() {
        assert!(validate_original_url("https://example.com").is_ok());
        assert!(validate_original_url("http://example.com").is_ok());
        assert!(validate_original_url("").is_err());
        assert!(validate_original_url("ftp://example.com").is_err());
    }

    #[test]
    fn slug_validation_delegates() {
        assert!(validate_custom_slug("abc-123").is_ok());
        assert!(validate_custom_slug("").is_err());
        assert!(validate_custom_slug("bad/char").is_err());
    }
}
