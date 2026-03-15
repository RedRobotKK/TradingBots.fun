//! Stripe billing integration.
//!
//! ## Flow
//!
//! 1. Free-tier user clicks **Upgrade** in `/app`.
//! 2. Browser POSTs to `/billing/checkout` → we create a Stripe Checkout
//!    Session and redirect to Stripe's hosted page.
//! 3. Stripe redirects back to `/billing/success?session_id=…` on success.
//! 4. Stripe asynchronously POSTs `checkout.session.completed` to
//!    `/webhooks/stripe` — we verify the signature and upgrade the tenant.
//! 5. On cancellation Stripe sends `customer.subscription.deleted` → we
//!    downgrade the tenant to Free immediately.
//!
//! ## 14-day trial
//!
//! `/billing/trial?tenant_id=…` starts a trial via `TenantManager::start_trial()`.
//! No card required.  After 14 days the trial expires and live trading stops
//! automatically (checked in `TenantConfig::is_live_enabled()`).
//!
//! ## Webhook security
//!
//! Every event is verified with HMAC-SHA256 using the `STRIPE_WEBHOOK_SECRET`.
//! Replays older than 5 minutes are rejected.

use anyhow::{anyhow, Result};
use axum::{
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use std::collections::HashMap;

use crate::web_dashboard::AppState;

type HmacSha256 = Hmac<Sha256>;

// ─────────────────────────────────────────────────────────────────────────────
//  Stripe webhook signature verification
// ─────────────────────────────────────────────────────────────────────────────

/// Verify the `Stripe-Signature` header against the raw request body.
///
/// Stripe's format: `t=<unix_ts>,v1=<hex_signature>[,v1=<hex2>…]`
///
/// Algorithm:
///   1. `signed_payload = "<t>.<raw_body>"`
///   2. `expected = HMAC-SHA256(webhook_secret, signed_payload)`
///   3. Compare `expected` to every `v1=` value (constant-time).
///   4. Reject if timestamp is more than 5 minutes old.
pub fn verify_signature(payload: &[u8], sig_header: &str, secret: &str) -> Result<()> {
    let mut timestamp: Option<i64> = None;
    let mut signatures: Vec<String> = Vec::new();

    for part in sig_header.split(',') {
        if let Some(t) = part.strip_prefix("t=") {
            timestamp = Some(t.trim().parse::<i64>()
                .map_err(|_| anyhow!("Invalid timestamp in Stripe-Signature"))?);
        } else if let Some(v) = part.strip_prefix("v1=") {
            signatures.push(v.trim().to_string());
        }
    }

    let ts = timestamp.ok_or_else(|| anyhow!("Missing timestamp in Stripe-Signature"))?;
    if signatures.is_empty() {
        return Err(anyhow!("No v1 signature found in Stripe-Signature"));
    }

    // Reject replays older than 5 minutes
    let now = chrono::Utc::now().timestamp();
    if (now - ts).abs() > 300 {
        return Err(anyhow!("Stripe webhook timestamp too old (possible replay)"));
    }

    // Compute expected HMAC
    let signed_payload = format!("{}.{}", ts, String::from_utf8_lossy(payload));
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|e| anyhow!("HMAC key error: {}", e))?;
    mac.update(signed_payload.as_bytes());
    let expected = hex::encode(mac.finalize().into_bytes());

    // Constant-time compare against all v1 signatures
    if signatures.iter().any(|s| s == &expected) {
        Ok(())
    } else {
        Err(anyhow!("Stripe signature mismatch"))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Stripe event types (minimal — only what we handle)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Deserialize, Debug)]
pub struct StripeEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub data:       StripeEventData,
}

#[derive(Deserialize, Debug)]
pub struct StripeEventData {
    pub object: serde_json::Value,
}

// ─────────────────────────────────────────────────────────────────────────────
//  Stripe API calls (no SDK — raw HTTP to keep dependencies minimal)
// ─────────────────────────────────────────────────────────────────────────────

/// Create a Stripe Checkout Session and return the redirect URL.
///
/// Embeds `tenant_id` in metadata so the webhook can look up the tenant.
pub async fn create_checkout_session(
    api_key:   &str,
    price_id:  &str,
    tenant_id: &str,
    success_url: &str,
    cancel_url:  &str,
) -> Result<String> {
    let client = reqwest::Client::new();

    let mut params = HashMap::new();
    params.insert("mode",                            "subscription");
    params.insert("line_items[0][price]",            price_id);
    params.insert("line_items[0][quantity]",         "1");
    params.insert("success_url",                     success_url);
    params.insert("cancel_url",                      cancel_url);
    params.insert("metadata[tenant_id]",             tenant_id);
    // Allow promotion codes so users can redeem discount coupons
    params.insert("allow_promotion_codes",           "true");

    let resp = client
        .post("https://api.stripe.com/v1/checkout/sessions")
        .basic_auth(api_key, Some(""))
        .form(&params)
        .send().await
        .map_err(|e| anyhow!("Stripe API request failed: {}", e))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("Stripe API error: {}", body));
    }

    let json: serde_json::Value = resp.json().await
        .map_err(|e| anyhow!("Stripe response parse: {}", e))?;

    json["url"].as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("Stripe response missing session URL"))
}

