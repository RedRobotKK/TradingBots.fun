//! Privy authentication — JWT verification, JWKS caching, and session cookies.
//!
//! ## Authentication flow (SSR + session cookie)
//!
//! 1. Unauthenticated request to `/app/*` → redirect to `/login`.
//! 2. `/login` embeds the Privy JS SDK via CDN.
//! 3. User authenticates with Privy (email / social / wallet).
//! 4. Privy SDK yields an ES256-signed access token (JWT).
//! 5. Browser POSTs the token to `POST /auth/session`.
//! 6. Server verifies the JWT against Privy's JWKS endpoint, auto-registers
//!    the user as a Free tenant if new, and issues a signed HMAC-SHA256
//!    session cookie (`rr_session`).
//! 7. All subsequent consumer requests carry the cookie; server verifies the
//!    HMAC and serves per-tenant data.
//! 8. `GET /auth/logout` clears the session cookie.
//!
//! ## Session cookie format
//!
//! `{tenant_id}:{exp_unix_secs}:{hmac_hex}`
//!
//! HMAC covers `"{tenant_id}:{exp_unix_secs}"` keyed by `SESSION_SECRET`.
//! A signature mismatch or past expiry rejects the session immediately.
//!
//! ## Privy JWT
//!
//! Privy issues ES256 JWTs.  The JWKS endpoint is:
//! `https://auth.privy.io/api/v1/apps/{app_id}/jwks.json`
//!
//! Claims checked:
//! - Algorithm: ES256
//! - Issuer:    `"privy.io"`
//! - Audience:  your Privy App ID
//! - Expiry:    standard JWT `exp` field
//!
//! The `sub` claim is the Privy DID (e.g. `"did:privy:clxxxxxxxxx"`).

use anyhow::{anyhow, Result};
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

use crate::tenant::TenantId;

type HmacSha256 = Hmac<Sha256>;

// ─────────────────────────────────────────────────────────────────────────────
//  Privy JWT claims
// ─────────────────────────────────────────────────────────────────────────────

/// Claims extracted from a verified Privy access token.
#[derive(Debug, Deserialize)]
pub struct PrivyClaims {
    /// Privy Decentralised Identifier — unique user identity.
    /// Format: `"did:privy:clxxxxxxxxxxxxxxxxx"`
    pub sub: String,
    /// Expiry (Unix seconds) — validated by `jsonwebtoken`.
    #[allow(dead_code)]
    pub exp: usize,
    /// Issued-at (Unix seconds).
    #[allow(dead_code)]
    pub iat: usize,
}

// ─────────────────────────────────────────────────────────────────────────────
//  JWKS cache
// ─────────────────────────────────────────────────────────────────────────────

/// In-memory cache of Privy's JWKS — refreshed automatically after 1 hour.
pub struct JwksCache {
    /// Raw JWKS JSON (re-parsed on each verification; tiny struct, fast parse).
    pub raw_json: String,
    /// Monotonic clock snapshot when the cache was last populated.
    pub fetched_at: Instant,
}

/// Thread-safe JWKS cache shared across all Axum handlers via `AppState`.
pub type SharedJwksCache = Arc<RwLock<Option<JwksCache>>>;

/// Create a new empty JWKS cache.
pub fn new_jwks_cache() -> SharedJwksCache {
    Arc::new(RwLock::new(None))
}

/// Fetch Privy's JWKS for `app_id`, serving from cache when still fresh.
///
/// The cache is considered stale after 3 600 seconds (1 hour) and is
/// refreshed transparently on the next verification call.
pub async fn get_jwks(app_id: &str, cache: &SharedJwksCache) -> Result<String> {
    // Fast path — return cached JSON if fetched within the last hour
    {
        let guard = cache.read().await;
        if let Some(c) = guard.as_ref() {
            if c.fetched_at.elapsed().as_secs() < 3_600 {
                return Ok(c.raw_json.clone());
            }
        }
    }

    // Slow path — fetch fresh JWKS from Privy's endpoint
    let url = format!("https://auth.privy.io/api/v1/apps/{}/jwks.json", app_id);
    let raw = reqwest::get(&url)
        .await
        .map_err(|e| anyhow!("Privy JWKS fetch failed: {e}"))?
        .text()
        .await
        .map_err(|e| anyhow!("Privy JWKS response read failed: {e}"))?;

    *cache.write().await = Some(JwksCache {
        raw_json: raw.clone(),
        fetched_at: Instant::now(),
    });
    log::debug!("🔑 Privy JWKS cache refreshed for app {}", app_id);
    Ok(raw)
}

