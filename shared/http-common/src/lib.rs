//! Shared HTTP utilities for the URL shortener workspace.
//!
//! Provides common response builders, validation, and utility functions
//! used across api-server, lambda-admin, and lambda-redirect.

use chrono::{DateTime, SecondsFormat, Utc};
use std::time::SystemTime;

// ============================================================================
// JSON Response Helpers (framework-agnostic)
// ============================================================================

/// Create a structured error JSON with a default message based on the code.
///
/// Returns: `{"error": {"code": "<code>", "message": "<default message>"}}`
pub fn json_err(code: &str) -> serde_json::Value {
    let message = match code {
        "not_found" => "Resource not found",
        "bad_request" => "Bad request",
        "invalid_slug" => "Invalid slug format",
        "unauthorized" => "Authentication required",
        "forbidden" => "Access denied",
        "conflict" => "Resource already exists",
        "error" | "internal" => "Internal server error",
        _ => code, // Fallback to code as message for unknown codes
    };
    serde_json::json!({"error": {"code": code, "message": message}})
}

/// Create a structured error JSON with a custom message.
///
/// Returns: `{"error": {"code": "<code>", "message": "<message>"}}`
pub fn json_error_with_message(code: &str, message: &str) -> serde_json::Value {
    serde_json::json!({"error": {"code": code, "message": message}})
}

// ============================================================================
// Validation Helpers
// ============================================================================

