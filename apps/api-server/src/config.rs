//! Centralized configuration for api-server.
//!
//! All environment variables are loaded and validated at startup to fail fast
//! on misconfiguration rather than at request time.

use axum::http::HeaderValue;
use std::env;
use std::fmt;
use std::path::PathBuf;

/// Authentication provider mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthProvider {
    /// Debug mode: accepts X-Debug-User header (DO NOT USE IN PRODUCTION)
    None,
    /// Google OIDC: verifies Google ID tokens
    Google,
}

impl AuthProvider {
    fn from_str(s: &str) -> Self {
        if s.eq_ignore_ascii_case("google") {
            Self::Google
        } else {
            Self::None
        }
    }
}

/// Storage backend provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageProvider {
    /// In-memory storage (data lost on restart)
    Memory,
    /// SQLite file-based storage
    Sqlite,
}

impl StorageProvider {
    fn from_str(s: &str) -> Self {
        if s.eq_ignore_ascii_case("sqlite") {
            Self::Sqlite
        } else {
            Self::Memory
        }
    }
}

/// Log output format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogFormat {
    Pretty,
    Json,
}

impl LogFormat {
    fn from_str(s: &str) -> Self {
        if s.eq_ignore_ascii_case("json") {
            Self::Json
        } else {
            Self::Pretty
        }
    }
}

/// Configuration error.
#[derive(Debug)]
pub struct ConfigError {
    pub field: &'static str,
    pub message: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Configuration error for {}: {}", self.field, self.message)
    }
}

impl std::error::Error for ConfigError {}

/// Server configuration loaded from environment variables.
///
/// All fields are validated at construction time.
#[derive(Debug, Clone)]
pub struct Config {
    /// Server port (default: 3001)
    pub port: u16,
    /// Authentication provider
    pub auth_provider: AuthProvider,
    /// Allowed domain for authentication (required for Google auth)
    pub allowed_domain: Option<String>,
    /// Google OAuth client ID (required for Google auth)
    pub google_oauth_client_id: Option<String>,
    /// Whether to skip Google signature verification (dev only)
    pub insecure_skip_signature: bool,
    /// CORS allow origin
    pub cors_allow_origin: HeaderValue,
    /// Storage provider
    pub storage_provider: StorageProvider,
    /// SQLite database path (when using sqlite storage)
    #[allow(dead_code)] // For future use with SQLite adapter config
    pub db_path: Option<PathBuf>,
    /// Log format
    pub log_format: LogFormat,
    /// Custom shortlink domain for generated URLs
    pub shortlink_domain: Option<String>,
}

impl Config {
    /// Load and validate configuration from environment variables.
    ///
    /// Fails fast on invalid configuration.
    pub fn from_env() -> Result<Self, ConfigError> {
        // Port
        let port = env::var("PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(3001);

        // Auth provider
        let auth_provider = AuthProvider::from_str(
            &env::var("AUTH_PROVIDER").unwrap_or_else(|_| "none".into()),
        );

        // Allowed domain
        let allowed_domain = env::var("ALLOWED_DOMAIN").ok();

        // Google OAuth client ID
        let google_oauth_client_id = env::var("GOOGLE_OAUTH_CLIENT_ID").ok();

        // Validate: Google auth requires both ALLOWED_DOMAIN and GOOGLE_OAUTH_CLIENT_ID
        if auth_provider == AuthProvider::Google {
            if allowed_domain.is_none() {
                return Err(ConfigError {
                    field: "ALLOWED_DOMAIN",
                    message: "Required when AUTH_PROVIDER=google".into(),
                });
            }
            if google_oauth_client_id.is_none() {
                return Err(ConfigError {
                    field: "GOOGLE_OAUTH_CLIENT_ID",
                    message: "Required when AUTH_PROVIDER=google".into(),
                });
            }
        }

        // Insecure skip signature
        let skip_sig = env::var("GOOGLE_AUTH_INSECURE_SKIP_SIGNATURE").unwrap_or_default();
        let insecure_skip_signature = matches!(
            skip_sig.to_lowercase().as_str(),
            "1" | "true" | "yes"
        );

        // CORS allow origin
        let cors_origin_str = env::var("CORS_ALLOW_ORIGIN").unwrap_or_else(|_| "*".into());
        let cors_allow_origin = if cors_origin_str == "*" {
            HeaderValue::from_static("*")
        } else {
            HeaderValue::from_str(&cors_origin_str).map_err(|e| ConfigError {
                field: "CORS_ALLOW_ORIGIN",
                message: format!("Invalid header value '{}': {}", cors_origin_str, e),
            })?
        };

        // Storage provider
        let storage_provider = StorageProvider::from_str(
            &env::var("STORAGE_PROVIDER").unwrap_or_else(|_| "sqlite".into()),
        );

        // DB path (for sqlite)
        let db_path = env::var("DB_PATH").ok().map(PathBuf::from);

        // Log format
        let log_format =
            LogFormat::from_str(&env::var("LOG_FORMAT").unwrap_or_else(|_| "pretty".into()));

        // Shortlink domain
        let shortlink_domain = env::var("SHORTLINK_DOMAIN").ok().filter(|s| !s.is_empty());

        Ok(Self {
            port,
            auth_provider,
            allowed_domain,
            google_oauth_client_id,
            insecure_skip_signature,
            cors_allow_origin,
            storage_provider,
            db_path,
            log_format,
            shortlink_domain,
        })
    }

    /// Log warnings about insecure configuration.
    pub fn warn_if_insecure(&self) {
        if self.auth_provider == AuthProvider::None {
            tracing::warn!(
                "AUTH_PROVIDER=none: Using debug authentication via X-Debug-User header. \
                 DO NOT USE IN PRODUCTION."
            );
            if self.allowed_domain.is_none() {
                tracing::warn!(
                    "ALLOWED_DOMAIN not set: Any email in X-Debug-User header will be accepted. \
                     Set ALLOWED_DOMAIN for domain restriction."
                );
            }
        }
        if self.insecure_skip_signature {
            tracing::warn!(
                "GOOGLE_AUTH_INSECURE_SKIP_SIGNATURE is set: ID token signature verification \
                 is DISABLED. DO NOT USE IN PRODUCTION."
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_provider_parsing() {
        assert_eq!(AuthProvider::from_str("none"), AuthProvider::None);
        assert_eq!(AuthProvider::from_str("NONE"), AuthProvider::None);
        assert_eq!(AuthProvider::from_str("google"), AuthProvider::Google);
        assert_eq!(AuthProvider::from_str("GOOGLE"), AuthProvider::Google);
        assert_eq!(AuthProvider::from_str("anything"), AuthProvider::None);
    }

    #[test]
    fn storage_provider_parsing() {
        assert_eq!(StorageProvider::from_str("memory"), StorageProvider::Memory);
        assert_eq!(StorageProvider::from_str("sqlite"), StorageProvider::Sqlite);
        assert_eq!(StorageProvider::from_str("SQLITE"), StorageProvider::Sqlite);
        assert_eq!(StorageProvider::from_str("anything"), StorageProvider::Memory);
    }

    #[test]
    fn log_format_parsing() {
        assert_eq!(LogFormat::from_str("pretty"), LogFormat::Pretty);
        assert_eq!(LogFormat::from_str("json"), LogFormat::Json);
        assert_eq!(LogFormat::from_str("JSON"), LogFormat::Json);
        assert_eq!(LogFormat::from_str("anything"), LogFormat::Pretty);
    }
}