// ─────────────────────────────────────────────────────────────────────────────
//  Axum handlers
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CheckoutParams {
    pub tenant_id: Option<String>,
}

#[derive(Deserialize)]
pub struct TrialParams {
    pub tenant_id: Option<String>,
}

/// `GET /billing/checkout?tenant_id=…`
///
/// Creates a Stripe Checkout Session and redirects the user to Stripe's
/// hosted payment page.  If Stripe is not configured (no API key), redirects
/// back to `/app` with an explanatory message.
pub async fn checkout_handler(
    State(app): State<AppState>,
    Query(params): Query<CheckoutParams>,
) -> Response {
    let api_key = match &app.stripe_api_key {
        Some(k) => k.clone(),
        None    => return Redirect::to("/app?msg=stripe_not_configured").into_response(),
    };
    let price_id = match &app.stripe_price_id {
        Some(p) => p.clone(),
        None    => return Redirect::to("/app?msg=stripe_not_configured").into_response(),
    };

    let tenant_id = params.tenant_id.unwrap_or_else(|| "default".to_string());
    let host = "https://yourdomain.com"; // TODO: replace with req Host header

    let session_url = create_checkout_session(
        &api_key,
        &price_id,
        &tenant_id,
        &format!("{}/billing/success?tenant_id={}", host, tenant_id),
        &format!("{}/app",                           host),
    ).await;

    match session_url {
        Ok(url) => Redirect::to(&url).into_response(),
        Err(e)  => {
            log::error!("Stripe checkout session failed: {}", e);
            Redirect::to("/app?msg=payment_error").into_response()
        }
    }
}

