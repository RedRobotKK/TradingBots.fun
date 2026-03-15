//! Conversion-funnel event tracking.
//!
//! Every meaningful step from anonymous landing-page visit → Pro upgrade is
//! recorded in the `funnel_events` table.  Events fire from:
//!
//! - Axum handlers (server-side: PAGE_VIEW, AUTH_SUCCESS, TRIAL_START, …)
//! - A tiny `<script>` injected into the landing page and /login
//!   (client-side: LOGIN_CLICK, AD_IMPRESSION, AD_CLICK) that POSTs to
//!   `POST /api/funnel` — see `funnel_event_handler` in web_dashboard.rs.
//!
//! ## Graceful degradation
//!
//! All functions accept `Option<&SharedDb>` and silently no-op when the
//! database is unavailable.  The trading bot and auth flows are never blocked
//! by analytics failures.
//!
//! ## Privacy
//!
//! - `anon_id` is a client-generated uuid4 stored in a first-party cookie.
//!   It is **never** linked to third-party identity graphs.
//! - No PII (email, wallet address) is stored in `funnel_events`.
//!   Tenant resolution uses only the internal UUID FK.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::db::SharedDb;
use crate::tenant::TenantId;

// ─────────────────────────────────────────────────────────────────────────────

/// All supported funnel step identifiers.
///
/// Must stay in sync with the CHECK constraint in `002_funnel_events.sql`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FunnelEvent {
    /// Anonymous landing-page visit (server-rendered).
    PageView,
    /// User clicked the "Connect Wallet" / "Sign In" button.
    LoginClick,
    /// Privy authentication completed successfully.
    AuthSuccess,
    /// New tenant registered; 14-day trial clock started.
    TrialStart,
    /// Onboarding T&C wall cleared.
    TermsAccepted,
    /// HL wallet address linked in Settings.
    WalletLinked,
    /// Tenant opened their first paper/live position.
    FirstPosition,
    /// "Upgrade to Pro" button clicked (intent signal).
    UpgradeClick,
    /// Stripe checkout session created (stronger intent signal).
    CheckoutStarted,
    /// Stripe `invoice.paid` webhook → subscription active.
    Upgraded,
    /// Trial ended without conversion (written by a nightly batch job).
    TrialExpired,
    /// Pro subscription cancelled via Stripe webhook.
    Churned,
    /// Ad creative rendered in the UI.
    AdImpression,
    /// Ad creative clicked.
    AdClick,
}