/// Validate a custom alias for shortlinks.
///
/// Rules:
/// - Length must be 3-32 characters
/// - Only ASCII alphanumeric, hyphen (-), and underscore (_) allowed
/// - This matches the domain Slug validation
pub fn is_valid_alias(s: &str) -> bool {
    if s.len() < 3 || s.len() > 32 {
        return false;
    }
    s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

// ============================================================================
// URL Building
// ============================================================================

/// Build a short URL from a host and slug.
///
/// If `SHORTLINK_DOMAIN` env var is set and non-empty, uses that as the base.
/// Otherwise falls back to `https://{host}/{slug}` or `/{slug}` if host is empty.
pub fn build_short_url_from_host(host: &str, slug: &str) -> String {
    if let Ok(dom) = std::env::var("SHORTLINK_DOMAIN") {
        if !dom.is_empty() {
            return format!("{}/{}", dom.trim_end_matches('/'), slug);
        }
    }
    if host.is_empty() {
        format!("/{}", slug)
    } else {
        format!("https://{}/{}", host, slug)
    }
}

// ============================================================================
// Time Utilities
// ============================================================================

/// Convert SystemTime to RFC3339 string (seconds precision, UTC).
pub fn system_time_to_rfc3339(t: SystemTime) -> String {
    let dt: DateTime<Utc> = t.into();
    dt.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Parse an RFC3339 string to SystemTime.
///
/// Returns an error if the string is not a valid RFC3339 timestamp.
pub fn rfc3339_to_system_time(s: &str) -> Result<SystemTime, chrono::ParseError> {
    let dt = DateTime::parse_from_rfc3339(s)?;
    Ok(dt.with_timezone(&Utc).into())
}

/// Parse an RFC3339 string to SystemTime (alias for ergonomic use).
pub fn parse_rfc3339(s: &str) -> Result<SystemTime, chrono::ParseError> {
    rfc3339_to_system_time(s)
}

/// Generate a unique ID string.
///
/// Combines timestamp with random bytes for uniqueness.
/// Format: `{timestamp_hex}_{random_hex}` (e.g., "18d4f1234_a3b2c1d4")
pub fn generate_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    // Simple random component using time-based seed
    let random: u32 = ((timestamp ^ 0xDEAD_BEEF) as u32)
        .wrapping_mul(1103515245)
        .wrapping_add(12345);

    format!("{:x}_{:08x}", timestamp, random)
}

// ============================================================================
// Query Parsing
// ============================================================================

/// Parse a `limit` query parameter from a query string.
///
/// Returns `Some(n)` if `limit=n` is found and `n` is in range 1-500.
/// Returns `None` otherwise.
pub fn parse_limit_query(query: Option<&str>) -> Option<usize> {
    let q = query?;
    for pair in q.split('&') {
        let mut it = pair.splitn(2, '=');
        let key = it.next()?;
        if key == "limit" {
            if let Some(val) = it.next() {
                if let Ok(n) = val.parse::<usize>() {
                    if (1..=500).contains(&n) {
                        return Some(n);
                    }
                }
            }
        }
    }
    None
}

/// Parse a named query parameter from a query string.
///
/// Returns `Some(value)` if the parameter is found, `None` otherwise.
/// URL-decodes the value.
pub fn parse_query_param(query: Option<&str>, name: &str) -> Option<String> {
    let q = query?;
    for pair in q.split('&') {
        let mut it = pair.splitn(2, '=');
        let key = it.next()?;
        if key == name {
            if let Some(val) = it.next() {
                // Basic URL decoding for common cases
                let decoded = val.replace("%40", "@").replace("%20", " ");
                return Some(decoded);
            }
        }
    }
    None
}

// ============================================================================
// Lambda HTTP Helpers (feature-gated)
// ============================================================================

#[cfg(feature = "lambda")]
pub mod lambda {
    //! Lambda-specific HTTP response builders using `lambda_http` types.

    use lambda_http::{Body, Response};

    /// Build an HTTP response with optional header and JSON body.
    ///
    /// # Panics
    /// Panics if JSON serialization or response construction fails (should not happen
    /// for well-formed JSON values).
    pub fn resp(
        status: u16,
        header: Option<(&str, String)>,
        body_json: Option<serde_json::Value>,
    ) -> Response<Body> {
        let mut rb = Response::builder().status(status);
        if let Some((k, v)) = header {
            rb = rb.header(k, v);
        }
        if let Some(val) = body_json {
            rb.header("content-type", "application/json")
                .body(Body::Text(
                    serde_json::to_string(&val).expect("JSON value serialization"),
                ))
                .expect("response body construction")
        } else {
            rb.body(Body::Empty)
                .expect("empty response body construction")
        }
    }

    /// Build an error response with status code and structured error body.
    pub fn resp_with_error(status: u16, code: &str, message: &str) -> Response<Body> {
        let body = crate::json_error_with_message(code, message);
        resp(status, None, Some(body))
    }

    /// Add CORS headers to a response.
    ///
    /// Uses `CORS_ALLOW_ORIGIN` env var, defaulting to `*`.
    pub fn with_cors(mut resp: Response<Body>) -> Response<Body> {
        use http::header::{HeaderName, HeaderValue};
        let headers = resp.headers_mut();
        let allow_origin =
            std::env::var("CORS_ALLOW_ORIGIN").unwrap_or_else(|_| "*".to_string());
        headers.insert(
            HeaderName::from_static("access-control-allow-origin"),
            HeaderValue::from_str(&allow_origin).unwrap_or(HeaderValue::from_static("*")),
        );
        headers.insert(
            HeaderName::from_static("access-control-allow-headers"),
            HeaderValue::from_static("authorization, content-type"),
        );
        headers.insert(
            HeaderName::from_static("access-control-allow-methods"),
            HeaderValue::from_static("OPTIONS, GET, POST, PATCH, DELETE"),
        );
        resp
    }

    /// Extract the Host header value from a Lambda request.
    pub fn get_host(req: &lambda_http::Request) -> &str {
        req.headers()
            .get("host")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_err() {
        let err = json_err("not_found");
        assert_eq!(err, serde_json::json!({"error": {"code": "not_found", "message": "Resource not found"}}));

        // Unknown code falls back to code as message
        let err = json_err("custom_error");
        assert_eq!(err, serde_json::json!({"error": {"code": "custom_error", "message": "custom_error"}}));
    }

    #[test]
    fn test_json_error_with_message() {
        let err = json_error_with_message("bad_request", "Invalid input");
        assert_eq!(
            err,
            serde_json::json!({"error": {"code": "bad_request", "message": "Invalid input"}})
        );
    }

    #[test]
    fn test_is_valid_alias() {
        assert!(is_valid_alias("abc123"));
        assert!(is_valid_alias("A0zZ9"));
        assert!(is_valid_alias("my-slug")); // hyphen allowed
        assert!(is_valid_alias("my_slug")); // underscore allowed
        assert!(is_valid_alias("mix-ed_123")); // mixed
        assert!(!is_valid_alias("ab")); // too short
        assert!(!is_valid_alias(&"a".repeat(33))); // too long
        assert!(!is_valid_alias("bad!slug")); // special chars not allowed
        assert!(!is_valid_alias("has space")); // spaces not allowed
    }

    #[test]
    fn test_parse_limit_query() {
        assert_eq!(parse_limit_query(Some("limit=1")), Some(1));
        assert_eq!(parse_limit_query(Some("limit=500")), Some(500));
        assert_eq!(parse_limit_query(Some("limit=0")), None);
        assert_eq!(parse_limit_query(Some("limit=501")), None);
        assert_eq!(parse_limit_query(Some("page_token=x&limit=42")), Some(42));
        assert_eq!(parse_limit_query(None), None);
    }

    #[test]
    fn test_parse_query_param() {
        assert_eq!(parse_query_param(Some("foo=bar"), "foo"), Some("bar".to_string()));
        assert_eq!(parse_query_param(Some("created_by=user%40example.com"), "created_by"), Some("user@example.com".to_string()));
        assert_eq!(parse_query_param(Some("limit=10&created_by=test%40test.com"), "created_by"), Some("test@test.com".to_string()));
        assert_eq!(parse_query_param(Some("foo=bar"), "missing"), None);
        assert_eq!(parse_query_param(None, "foo"), None);
    }

    #[test]
    fn test_build_short_url_from_host() {
        // Without SHORTLINK_DOMAIN set
        std::env::remove_var("SHORTLINK_DOMAIN");
        assert_eq!(build_short_url_from_host("example.com", "abc"), "https://example.com/abc");
        assert_eq!(build_short_url_from_host("", "abc"), "/abc");
    }
}