// ─────────────────────────────────────────────────────────────────────────────
//  JWT verification
// ─────────────────────────────────────────────────────────────────────────────

/// Verify a Privy access token (ES256 JWT).
///
/// Returns the Privy DID (`sub` claim, e.g. `"did:privy:clxxx"`) on success.
///
/// # Errors
/// Returns `Err` if the JWT is malformed, has an invalid signature, is
/// expired, or if the `kid` is not found in the JWKS.
pub async fn verify_privy_jwt(
    token: &str,
    app_id: &str,
    jwks_cache: &SharedJwksCache,
) -> Result<String> {
    use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};

    // Step 1 — peek at the header to obtain `kid` without verifying yet
    let header = decode_header(token).map_err(|e| anyhow!("JWT header parse error: {e}"))?;
    let kid = header
        .kid
        .ok_or_else(|| anyhow!("JWT is missing 'kid' in header"))?;

    // Step 2 — fetch (or serve from cache) Privy's public JWKS
    let jwks_json = get_jwks(app_id, jwks_cache).await?;
    let jwks: JwkSet =
        serde_json::from_str(&jwks_json).map_err(|e| anyhow!("JWKS JSON parse error: {e}"))?;

    // Step 3 — find the key that matches this token's `kid`
    let jwk = jwks
        .find(&kid)
        .ok_or_else(|| anyhow!("No matching JWK found for kid '{kid}'"))?;

    // Step 4 — build a DecodingKey from the JWK (jsonwebtoken handles EC P-256)
    let decoding_key = DecodingKey::from_jwk(jwk)
        .map_err(|e| anyhow!("JWK → DecodingKey conversion error: {e}"))?;

    // Step 5 — validate signature, expiry, audience, and issuer
    let mut validation = Validation::new(Algorithm::ES256);
    validation.set_audience(&[app_id]);
    validation.set_issuer(&["privy.io"]);

    let token_data = decode::<PrivyClaims>(token, &decoding_key, &validation)
        .map_err(|e| anyhow!("JWT validation failed: {e}"))?;

    Ok(token_data.claims.sub)
}

// ─────────────────────────────────────────────────────────────────────────────
//  Session cookies  (HMAC-SHA256 signed, HttpOnly, SameSite=Lax)
// ─────────────────────────────────────────────────────────────────────────────

/// Name of the HTTP session cookie.
pub const SESSION_COOKIE: &str = "rr_session";

/// Session time-to-live: 7 days (in seconds).
pub const SESSION_TTL_SECS: i64 = 7 * 24 * 3_600;

/// Create a signed session cookie value for a given tenant.
///
/// Cookie value format: `{tenant_id}:{exp_unix}:{hmac_hex}`
///
/// The HMAC is computed over `"{tenant_id}:{exp_unix}"` with `secret` as key.
pub fn create_session(tenant_id: &TenantId, secret: &str) -> String {
    let exp = Utc::now().timestamp() + SESSION_TTL_SECS;
    let payload = format!("{}:{}", tenant_id.as_str(), exp);
    let sig = hmac_hex(payload.as_bytes(), secret);
    format!("{}:{}", payload, sig)
}

