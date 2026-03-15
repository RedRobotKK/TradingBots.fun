//! Multi-tenant support — each tenant gets an isolated BotState backed by
//! their own Hyperliquid sub-account credentials.
//!
//! ## How it works
//!
//! 1. The operator registers tenants via `TenantManager::register()`.
//! 2. Each tenant has:
//!    - A unique `TenantId` (UUID).
//!    - An HL wallet address + optional secret (for live trading).
//!    - An isolated `SharedState` so P&L, positions, and capital never mix.
//!    - A `TenantConfig` that inherits most settings from the global `Config`
//!      but overrides capital and API credentials per-tenant.
//! 3. The trading loop calls `TenantManager::for_each()` to iterate tenants.
//! 4. The web layer reads `TenantManager::get()` to serve per-tenant state.
//!
//! ## Revenue
//!
//! Every live order already embeds the platform builder code (set in `Config`).
//! The builder fee is invisible to tenants — they simply deposit and withdraw
//! from their own HL accounts; the platform earns 2–3 bps on every fill.
//!
//! ## Phase 1 (current): in-memory only
//!
//! Tenants are registered at startup from environment variables or a config
//! file.  Persistence to SQLite / Postgres is planned for Phase 2.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;
use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration, Utc};

use crate::web_dashboard::{BotState, SharedState};

// ─────────────────────────────────────────────────────────────────────────────

/// Opaque tenant identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TenantId(pub String);

impl TenantId {
    pub fn new() -> Self { TenantId(Uuid::new_v4().to_string()) }
    pub fn from_str(s: &str) -> Self { TenantId(s.to_string()) }
    pub fn as_str(&self) -> &str { &self.0 }
}

impl std::fmt::Display for TenantId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ─────────────────────────────────────────────────────────────────────────────

/// Tier determines fee schedule applied to this tenant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TenantTier {
    /// Free tier — paper trading only, no live orders.
    Free,
    /// Paid tier — live trading enabled, full feature access.
    Pro,
    /// Internal / operator — no fees, used for the platform's own capital.
    Internal,
}

// ─────────────────────────────────────────────────────────────────────────────

/// Per-tenant configuration overlay (overrides global `Config` where set).
#[derive(Debug, Clone)]
pub struct TenantConfig {
    /// Display name shown in the consumer webapp.
    pub display_name:    String,
    /// Contact email — used for P&L digests and Stripe customer lookup.
    pub email:           Option<String>,
    /// Privy Decentralised Identifier — set on first Privy login.
    /// Format: `"did:privy:clxxxxxxxxxxxxxxxxx"`.
    /// Used to look up an existing tenant across browser sessions.
    pub privy_did:       Option<String>,
    /// Stripe customer ID — set when the tenant completes checkout.
    pub stripe_customer: Option<String>,
    /// Stripe subscription ID — used to cancel on churn.
    pub stripe_sub_id:   Option<String>,
    /// Initial capital allocation in USD.
    pub initial_capital: f64,
    /// Hyperliquid wallet address (0x…) — required for live trading.
    pub wallet_address:  Option<String>,
    /// Private key hex — required for live order signing.  **Never logged.**
    pub secret_key:      Option<String>,
    /// Service tier.
    pub tier:            TenantTier,
    /// Whether live trading is active for this tenant.
    pub live_trading:    bool,
    /// Pro trial expiry — `Some(t)` while trial is active, `None` otherwise.
    /// When `Some` AND `tier == Free`, live trading is still allowed until `t`.
    pub trial_ends_at:   Option<DateTime<Utc>>,

    // ── Legal & onboarding ────────────────────────────────────────────────

    /// Timestamp when the tenant accepted the platform Terms & Risk Disclosure.
    /// `None` means the terms wall has not yet been cleared — the `/app`
    /// consumer routes will redirect to `/app/onboarding` until this is set.
    pub terms_accepted_at:  Option<DateTime<Utc>>,

    // ── Wallet & balance tracking ─────────────────────────────────────────