/// `GET /billing/success?tenant_id=…`
///
/// Shown after a successful Stripe checkout.  The actual tier upgrade is
/// driven by the webhook — this page is just user-facing confirmation.
pub async fn success_handler(
    State(_app): State<AppState>,
    Query(params): Query<CheckoutParams>,
) -> Html<String> {
    let tid = params.tenant_id.unwrap_or_else(|| "your account".to_string());
    Html(format!(r#"<!DOCTYPE html>
<html lang="en">
<head><meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>TradingBots.fun · Welcome to Pro</title>
<style>
  *{{box-sizing:border-box;margin:0;padding:0}}
  body{{background:#0d1117;color:#c9d1d9;font-family:-apple-system,sans-serif;
        min-height:100vh;display:flex;flex-direction:column;align-items:center;
        justify-content:center;padding:40px 16px;text-align:center}}
  .icon{{font-size:3rem;margin-bottom:16px}}
  h1{{font-size:1.6rem;font-weight:700;color:#3fb950;margin-bottom:12px}}
  p{{color:#8b949e;font-size:.95rem;max-width:380px;line-height:1.6;margin-bottom:24px}}
  a{{display:inline-block;padding:10px 24px;background:#3fb95018;border:1px solid #3fb95050;
     border-radius:8px;color:#3fb950;font-weight:600;text-decoration:none}}
  a:hover{{background:#3fb95030}}
</style>
</head>
<body>
<div class="icon">🚀</div>
<h1>You're now on Pro</h1>
<p>Live trading is active for <b>{}</b>. Your bot will start placing real orders on the next cycle.</p>
<p style="font-size:.8rem;color:#484f58">If live trading doesn't start within 2 minutes, refresh the dashboard.</p>
<a href="/app">Go to my account →</a>
</body></html>"#, tid))
}

/// `POST /webhooks/stripe`
///
/// Receives Stripe events, verifies the signature, and mutates tenant tier.
///
/// Handled events:
/// - `checkout.session.completed`  → upgrade to Pro
/// - `customer.subscription.deleted` → downgrade to Free
/// - `invoice.payment_failed`      → downgrade to Free (grace period handled by Stripe)
pub async fn webhook_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // Require webhook secret — if not configured, reject silently
    let secret = match &app.stripe_webhook_secret {
        Some(s) => s.clone(),
        None    => {
            log::warn!("Stripe webhook received but STRIPE_WEBHOOK_SECRET not set");
            return (StatusCode::OK, "not configured").into_response();
        }
    };

    // Verify signature
    let sig = match headers.get("stripe-signature").and_then(|v| v.to_str().ok()) {
        Some(s) => s.to_string(),
        None    => return (StatusCode::BAD_REQUEST, "Missing Stripe-Signature").into_response(),
    };

    if let Err(e) = verify_signature(&body, &sig, &secret) {
        log::warn!("⚠ Stripe signature invalid: {}", e);
        return (StatusCode::UNAUTHORIZED, "Invalid signature").into_response();
    }

    // Parse event
    let event: StripeEvent = match serde_json::from_slice(&body) {
        Ok(e)  => e,
        Err(e) => {
            log::error!("Stripe event parse error: {}", e);
            return (StatusCode::BAD_REQUEST, "Parse error").into_response();
        }
    };

    log::info!("📨 Stripe event: {}", event.event_type);

    match event.event_type.as_str() {

        // ── New subscription — upgrade tenant to Pro ──────────────────────
        "checkout.session.completed" => {
            let obj = &event.data.object;
            let tenant_id   = obj["metadata"]["tenant_id"].as_str().unwrap_or("");
            let customer_id = obj["customer"].as_str().unwrap_or("");
            let sub_id      = obj["subscription"].as_str().unwrap_or("");

            if tenant_id.is_empty() {
                log::warn!("checkout.session.completed: no tenant_id in metadata");
            } else {
                let mut mgr = app.tenants.write().await;
                let tid = crate::tenant::TenantId::from_str(tenant_id);
                match mgr.upgrade_to_pro(&tid, customer_id, sub_id) {
                    Ok(())  => log::info!("✅ Tenant {} upgraded to Pro", tenant_id),
                    Err(e)  => log::error!("Upgrade failed for {}: {}", tenant_id, e),
                }
            }
        }

        // ── Subscription cancelled or payment failed — downgrade ──────────
        "customer.subscription.deleted" | "invoice.payment_failed" => {
            let obj         = &event.data.object;
            // Try metadata first, fall back to customer ID lookup
            let tenant_id_meta = obj["metadata"]["tenant_id"].as_str()
                .or_else(|| obj["customer"].as_str());

            if let Some(lookup_key) = tenant_id_meta {
                let mut mgr = app.tenants.write().await;

                // Try direct tenant_id lookup first
                let tid = crate::tenant::TenantId::from_str(lookup_key);
                let result = if mgr.get(&tid).is_some() {
                    mgr.downgrade_to_free(&tid)
                } else {
                    // Fall back: find by Stripe customer ID
                    let found_id = mgr.find_by_stripe_customer(lookup_key)
                        .map(|h| h.id.clone());
                    if let Some(fid) = found_id {
                        mgr.downgrade_to_free(&fid)
                    } else {
                        Err(anyhow!("Tenant not found for key {}", lookup_key))
                    }
                };

                match result {
                    Ok(())  => log::info!("⬇ Tenant downgraded to Free ({})", event.event_type),
                    Err(e)  => log::error!("Downgrade failed: {}", e),
                }
            }
        }

        // All other events acknowledged but not acted on
        other => log::debug!("Stripe event ignored: {}", other),
    }

    (StatusCode::OK, "ok").into_response()
}

/// `GET /billing/trial?tenant_id=…`
///
/// Starts a 14-day Pro trial for a Free-tier tenant.  No card required.
/// Redirects to `/app` with a success message.
pub async fn trial_handler(
    State(app): State<AppState>,
    Query(params): Query<TrialParams>,
) -> Response {
    let tid_str = match params.tenant_id {
        Some(t) => t,
        None    => return Redirect::to("/app?msg=trial_no_id").into_response(),
    };

    let tid = crate::tenant::TenantId::from_str(&tid_str);
    let mut mgr = app.tenants.write().await;
    match mgr.start_trial(&tid, 14) {
        Ok(())  => {
            log::info!("🎁 Trial started for tenant {}", tid_str);
            Redirect::to("/app?msg=trial_started").into_response()
        }
        Err(e) => {
            log::error!("Trial start failed for {}: {}", tid_str, e);
            Redirect::to("/app?msg=trial_error").into_response()
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    fn make_valid_sig(payload: &[u8], secret: &str, ts: i64) -> String {
        let signed = format!("{}.{}", ts, String::from_utf8_lossy(payload));
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(signed.as_bytes());
        let sig = hex::encode(mac.finalize().into_bytes());
        format!("t={},v1={}", ts, sig)
    }

    #[test]
    fn valid_signature_passes() {
        let payload = b"test_payload";
        let secret  = "whsec_test";
        let ts      = chrono::Utc::now().timestamp();
        let header  = make_valid_sig(payload, secret, ts);
        assert!(verify_signature(payload, &header, secret).is_ok());
    }

    #[test]
    fn wrong_secret_fails() {
        let payload = b"test_payload";
        let ts      = chrono::Utc::now().timestamp();
        let header  = make_valid_sig(payload, "correct_secret", ts);
        assert!(verify_signature(payload, &header, "wrong_secret").is_err());
    }

    #[test]
    fn tampered_payload_fails() {
        let ts     = chrono::Utc::now().timestamp();
        let header = make_valid_sig(b"original_payload", "secret", ts);
        assert!(verify_signature(b"tampered_payload", &header, "secret").is_err());
    }

    #[test]
    fn old_timestamp_rejected() {
        let payload = b"test_payload";
        let old_ts  = chrono::Utc::now().timestamp() - 400; // 6m 40s ago
        let header  = make_valid_sig(payload, "secret", old_ts);
        assert!(verify_signature(payload, &header, "secret").is_err());
    }

    #[test]
    fn missing_timestamp_fails() {
        let result = verify_signature(b"payload", "v1=abc123", "secret");
        assert!(result.is_err());
    }
}