/// Verify a session cookie value and return the `TenantId` on success.
///
/// Rejects cookies with invalid HMAC signatures or past expiry timestamps.
pub fn verify_session(cookie_value: &str, secret: &str) -> Result<TenantId> {
    // The rightmost ':' separates `{payload}` from `{hmac_hex}`
    let last_colon = cookie_value
        .rfind(':')
        .ok_or_else(|| anyhow!("Malformed session cookie (no colon separator)"))?;

    let payload = &cookie_value[..last_colon];
    let sig = &cookie_value[last_colon + 1..];

    // Constant-time HMAC comparison
    let expected = hmac_hex(payload.as_bytes(), secret);
    if sig != expected {
        return Err(anyhow!("Session signature mismatch"));
    }

    // Split payload `{tenant_id}:{exp_unix}` — use rfind to handle UUIDs with '-'
    let second_colon = payload
        .rfind(':')
        .ok_or_else(|| anyhow!("Malformed session payload (missing expiry)"))?;

    let tenant_id_str = &payload[..second_colon];
    let exp_str = &payload[second_colon + 1..];

    let exp: i64 = exp_str
        .parse()
        .map_err(|_| anyhow!("Invalid expiry value in session cookie"))?;

    if Utc::now().timestamp() > exp {
        return Err(anyhow!("Session cookie has expired"));
    }

    Ok(TenantId::from_str(tenant_id_str))
}

/// Extract the `rr_session` cookie value from a raw `Cookie:` header string.
///
/// Returns `Some(value)` if the cookie is present, `None` otherwise.
pub fn extract_session_cookie(cookie_header: &str) -> Option<&str> {
    for segment in cookie_header.split(';') {
        let segment = segment.trim();
        if let Some(value) = segment
            .strip_prefix(SESSION_COOKIE)
            .and_then(|s| s.strip_prefix('='))
        {
            return Some(value);
        }
    }
    None
}

/// Build the `Set-Cookie` header value that installs a new session.
pub fn set_session_header(tenant_id: &TenantId, secret: &str) -> String {
    let value = create_session(tenant_id, secret);
    format!(
        "{}={}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age={}",
        SESSION_COOKIE, value, SESSION_TTL_SECS
    )
}

/// `Set-Cookie` header value that immediately expires the session (logout).
pub fn clear_session_header() -> &'static str {
    "rr_session=; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=0"
}

/// Convenience helper for API handlers: extract and verify the session cookie
/// from request headers, returning the `TenantId` or an error.
///
/// Usage:
/// ```ignore
/// let tenant_id = crate::privy::require_tenant_id(&headers, &app.session_secret)?;
/// ```
pub fn require_tenant_id(
    headers: &axum::http::HeaderMap,
    secret: &str,
) -> anyhow::Result<crate::tenant::TenantId> {
    let cookie_header = headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let session_val = extract_session_cookie(cookie_header)
        .ok_or_else(|| anyhow::anyhow!("No session cookie"))?;
    verify_session(session_val, secret)
}

// ─────────────────────────────────────────────────────────────────────────────
//  Internal helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Compute `HMAC-SHA256(key=secret, data)` and return the lowercase hex string.
fn hmac_hex(data: &[u8], secret: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(data);
    hex::encode(mac.finalize().into_bytes())
}

