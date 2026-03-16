//! Transactional email — thin async wrapper around the Resend REST API.
//!
//! ## Why Resend?
//! - Single HTTPS POST, no SMTP complexity
//! - Uses the existing `reqwest` dependency — zero new crates
//! - 100 emails/day free; $20/month for 50k — fits our scale
//!
//! ## Configuration (env vars)
//! | Variable           | Example                               | Required? |
//! |--------------------|---------------------------------------|-----------|
//! | `RESEND_API_KEY`   | `re_abc123…`                          | Yes       |
//! | `EMAIL_FROM`       | `TradingBots.fun <hello@tradingbots.fun>` | No (default) |
//!
//! ## Emails sent today
//! | Trigger                        | Subject                                       |
//! |--------------------------------|-----------------------------------------------|
//! | Trial expires, 1× per user     | "Your trial ended — try Pro for $9.95"        |
//!
//! All sends are fire-and-forget; errors are logged and do not block the caller.
//! Idempotency is guaranteed by `promo_email_sent_at` in the DB — the hourly
//! job skips rows where that column is non-NULL.

use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::json;

// ─────────────────────────────────────────────────────────────────────────────

const RESEND_API: &str = "https://api.resend.com/emails";
const DEFAULT_FROM: &str = "TradingBots.fun <hello@tradingbots.fun>";

// ─────────────────────────────────────────────────────────────────────────────

/// Thin wrapper around the Resend transactional email API.
///
/// Clone-cheap: the inner `reqwest::Client` already uses an `Arc` internally.
#[derive(Clone)]
pub struct Mailer {
    client:  Client,
    api_key: String,
    from:    String,
}

impl Mailer {
    /// Construct from env-sourced values.  Returns `None` if no API key is set,
    /// so callers can gracefully degrade when email is not configured.
    pub fn new(api_key: Option<&str>, from: Option<&str>) -> Option<Self> {
        let key = api_key?;
        Some(Self {
            client:  Client::new(),
            api_key: key.to_owned(),
            from:    from.unwrap_or(DEFAULT_FROM).to_owned(),
        })
    }

    // ── Core send ────────────────────────────────────────────────────────────

    /// Send a transactional email.
    ///
    /// Errors are returned (not logged) so the caller decides how to handle them.
    pub async fn send(&self, to: &str, subject: &str, html: &str) -> Result<()> {
        let body = json!({
            "from":    self.from,
            "to":      [to],
            "subject": subject,
            "html":    html,
        });

        let resp = self.client
            .post(RESEND_API)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .context("mailer: HTTP request failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body   = resp.text().await.unwrap_or_default();
            anyhow::bail!("mailer: Resend returned {status}: {body}");
        }

        Ok(())
    }

    // ── Email templates ──────────────────────────────────────────────────────