    /// UTC timestamp when the tenant's HL wallet was first confirmed by the
    /// operator or self-linked in `/app/settings`.  Used for audit purposes.
    pub wallet_linked_at:   Option<DateTime<Utc>>,

    /// Last known cleared (settled) HL balance in USD.
    /// Updated by the trading loop on each cycle and used by `fund_tracker`
    /// to detect deposits / withdrawals between cycles.
    /// This is the *cleared* balance — NOT the mark-to-market equity.
    pub hl_balance_usd:     f64,
}

impl TenantConfig {
    pub fn paper(name: &str, capital: f64) -> Self {
        TenantConfig {
            display_name:       name.to_string(),
            email:              None,
            privy_did:          None,
            stripe_customer:    None,
            stripe_sub_id:      None,
            initial_capital:    capital,
            wallet_address:     None,
            secret_key:         None,
            tier:               TenantTier::Free,
            live_trading:       false,
            trial_ends_at:      None,
            terms_accepted_at:  None,
            wallet_linked_at:   None,
            hl_balance_usd:     0.0,
        }
    }

    pub fn live(name: &str, capital: f64, wallet: &str, secret: &str) -> Self {
        TenantConfig {
            display_name:       name.to_string(),
            email:              None,
            privy_did:          None,
            stripe_customer:    None,
            stripe_sub_id:      None,
            initial_capital:    capital,
            wallet_address:     Some(wallet.to_string()),
            secret_key:         Some(secret.to_string()),
            tier:               TenantTier::Pro,
            live_trading:       true,
            trial_ends_at:      None,
            terms_accepted_at:  None,
            wallet_linked_at:   None,
            hl_balance_usd:     0.0,
        }
    }

    /// Is live trading currently allowed for this tenant?
    ///
    /// True when:
    ///   - Tier is `Pro` or `Internal`, OR
    ///   - An active 14-day trial has not yet expired.
    pub fn is_live_enabled(&self) -> bool {
        match self.tier {
            TenantTier::Pro | TenantTier::Internal => self.live_trading,
            TenantTier::Free => {
                self.trial_ends_at
                    .map(|exp| Utc::now() < exp)
                    .unwrap_or(false)
            }
        }
    }

    /// Days remaining in trial, or 0 if no active trial.
    pub fn trial_days_remaining(&self) -> i64 {
        if self.tier != TenantTier::Free { return 0; }
        self.trial_ends_at
            .map(|exp| (exp - Utc::now()).num_days().max(0))
            .unwrap_or(0)
    }
}

// ─────────────────────────────────────────────────────────────────────────────

/// A single registered tenant with its isolated runtime state.
pub struct TenantHandle {
    pub id:     TenantId,
    pub config: TenantConfig,
    /// Isolated trading state — never shared with other tenants.
    pub state:  SharedState,
}