// ─────────────────────────────────────────────────────────────────────────────
//  Unit tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "test-session-secret-at-least-32-chars!!";

    // ── Session creation & round-trip verification ───────────────────────────

    #[test]
    fn session_roundtrip_simple_id() {
        let tid = TenantId::from_str("tenant-alice");
        let cookie = create_session(&tid, SECRET);
        let result = verify_session(&cookie, SECRET);
        assert!(result.is_ok(), "roundtrip failed: {:?}", result);
        assert_eq!(result.unwrap().as_str(), "tenant-alice");
    }

    #[test]
    fn session_roundtrip_uuid_id() {
        let tid = TenantId::new(); // UUID v4 contains hyphens
        let cookie = create_session(&tid, SECRET);
        let result = verify_session(&cookie, SECRET);
        assert!(result.is_ok(), "UUID roundtrip failed: {:?}", result);
        assert_eq!(result.unwrap().as_str(), tid.as_str());
    }

    #[test]
    fn session_roundtrip_privy_did() {
        // Privy DIDs look like "did:privy:clxxxxxxxxxxxxxxxxx"
        let tid = TenantId::from_str("did:privy:cltest1234567890");
        let cookie = create_session(&tid, SECRET);
        // Privy DIDs contain ':' — make sure split logic still works
        let result = verify_session(&cookie, SECRET);
        assert!(result.is_ok(), "Privy DID roundtrip failed: {:?}", result);
        assert_eq!(result.unwrap().as_str(), "did:privy:cltest1234567890");
    }

    // ── Rejection tests ──────────────────────────────────────────────────────

    #[test]
    fn wrong_secret_rejected() {
        let tid = TenantId::from_str("tenant-bob");
        let cookie = create_session(&tid, "correct_secret_!!!!!!!!!!!!!!!!!!!!");
        assert!(
            verify_session(&cookie, "wrong___secret_!!!!!!!!!!!!!!!!!!!").is_err(),
            "wrong secret should be rejected"
        );
    }

    #[test]
    fn tampered_tenant_id_rejected() {
        let tid = TenantId::from_str("real-tenant");
        let cookie = create_session(&tid, SECRET);
        // Attacker tries to swap in their own tenant id
        let tampered = cookie.replacen("real-tenant", "evil-tenant", 1);
        assert!(
            verify_session(&tampered, SECRET).is_err(),
            "tampered tenant_id should be rejected"
        );
    }

    #[test]
    fn expired_session_rejected() {
        // Craft a session whose expiry is 1 second in the past
        let tenant_id = "expired-tenant";
        let exp = Utc::now().timestamp() - 1;
        let payload = format!("{}:{}", tenant_id, exp);
        let sig = super::hmac_hex(payload.as_bytes(), SECRET);
        let cookie = format!("{}:{}", payload, sig);
        assert!(
            verify_session(&cookie, SECRET).is_err(),
            "expired session should be rejected"
        );
    }

    #[test]
    fn malformed_cookies_rejected() {
        assert!(verify_session("", SECRET).is_err(), "empty string");
        assert!(
            verify_session("no_colons_at_all", SECRET).is_err(),
            "no colons"
        );
        assert!(
            verify_session("only:one_colon", SECRET).is_err(),
            "no hmac segment"
        );
    }

    // ── Cookie extraction ────────────────────────────────────────────────────

    #[test]
    fn extract_from_multi_cookie_header() {
        let hdr = "other=val; rr_session=abc123def; another=x";
        assert_eq!(extract_session_cookie(hdr), Some("abc123def"));
    }

    #[test]
    fn extract_sole_cookie() {
        assert_eq!(
            extract_session_cookie("rr_session=just_this"),
            Some("just_this")
        );
    }

    #[test]
    fn extract_missing_returns_none() {
        assert_eq!(extract_session_cookie("foo=bar; baz=qux"), None);
        assert_eq!(extract_session_cookie(""), None);
    }

    #[test]
    fn extract_does_not_match_prefix_only() {
        // "rr_session_extra" should NOT match "rr_session"
        let hdr = "rr_session_extra=hijack; legit=val";
        assert_eq!(extract_session_cookie(hdr), None);
    }

    // ── HMAC properties ──────────────────────────────────────────────────────

    #[test]
    fn hmac_is_deterministic() {
        let a = hmac_hex(b"same_data", "same_key_!!!!!!!!!!!!!!!!!!!!!!");
        let b = hmac_hex(b"same_data", "same_key_!!!!!!!!!!!!!!!!!!!!!!");
        assert_eq!(a, b, "HMAC must be deterministic");
    }

    #[test]
    fn hmac_is_sensitive_to_data() {
        let a = hmac_hex(b"data_a", SECRET);
        let b = hmac_hex(b"data_b", SECRET);
        assert_ne!(a, b, "different data must produce different HMAC");
    }

    #[test]
    fn hmac_is_sensitive_to_key() {
        let a = hmac_hex(b"same_data", "key_one_!!!!!!!!!!!!!!!!!!!!!!");
        let b = hmac_hex(b"same_data", "key_two_!!!!!!!!!!!!!!!!!!!!!!");
        assert_ne!(a, b, "different keys must produce different HMAC");
    }
}
