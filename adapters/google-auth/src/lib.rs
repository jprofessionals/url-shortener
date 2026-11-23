//! Verifies Google ID tokens against Google's JWKS.

use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedUser {
    pub email: String,
    pub sub: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuthError {
    #[error("missing or malformed token")]
    Malformed,
    #[error("invalid token payload: {0}")]
    InvalidPayload(&'static str),
    #[error("signature invalid")]
    SignatureInvalid,
    #[error("token expired")]
    Expired,
    #[error("audience mismatch")]
    BadAudience,
    #[error("email not verified")]
    EmailNotVerified,
    #[error("domain not allowed")]
    DomainNotAllowed,
    #[error("network or jwks fetch error")]
    Network,
}

#[derive(Debug, Deserialize)]
struct Claims {
    sub: String,
    email: Option<String>,
    email_verified: Option<bool>,
    hd: Option<String>,
}

/// Verify a Google Workspace user's signed ID token.
pub async fn verify_async(
    id_token: &str,
    expected_aud: &str,
    allowed_domain: &str,
) -> Result<VerifiedUser, AuthError> {
    let header = decode_header(id_token).map_err(|_| AuthError::Malformed)?;
    if header.alg != Algorithm::RS256 {
        // Only RS256 supported for Google ID tokens
        return Err(AuthError::Malformed);
    }
    let kid = header.kid.ok_or(AuthError::Malformed)?;
    let key = jwks_get_key_async(&kid)
        .await
        .map_err(|_| AuthError::Network)?;

    verify_signed_claims(id_token, &key, expected_aud, allowed_domain)
}

fn verify_signed_claims(
    id_token: &str,
    key: &DecodingKey,
    expected_aud: &str,
    allowed_domain: &str,
) -> Result<VerifiedUser, AuthError> {
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    validation.set_audience(&[expected_aud]);
    validation.set_issuer(&["accounts.google.com", "https://accounts.google.com"]);

    let token_data = decode::<Claims>(id_token, key, &validation).map_err(|e| match e.kind() {
        jsonwebtoken::errors::ErrorKind::InvalidToken
        | jsonwebtoken::errors::ErrorKind::InvalidSignature => AuthError::SignatureInvalid,
        jsonwebtoken::errors::ErrorKind::ExpiredSignature => AuthError::Expired,
        jsonwebtoken::errors::ErrorKind::InvalidAudience => AuthError::BadAudience,
        _ => AuthError::Malformed,
    })?;

    let claims = token_data.claims;
    let email = claims.email.ok_or(AuthError::InvalidPayload("email"))?;
    if claims.email_verified != Some(true) {
        return Err(AuthError::EmailNotVerified);
    }

    if !claims
        .hd
        .is_some_and(|hd| hd.eq_ignore_ascii_case(allowed_domain))
    {
        return Err(AuthError::DomainNotAllowed);
    }

    Ok(VerifiedUser {
        email,
        sub: claims.sub,
    })
}

// ---- JWKS cache & fetch ----

const JWKS_URL: &str = "https://www.googleapis.com/oauth2/v3/certs";
const JWKS_TTL: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

#[derive(Debug, Deserialize)]
struct Jwk {
    kid: String,
    kty: String,
    #[allow(dead_code)] // Part of JWKS structure, used for deserialization
    alg: Option<String>,
    n: Option<String>,
    e: Option<String>,
}

struct JwksCache {
    fetched_at: SystemTime,
    keys: HashMap<String, DecodingKey>,
}

static CACHE: LazyLock<Mutex<JwksCache>> = LazyLock::new(|| {
    Mutex::new(JwksCache {
        fetched_at: UNIX_EPOCH,
        keys: HashMap::new(),
    })
});

async fn jwks_get_key_async(kid: &str) -> Result<DecodingKey, ()> {
    // First, check cache without blocking async work under the mutex.
    {
        let cache = CACHE.lock().unwrap();
        let now = SystemTime::now();
        let fresh = cache.fetched_at + JWKS_TTL > now;
        if fresh {
            if let Some(k) = cache.keys.get(kid) {
                return Ok(k.clone());
            }
        }
    }

    // Fetch outside the lock
    let new_map = fetch_jwks_map_async().await.map_err(|_| ())?;
    // Update cache
    let mut cache = CACHE.lock().unwrap();
    cache.keys = new_map;
    cache.fetched_at = SystemTime::now();
    cache.keys.get(kid).cloned().ok_or(())
}

async fn fetch_jwks_map_async() -> Result<HashMap<String, DecodingKey>, reqwest::Error> {
    let resp = reqwest::Client::new().get(JWKS_URL).send().await?;
    let jwks: Jwks = resp.json().await?;
    let mut map = HashMap::new();
    for k in jwks.keys.into_iter() {
        if k.kty == "RSA" {
            if let (Some(n), Some(e)) = (k.n.as_deref(), k.e.as_deref()) {
                if let Ok(key) = DecodingKey::from_rsa_components(n, e) {
                    map.insert(k.kid, key);
                }
            }
        }
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_lc_rs::{
        rand::SystemRandom,
        rsa::{KeyPair, KeySize},
        signature::{KeyPair as _, RSA_PKCS1_SHA256},
    };
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use serde_json::{json, Value};

    fn signed_token(key: &KeyPair, claims: &Value) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","kid":"test-key"}"#);
        let claims = URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).unwrap());
        let signing_input = format!("{header}.{claims}");
        let mut signature = vec![0; key.public_key().modulus_len()];
        key.sign(
            &RSA_PKCS1_SHA256,
            &SystemRandom::new(),
            signing_input.as_bytes(),
            &mut signature,
        )
        .unwrap();
        format!("{signing_input}.{}", URL_SAFE_NO_PAD.encode(signature))
    }

    #[test]
    fn requires_signed_google_workspace_claims() {
        let key = KeyPair::generate(KeySize::Rsa2048).unwrap();
        let public_key = key.public_key();
        let n = URL_SAFE_NO_PAD.encode(public_key.modulus().big_endian_without_leading_zero());
        let e = URL_SAFE_NO_PAD.encode(public_key.exponent().big_endian_without_leading_zero());
        let decoding_key = DecodingKey::from_rsa_components(&n, &e).unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let valid = json!({
            "sub": "test-user",
            "aud": "test-client",
            "iss": "https://accounts.google.com",
            "exp": now + 300,
            "email": "user@example.test",
            "email_verified": true,
            "hd": "example.test"
        });
        let verify = |claims: &Value| {
            verify_signed_claims(
                &signed_token(&key, claims),
                &decoding_key,
                "test-client",
                "example.test",
            )
        };

        assert_eq!(verify(&valid).unwrap().email, "user@example.test");

        for (claim, value, expected) in [
            ("aud", json!("other-client"), AuthError::BadAudience),
            ("iss", json!("https://example.test"), AuthError::Malformed),
            ("exp", json!(now - 300), AuthError::Expired),
            ("hd", json!("other.test"), AuthError::DomainNotAllowed),
            ("email_verified", json!(false), AuthError::EmailNotVerified),
        ] {
            let mut claims = valid.clone();
            claims[claim] = value;
            assert_eq!(verify(&claims), Err(expected), "{claim}");
        }

        for claim in ["aud", "iss", "exp", "sub", "hd"] {
            let mut claims = valid.clone();
            claims.as_object_mut().unwrap().remove(claim);
            let expected = if claim == "hd" {
                AuthError::DomainNotAllowed
            } else {
                AuthError::Malformed
            };
            assert_eq!(verify(&claims), Err(expected), "{claim}");
        }
    }
}