impl TenantHandle {
    pub fn new(id: TenantId, config: TenantConfig) -> Self {
        let mut initial = BotState::default();
        initial.capital         = config.initial_capital;
        initial.initial_capital = config.initial_capital;
        initial.peak_equity     = config.initial_capital;
        initial.status          = format!("Tenant {} initialised", id);

        TenantHandle {
            id,
            config,
            state: Arc::new(RwLock::new(initial)),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────

/// Registry of all active tenants.
///
/// Wrapped in `Arc<RwLock<>>` and shared across the Axum router and trading
/// loop via Axum's `State` extractor.
pub struct TenantManager {
    tenants: HashMap<TenantId, TenantHandle>,
}

impl TenantManager {
    pub fn new() -> Self {
        TenantManager { tenants: HashMap::new() }
    }

    /// Register a new tenant.  Returns the assigned `TenantId`.
    pub fn register(&mut self, config: TenantConfig) -> TenantId {
        let id = TenantId::new();
        let handle = TenantHandle::new(id.clone(), config);
        log::info!("🏢 Registered tenant {} ({})", handle.config.display_name, id);
        self.tenants.insert(id.clone(), handle);
        id
    }

    /// Look up a tenant by ID.
    pub fn get(&self, id: &TenantId) -> Option<&TenantHandle> {
        self.tenants.get(id)
    }

    /// Iterate all active tenants.
    pub fn all(&self) -> impl Iterator<Item = &TenantHandle> {
        self.tenants.values()
    }

    /// Count of registered tenants.
    pub fn count(&self) -> usize { self.tenants.len() }

    /// Remove a tenant (e.g., on churn).
    pub fn deregister(&mut self, id: &TenantId) -> Result<()> {
        self.tenants.remove(id)
            .ok_or_else(|| anyhow!("Tenant {} not found", id))?;
        log::info!("🏢 Deregistered tenant {}", id);
        Ok(())
    }

    // ── Billing mutations ─────────────────────────────────────────────────────

    /// Upgrade a tenant to Pro after a successful Stripe payment.
    ///
    /// Stores the Stripe customer + subscription IDs for future cancellation.
    /// Clears any active trial (no longer needed).
    pub fn upgrade_to_pro(
        &mut self,
        id:            &TenantId,
        customer_id:   &str,
        subscription_id: &str,
    ) -> Result<()> {
        let handle = self.tenants.get_mut(id)
            .ok_or_else(|| anyhow!("Tenant {} not found", id))?;
        handle.config.tier            = TenantTier::Pro;
        handle.config.live_trading    = true;
        handle.config.trial_ends_at   = None;
        handle.config.stripe_customer = Some(customer_id.to_string());
        handle.config.stripe_sub_id   = Some(subscription_id.to_string());
        log::info!("💳 Tenant {} → Pro (sub {})", id, subscription_id);
        Ok(())
    }

    /// Downgrade a tenant to Free (subscription cancelled or payment failed).
    ///
    /// Live trading is disabled immediately; paper trading continues.
    pub fn downgrade_to_free(&mut self, id: &TenantId) -> Result<()> {
        let handle = self.tenants.get_mut(id)
            .ok_or_else(|| anyhow!("Tenant {} not found", id))?;
        handle.config.tier          = TenantTier::Free;
        handle.config.live_trading  = false;
        handle.config.stripe_sub_id = None;
        log::info!("⬇ Tenant {} → Free (sub ended)", id);
        Ok(())
    }

    /// Start a 14-day Pro trial for a Free-tier tenant.
    ///
    /// Does nothing if the tenant is already Pro or has an active trial.
    pub fn start_trial(&mut self, id: &TenantId, days: u64) -> Result<()> {
        let handle = self.tenants.get_mut(id)
            .ok_or_else(|| anyhow!("Tenant {} not found", id))?;
        if handle.config.tier == TenantTier::Pro {
            return Ok(()); // already paid
        }
        if handle.config.trial_ends_at.map(|e| Utc::now() < e).unwrap_or(false) {
            return Ok(()); // trial already active
        }
        handle.config.live_trading  = true;
        handle.config.trial_ends_at = Some(Utc::now() + Duration::days(days as i64));
        log::info!("🎁 Tenant {} trial started ({} days)", id, days);
        Ok(())
    }

    /// Look up a tenant by Stripe customer ID (needed for webhook events that
    /// don't carry tenant_id metadata).
    pub fn find_by_stripe_customer(&self, customer_id: &str) -> Option<&TenantHandle> {
        self.tenants.values()
            .find(|h| h.config.stripe_customer.as_deref() == Some(customer_id))
    }

    // ── Privy identity ────────────────────────────────────────────────────────

    /// Look up a tenant by Privy DID.
    ///
    /// Returns `None` if no tenant with the given DID is registered.
    pub fn find_by_privy_did(&self, did: &str) -> Option<&TenantHandle> {
        self.tenants.values()
            .find(|h| h.config.privy_did.as_deref() == Some(did))
    }

    /// Find an existing tenant by Privy DID, or register a new Free tenant if
    /// this DID has never been seen before.
    ///
    /// Called on every successful Privy login.  New users are created with
    /// zero capital and no HL wallet; the operator links those separately.
    /// Returns the `TenantId` (existing or newly created).
    pub fn register_or_get_by_privy_did(
        &mut self,
        did:   &str,
        email: Option<String>,
    ) -> TenantId {
        // Return existing tenant if we already know this Privy DID
        if let Some(handle) = self.find_by_privy_did(did) {
            return handle.id.clone();
        }

        // New user — register as Free, paper-trading only, zero capital until
        // the operator links their HL wallet and deposits funds.
        let mut cfg         = TenantConfig::paper(did, 0.0);
        cfg.privy_did       = Some(did.to_string());
        cfg.email           = email;
        cfg.display_name    = did.to_string(); // DID shown until name is set

        let id = self.register(cfg);
        log::info!("👤 New Privy user registered: tenant_id={} did={}", id, did);
        id
    }

    // ── Legal acceptance ──────────────────────────────────────────────────────

    /// Record that the tenant has accepted the platform Terms & Risk Disclosure.
    ///
    /// Idempotent — calling twice does not overwrite the original timestamp.
    pub fn accept_terms(&mut self, id: &TenantId) -> Result<()> {
        let handle = self.tenants.get_mut(id)
            .ok_or_else(|| anyhow!("Tenant {} not found", id))?;
        if handle.config.terms_accepted_at.is_none() {
            handle.config.terms_accepted_at = Some(Utc::now());
            log::info!("✅ Tenant {} accepted terms", id);
        }
        Ok(())
    }

    /// Returns `true` when the tenant has accepted the Terms & Risk Disclosure.
    pub fn has_accepted_terms(&self, id: &TenantId) -> bool {
        self.tenants.get(id)
            .and_then(|h| h.config.terms_accepted_at)
            .is_some()
    }

    // ── Wallet management ─────────────────────────────────────────────────────

    /// Link a Hyperliquid wallet address to this tenant.
    ///
    /// The address must start with `"0x"` and be at least 10 characters long.
    /// Stamps `wallet_linked_at` on first link; subsequent calls update the
    /// address (e.g. if a user migrates to a new wallet) but preserve the
    /// original `wallet_linked_at` timestamp.
    pub fn link_wallet(&mut self, id: &TenantId, address: &str) -> Result<()> {
        if !address.starts_with("0x") || address.len() < 10 {
            return Err(anyhow!(
                "Invalid wallet address '{}': must start with 0x and be ≥10 chars",
                address
            ));
        }
        let handle = self.tenants.get_mut(id)
            .ok_or_else(|| anyhow!("Tenant {} not found", id))?;
        handle.config.wallet_address = Some(address.to_string());
        if handle.config.wallet_linked_at.is_none() {
            handle.config.wallet_linked_at = Some(Utc::now());
        }
        log::info!("🔗 Tenant {} linked wallet {}", id, address);
        Ok(())
    }

    // ── Balance tracking ──────────────────────────────────────────────────────

    /// Update the stored HL cleared balance for a tenant.
    ///
    /// Returns the *previous* balance (needed by `fund_tracker::detect_and_record`
    /// to compute the delta for deposit/withdrawal detection).
    pub fn update_hl_balance(&mut self, id: &TenantId, new_balance: f64) -> Result<f64> {
        let handle = self.tenants.get_mut(id)
            .ok_or_else(|| anyhow!("Tenant {} not found", id))?;
        let prev = handle.config.hl_balance_usd;
        handle.config.hl_balance_usd = new_balance;
        Ok(prev)
    }
}

/// Shared `TenantManager` alias — passed around as Axum `State`.
pub type SharedTenantManager = Arc<RwLock<TenantManager>>;

/// Convenience constructor.
pub fn new_tenant_manager() -> SharedTenantManager {
    Arc::new(RwLock::new(TenantManager::new()))
}

// ─────────────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_lookup() {
        let mut mgr = TenantManager::new();
        let id = mgr.register(TenantConfig::paper("Alice", 1000.0));
        assert!(mgr.get(&id).is_some());
        assert_eq!(mgr.get(&id).unwrap().config.display_name, "Alice");
        assert_eq!(mgr.count(), 1);
    }

    #[test]
    fn deregister_removes_tenant() {
        let mut mgr = TenantManager::new();
        let id = mgr.register(TenantConfig::paper("Bob", 500.0));
        assert_eq!(mgr.count(), 1);
        mgr.deregister(&id).unwrap();
        assert_eq!(mgr.count(), 0);
        assert!(mgr.get(&id).is_none());
    }

    #[test]
    fn deregister_unknown_returns_err() {
        let mut mgr = TenantManager::new();
        let fake_id = TenantId::from_str("nonexistent");
        assert!(mgr.deregister(&fake_id).is_err());
    }

    #[test]
    fn multiple_tenants_isolated() {
        let mut mgr = TenantManager::new();
        let a = mgr.register(TenantConfig::paper("Alice", 1000.0));
        let b = mgr.register(TenantConfig::paper("Bob",   2000.0));
        assert_ne!(a, b);
        // States are different Arc instances
        let sa = Arc::as_ptr(&mgr.get(&a).unwrap().state);
        let sb = Arc::as_ptr(&mgr.get(&b).unwrap().state);
        assert_ne!(sa, sb);
    }

    #[test]
    fn live_config_has_wallet_and_secret() {
        let cfg = TenantConfig::live("Carol", 5000.0, "0xABC", "deadbeef");
        assert_eq!(cfg.tier, TenantTier::Pro);
        assert!(cfg.live_trading);
        assert_eq!(cfg.wallet_address.as_deref(), Some("0xABC"));
        assert_eq!(cfg.secret_key.as_deref(), Some("deadbeef"));
    }

    #[tokio::test]
    async fn initial_state_matches_config_capital() {
        let mut mgr = TenantManager::new();
        let id = mgr.register(TenantConfig::paper("Dave", 7500.0));
        let state = mgr.get(&id).unwrap().state.read().await;
        assert_eq!(state.capital, 7500.0);
        assert_eq!(state.initial_capital, 7500.0);
    }

    // ── Privy identity tests ──────────────────────────────────────────────────

    #[test]
    fn register_or_get_creates_new_tenant_for_unknown_did() {
        let mut mgr = TenantManager::new();
        let did     = "did:privy:cltest0000000001";
        let id      = mgr.register_or_get_by_privy_did(did, Some("alice@test.com".into()));
        assert!(mgr.get(&id).is_some());
        assert_eq!(mgr.get(&id).unwrap().config.privy_did.as_deref(), Some(did));
        assert_eq!(mgr.get(&id).unwrap().config.email.as_deref(), Some("alice@test.com"));
        assert_eq!(mgr.count(), 1);
    }

    #[test]
    fn register_or_get_returns_same_id_for_known_did() {
        let mut mgr = TenantManager::new();
        let did     = "did:privy:cltest0000000002";
        let id1     = mgr.register_or_get_by_privy_did(did, None);
        let id2     = mgr.register_or_get_by_privy_did(did, None);
        // Second call must return the SAME tenant, not create a duplicate
        assert_eq!(id1, id2,    "same DID must map to same tenant_id");
        assert_eq!(mgr.count(), 1, "must not create a second tenant");
    }

    #[test]
    fn find_by_privy_did_returns_none_for_unknown_did() {
        let mut mgr = TenantManager::new();
        mgr.register_or_get_by_privy_did("did:privy:cltest0000000003", None);
        let result = mgr.find_by_privy_did("did:privy:nobody");
        assert!(result.is_none());
    }

    // ── Terms acceptance ──────────────────────────────────────────────────────

    #[test]
    fn new_tenant_has_not_accepted_terms() {
        let mut mgr = TenantManager::new();
        let id = mgr.register(TenantConfig::paper("Eve", 0.0));
        assert!(!mgr.has_accepted_terms(&id));
        assert!(mgr.get(&id).unwrap().config.terms_accepted_at.is_none());
    }

    #[test]
    fn accept_terms_sets_timestamp() {
        let mut mgr = TenantManager::new();
        let id = mgr.register(TenantConfig::paper("Frank", 0.0));
        mgr.accept_terms(&id).unwrap();
        assert!(mgr.has_accepted_terms(&id));
        assert!(mgr.get(&id).unwrap().config.terms_accepted_at.is_some());
    }

    #[test]
    fn accept_terms_is_idempotent() {
        let mut mgr = TenantManager::new();
        let id = mgr.register(TenantConfig::paper("Grace", 0.0));
        mgr.accept_terms(&id).unwrap();
        let first_ts = mgr.get(&id).unwrap().config.terms_accepted_at;
        // Small sleep is not safe in tests — just call again and verify it's the same value
        mgr.accept_terms(&id).unwrap();
        let second_ts = mgr.get(&id).unwrap().config.terms_accepted_at;
        assert_eq!(first_ts, second_ts, "accept_terms must not overwrite the original timestamp");
    }

    // ── Wallet linking ────────────────────────────────────────────────────────

    #[test]
    fn link_wallet_stores_address_and_timestamp() {
        let mut mgr = TenantManager::new();
        let id = mgr.register(TenantConfig::paper("Heidi", 0.0));
        mgr.link_wallet(&id, "0xAbCd1234567890ef").unwrap();
        let cfg = &mgr.get(&id).unwrap().config;
        assert_eq!(cfg.wallet_address.as_deref(), Some("0xAbCd1234567890ef"));
        assert!(cfg.wallet_linked_at.is_some());
    }

    #[test]
    fn link_wallet_preserves_original_timestamp_on_update() {
        let mut mgr = TenantManager::new();
        let id = mgr.register(TenantConfig::paper("Ivan", 0.0));
        mgr.link_wallet(&id, "0xFirstWallet1234567890").unwrap();
        let first_ts = mgr.get(&id).unwrap().config.wallet_linked_at;
        mgr.link_wallet(&id, "0xSecondWallet12345678").unwrap();
        let second_ts = mgr.get(&id).unwrap().config.wallet_linked_at;
        assert_eq!(first_ts, second_ts, "wallet_linked_at must not change on re-link");
    }

    #[test]
    fn link_wallet_rejects_non_hex_address() {
        let mut mgr = TenantManager::new();
        let id = mgr.register(TenantConfig::paper("Judy", 0.0));
        assert!(mgr.link_wallet(&id, "not-a-wallet").is_err());
    }

    #[test]
    fn link_wallet_rejects_short_address() {
        let mut mgr = TenantManager::new();
        let id = mgr.register(TenantConfig::paper("Karl", 0.0));
        assert!(mgr.link_wallet(&id, "0x123").is_err());
    }

    // ── Balance tracking ──────────────────────────────────────────────────────

    #[test]
    fn update_hl_balance_returns_previous_value() {
        let mut mgr = TenantManager::new();
        let id = mgr.register(TenantConfig::paper("Laura", 0.0));
        let prev = mgr.update_hl_balance(&id, 500.0).unwrap();
        assert!((prev - 0.0).abs() < 0.001, "initial balance should be 0");
        assert!((mgr.get(&id).unwrap().config.hl_balance_usd - 500.0).abs() < 0.001);
    }

    #[test]
    fn update_hl_balance_tracks_successive_changes() {
        let mut mgr = TenantManager::new();
        let id = mgr.register(TenantConfig::paper("Mike", 0.0));
        mgr.update_hl_balance(&id, 1000.0).unwrap();
        let prev2 = mgr.update_hl_balance(&id, 1500.0).unwrap();
        assert!((prev2 - 1000.0).abs() < 0.001, "second update should return the first set value");
    }
}