    /// Returns the HTML body for the post-trial promo email.
    ///
    /// Arguments:
    /// - `display_name`  — first-name or wallet short (e.g. "0x1234")
    /// - `checkout_url`  — full URL to the $9.95 promo Stripe Checkout session
    ///                      (e.g. `https://tradingbots.fun/billing/checkout?promo=1&tenant_id=…`)
    pub fn trial_expiry_html(display_name: &str, checkout_url: &str) -> String {
        let name = if display_name.is_empty() { "Trader" } else { display_name };

        format!(r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Your TradingBots.fun trial ended</title>
<style>
  * {{ box-sizing: border-box; margin: 0; padding: 0; }}
  body {{
    background: #0d1117;
    color: #c9d1d9;
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif;
    font-size: 15px;
    line-height: 1.6;
  }}
  .wrap  {{ max-width: 580px; margin: 40px auto; padding: 0 20px 40px; }}
  .logo  {{ font-size: 22px; font-weight: 700; color: #e6edf3; letter-spacing: -.5px; margin-bottom: 32px; }}
  .logo span {{ color: #e6343a; }}
  h1     {{ font-size: 24px; color: #e6edf3; font-weight: 700; margin-bottom: 12px; }}
  p      {{ margin-bottom: 16px; color: #8b949e; }}
  p.hi   {{ color: #c9d1d9; font-size: 16px; }}
  .card  {{
    background: #161b22;
    border: 1px solid #30363d;
    border-radius: 10px;
    padding: 28px 32px;
    margin: 28px 0;
  }}
  .price {{
    font-size: 42px;
    font-weight: 800;
    color: #e6edf3;
    letter-spacing: -1px;
    line-height: 1;
  }}
  .price sub  {{ font-size: 16px; font-weight: 500; color: #8b949e; vertical-align: bottom; margin-left: 2px; }}
  .was  {{ font-size: 13px; color: #6e7681; text-decoration: line-through; margin-top: 4px; }}
  .badge {{
    display: inline-block;
    background: rgba(230,52,58,.15);
    color: #e6343a;
    font-size: 12px;
    font-weight: 600;
    padding: 3px 10px;
    border-radius: 20px;
    margin-bottom: 16px;
    letter-spacing: .4px;
    text-transform: uppercase;
  }}
  .feat  {{ margin-top: 18px; }}
  .feat li {{
    list-style: none;
    padding: 6px 0;
    color: #8b949e;
    border-bottom: 1px solid #21262d;
    font-size: 14px;
  }}
  .feat li:last-child {{ border-bottom: none; }}
  .feat li::before {{ content: "✓  "; color: #3fb950; font-weight: 700; }}
  .cta {{
    display: block;
    background: #e6343a;
    color: #fff !important;
    text-decoration: none;
    font-weight: 700;
    font-size: 16px;
    text-align: center;
    padding: 15px 24px;
    border-radius: 8px;
    margin-top: 28px;
    letter-spacing: .2px;
  }}
  .cta:hover {{ background: #c9282d; }}
  .fine  {{ font-size: 12px; color: #6e7681; margin-top: 8px; text-align: center; }}
  .divider {{ border: none; border-top: 1px solid #21262d; margin: 32px 0; }}
  .footer {{ font-size: 12px; color: #6e7681; line-height: 1.8; }}
  .footer a {{ color: #6e7681; }}
</style>
</head>
<body>
<div class="wrap">
  <div class="logo">Trading<span>Bots</span>.fun</div>

  <h1>Your free trial has ended</h1>
  <p class="hi">Hey {name},</p>
  <p>Your 14-day trial just wrapped up. Your bots have been paused and you're back to the free plan (2 positions max).</p>
  <p>Here's a limited offer to unlock everything again — <strong style="color:#e6edf3">for less than a coffee this month</strong>:</p>

  <div class="card">
    <span class="badge">Intro offer — first month only</span>
    <div class="price">$9.95 <sub>/ mo</sub></div>
    <div class="was">Usually $19.99/month</div>

    <ul class="feat">
      <li>Up to 6 simultaneous trading bots</li>
      <li>Long <strong>and</strong> short positions — hedge both directions</li>
      <li>Live Hyperliquid perpetuals (real capital)</li>
      <li>AI signal scoring + daily performance reports</li>
      <li>Leaderboard entry — compete for weekly prizes</li>
      <li>No ads — ever</li>
    </ul>

    <a href="{checkout_url}" class="cta">Claim $9.95 intro offer →</a>
    <p class="fine">Renews at $19.99/month. Cancel anytime.</p>
  </div>

  <p>After your first month, billing moves to the standard $19.99/month — but you can cancel at any time, no questions asked.</p>
  <p>If you have questions, just reply to this email.</p>
  <p style="color:#c9d1d9">— The TradingBots.fun team</p>

  <hr class="divider">
  <div class="footer">
    You're receiving this because you signed up for a free trial at TradingBots.fun.<br>
    <a href="https://tradingbots.fun/app/settings">Manage email preferences</a>
  </div>
</div>
</body>
</html>
"##,
            name         = html_escape(name),
            checkout_url = checkout_url,
        )
    }
}

// ─────────────────────────────────────────────────────────────────────────────

/// Minimal HTML escaping for user-supplied strings rendered into email HTML.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
     .replace('\'', "&#39;")
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mailer_requires_api_key() {
        assert!(Mailer::new(None, None).is_none());
        assert!(Mailer::new(Some("re_test_key"), None).is_some());
    }

    #[test]
    fn html_escape_sanitises_xss() {
        let out = html_escape("<script>alert('xss')</script>");
        assert!(!out.contains('<'));
        assert!(!out.contains('>'));
        assert!(!out.contains('\''));
    }

    #[test]
    fn trial_expiry_html_contains_name_and_url() {
        let html = Mailer::trial_expiry_html("Alice", "https://example.com/checkout?promo=1");
        assert!(html.contains("Alice"));
        assert!(html.contains("https://example.com/checkout?promo=1"));
        assert!(html.contains("$9.95"));
        assert!(html.contains("$19.99"));
    }

    #[test]
    fn trial_expiry_html_defaults_empty_name() {
        let html = Mailer::trial_expiry_html("", "https://example.com/");
        assert!(html.contains("Trader")); // fallback name
    }
}