impl FunnelEvent {
    pub fn as_str(&self) -> &'static str {
        match self {
            FunnelEvent::PageView         => "PAGE_VIEW",
            FunnelEvent::LoginClick       => "LOGIN_CLICK",
            FunnelEvent::AuthSuccess      => "AUTH_SUCCESS",
            FunnelEvent::TrialStart       => "TRIAL_START",
            FunnelEvent::TermsAccepted    => "TERMS_ACCEPTED",
            FunnelEvent::WalletLinked     => "WALLET_LINKED",
            FunnelEvent::FirstPosition    => "FIRST_POSITION",
            FunnelEvent::UpgradeClick     => "UPGRADE_CLICK",
            FunnelEvent::CheckoutStarted  => "CHECKOUT_STARTED",
            FunnelEvent::Upgraded         => "UPGRADED",
            FunnelEvent::TrialExpired     => "TRIAL_EXPIRED",
            FunnelEvent::Churned          => "CHURNED",
            FunnelEvent::AdImpression     => "AD_IMPRESSION",
            FunnelEvent::AdClick          => "AD_CLICK",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────

/// Fire a funnel event.  Never panics; errors are logged and discarded.
///
/// # Arguments
/// * `db`        – shared DB pool (no-op if `None`)
/// * `event`     – which step occurred
/// * `anon_id`   – uuid4 from the visitor cookie (empty string if unknown)
/// * `tenant_id` – `Some` once the user is authenticated, `None` for pre-auth
/// * `props`     – optional JSONB context (UTM params, ad network, CPM, …)
pub async fn record(
    db:        &Option<SharedDb>,
    event:     FunnelEvent,
    anon_id:   &str,
    tenant_id: Option<&TenantId>,
    props:     Option<Value>,
) {
    let pool = match db {
        Some(p) => p,
        None    => return, // no DB — silently skip
    };

    let tid = tenant_id.map(|t| {
        uuid::Uuid::parse_str(t.as_str()).ok()
    }).flatten();

    // Use query() (not query!()) to avoid compile-time DB schema checks —
    // the table is created at runtime via migrations, not at compile time.
    let result = sqlx::query(
        "INSERT INTO funnel_events (anon_id, tenant_id, event_type, properties) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(anon_id)
    .bind(tid)
    .bind(event.as_str())
    .bind(props)
    .execute(pool.pool())   // pool() exposes the inner &PgPool which implements Executor
    .await;

    if let Err(e) = result {
        log::warn!("funnel::record({}) failed: {}", event.as_str(), e);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Convenience helpers used in web_dashboard.rs handlers
// ─────────────────────────────────────────────────────────────────────────────

/// Record a landing-page view with optional UTM/referrer context.
pub async fn page_view(
    db:      &Option<SharedDb>,
    anon_id: &str,
    path:    &str,
    referrer: Option<&str>,
    utm_source: Option<&str>,
    utm_campaign: Option<&str>,
) {
    let props = json!({
        "path":         path,
        "referrer":     referrer,
        "utm_source":   utm_source,
        "utm_campaign": utm_campaign,
    });
    record(db, FunnelEvent::PageView, anon_id, None, Some(props)).await;
}

/// Record a successful Privy login + optional TRIAL_START for new users.
pub async fn auth_success(
    db:        &Option<SharedDb>,
    anon_id:   &str,
    tenant_id: &TenantId,
    is_new:    bool,
) {
    record(db, FunnelEvent::AuthSuccess, anon_id, Some(tenant_id), None).await;
    if is_new {
        record(db, FunnelEvent::TrialStart, anon_id, Some(tenant_id), Some(json!({
            "trial_days": 14,
        }))).await;
    }
}

/// Record an ad impression with network + CPM metadata.
///
/// `network`  – e.g. `"coinzilla"`, `"bitmedia"`, `"adsterra"`
/// `ad_unit`  – e.g. `"banner_300x250"`, `"sticky_footer"`
/// `cpm_usd`  – agreed/estimated CPM for this placement (used in ad_revenue_daily view)
pub async fn ad_impression(
    db:        &Option<SharedDb>,
    anon_id:   &str,
    tenant_id: Option<&TenantId>,
    network:   &str,
    ad_unit:   &str,
    cpm_usd:   f64,
) {
    let props = json!({
        "network": network,
        "ad_unit": ad_unit,
        "cpm_usd": cpm_usd,
    });
    record(db, FunnelEvent::AdImpression, anon_id, tenant_id, Some(props)).await;
}

// ─────────────────────────────────────────────────────────────────────────────
//  Inline JavaScript snippet — embed in landing page + /login
// ─────────────────────────────────────────────────────────────────────────────

/// Returns a `<script>` block that:
///   1. Reads / creates an `anon_id` uuid4 in localStorage.
///   2. Sends it as a cookie on every request (picked up server-side).
///   3. POSTs `LOGIN_CLICK` when the "Connect Wallet" button is clicked.
///   4. Fires `AD_IMPRESSION` / `AD_CLICK` for any ad container with
///      `data-ad-network` and `data-ad-cpm` attributes.
pub fn client_tracking_script() -> &'static str {
    r#"<script>
// ── RedRobot funnel tracking (first-party, no cookies sold to third parties) ──
(function() {
  // Initialise or read the anonymous visitor ID (uuid4 in localStorage)
  let aid = localStorage.getItem('rr_anon_id');
  if (!aid) {
    aid = ([1e7]+-1e3+-4e3+-8e3+-1e11).replace(/[018]/g, c =>
      (c ^ crypto.getRandomValues(new Uint8Array(1))[0] & 15 >> c / 4).toString(16));
    localStorage.setItem('rr_anon_id', aid);
  }
  // Write as a first-party session cookie so server-side handlers can read it
  document.cookie = 'rr_anon_id=' + aid + '; path=/; SameSite=Lax';

  // Fire a funnel event via the lightweight POST endpoint
  function fire(event_type, extra) {
    navigator.sendBeacon('/api/funnel', JSON.stringify(
      Object.assign({ event_type, anon_id: aid }, extra || {})
    ));
  }

  // LOGIN_CLICK — any element with data-funnel="login_click"
  document.querySelectorAll('[data-funnel="login_click"]').forEach(el => {
    el.addEventListener('click', () => fire('LOGIN_CLICK'));
  });

  // UPGRADE_CLICK — any element with data-funnel="upgrade_click"
  document.querySelectorAll('[data-funnel="upgrade_click"]').forEach(el => {
    el.addEventListener('click', () => fire('UPGRADE_CLICK'));
  });

  // AD tracking — fire AD_IMPRESSION for each ad container in the viewport
  if ('IntersectionObserver' in window) {
    const obs = new IntersectionObserver(entries => {
      entries.forEach(e => {
        if (e.isIntersecting) {
          const el = e.target;
          fire('AD_IMPRESSION', {
            network: el.dataset.adNetwork || 'unknown',
            ad_unit: el.dataset.adUnit   || 'unknown',
            cpm_usd: parseFloat(el.dataset.adCpm) || 0,
          });
          obs.unobserve(el); // only fire once per element per page load
        }
      });
    }, { threshold: 0.5 });
    document.querySelectorAll('[data-ad-network]').forEach(el => obs.observe(el));
  }

  // AD_CLICK
  document.querySelectorAll('[data-ad-network]').forEach(el => {
    el.addEventListener('click', () => fire('AD_CLICK', {
      network: el.dataset.adNetwork || 'unknown',
      ad_unit: el.dataset.adUnit   || 'unknown',
    }));
  });
})();
</script>"#
}

// ─────────────────────────────────────────────────────────────────────────────
//  Request payload accepted by POST /api/funnel
// ─────────────────────────────────────────────────────────────────────────────

/// Body sent by `client_tracking_script()` to `POST /api/funnel`.
#[derive(Debug, Deserialize)]
pub struct FunnelEventPayload {
    pub event_type: String,
    pub anon_id:    String,
    #[serde(flatten)]
    pub extra:      Value,
}
