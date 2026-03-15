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
}

impl TenantConfig {
    pub fn paper(name: &str, capital: f64) -> Self {
        TenantConfig {
            display_name:    name.to_string(),
            email:           None,
            stripe_customer: None,
            stripe_sub_id:   None,
            initial_capital: capital,
            wallet_address:  None,
            secret_key:      None,
            tier:            TenantTier::Free,
            live_trading:    false,
            trial_ends_at:   None,
        }
    }

    pub fn live(name: &str, capital: f64, wallet: &str, secret: &str) -> Self {
        TenantConfig {
            display_name:    name.to_string(),
            email:           None,
            stripe_customer: None,
            stripe_sub_id:   None,
            initial_capital: capital,
            wallet_address:  Some(wallet.to_string()),
            secret_key:      Some(secret.to_string()),
            tier:            TenantTier::Pro,
            live_trading:    true,
            trial_ends_at:   None,
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
}
