//! On-chain exchange flow signal module.
//!
//! Fetches exchange netflow data from Coinglass — the net quantity of coins
//! moving to or from centralised exchanges.  This is a direct measure of
//! near-term selling (or accumulation) intent:
//!
//! | 24h Exchange Netflow   | Market state               | Signal         |
//! |------------------------|----------------------------|----------------|
//! | Large positive inflow  | Coins moving TO exchanges  | Bearish (sell) |
//! | Near zero              | Balanced flows             | Neutral        |
//! | Large negative outflow | Coins leaving exchanges    | Bullish (HODL) |
//!
//! ## Source
//!
//! Coinglass Exchange Flow API (free tier, 10 req/day, cached 30 min).
//! Endpoint: `GET https://open-api.coinglass.com/public/v2/indicator/exchange_balance_list`
//! Auth: `CG-API-KEY: {COINGLASS_API_KEY}` header.
//!
//! If `COINGLASS_API_KEY` is not set the module returns neutral (0.0)
//! for all symbols — the trading loop continues without on-chain data.
//!
//! ## Wiring
//!
//! `decision.rs` calls `OnchainCache::get(symbol)` which returns `OnchainData`.
//! `OnchainData::signal_strength()` returns a value in [−1.0, +1.0] which is
//! added to the decision score with weight `weights.onchain`.

use anyhow::Result;
use log::{info, warn};
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Cache TTL — on-chain flows change slowly; 30-minute refresh is sufficient.
const CACHE_TTL: Duration = Duration::from_secs(1_800);
const BASE_URL:  &str     = "https://open-api.coinglass.com/public/v2/indicator/exchange_balance_list";

// ─────────────────────────── Public types ────────────────────────────────────

/// On-chain exchange flow snapshot for a single asset.
#[derive(Debug, Clone)]
pub struct OnchainData {
    // symbol and netflow_24h are public for future dashboard / API use.
    #[allow(dead_code)]
    pub symbol: String,
    /// Net coins moved to exchanges in the last 24 h.
    /// Positive = inflows (bearish), Negative = outflows (bullish).
    #[allow(dead_code)]
    pub netflow_24h: f64,
    /// Normalised signal strength in [−1.0, +1.0].
    /// Positive = bullish (outflows), Negative = bearish (inflows).
    cached_strength: f64,
}

impl OnchainData {
    /// Contrarian signal strength derived from the netflow z-score bucket.
    /// Range: −1.0 (strong inflows / bearish) … +1.0 (strong outflows / bullish).
    pub fn signal_strength(&self) -> f64 {
        self.cached_strength
    }

    /// Human-readable label for the dashboard (reserved for future UI widget).
    #[allow(dead_code)]
    pub fn label(&self) -> &'static str {
        match self.cached_strength {
            s if s >  0.60 => "🟢 Strong outflows",
            s if s >  0.25 => "🟢 Outflows",
            s if s > -0.25 => "⚪ Neutral",
            s if s > -0.60 => "🔴 Inflows",
            _              => "🔴 Strong inflows",
        }
    }
}

/// Neutral placeholder returned when the API key is absent or a fetch fails.
fn neutral(symbol: &str) -> OnchainData {
    OnchainData { symbol: symbol.to_string(), netflow_24h: 0.0, cached_strength: 0.0 }
}

// ─────────────────────────── API response types ───────────────────────────────

#[derive(Deserialize)]
struct CgResponse {
    data: Vec<CgEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CgEntry {
    symbol:          String,
    /// Net change in exchange balance over 24 h (in native coin units).
    net_flow:        Option<f64>,
    /// Exchange total balance in native coin units (used to normalise netflow).
    exchange_balance: Option<f64>,
}

// ─────────────────────────── Cache ───────────────────────────────────────────

struct Inner {
    data:       HashMap<String, OnchainData>,
    fetched_at: Option<Instant>,
}

pub struct OnchainCache {
    inner:   RwLock<Inner>,
    client:  Client,
    api_key: Option<String>,
}

pub type SharedOnchain = Arc<OnchainCache>;

impl OnchainCache {
    pub fn new() -> SharedOnchain {
        let api_key = std::env::var("COINGLASS_API_KEY").ok();
        if api_key.is_none() {
            info!("ℹ  COINGLASS_API_KEY not set — on-chain signal disabled (neutral)");
        }
        Arc::new(Self {
            inner:   RwLock::new(Inner { data: HashMap::new(), fetched_at: None }),
            client:  Client::builder().timeout(Duration::from_secs(15)).build().unwrap_or_default(),
            api_key,
        })
    }

    /// Pre-warm on startup.
    pub async fn warm(self: &Arc<Self>) {
        if self.api_key.is_some() {
            self.refresh().await;
        }
    }

    /// Retrieve on-chain data for a symbol.
    /// Refreshes the cache if stale; returns neutral if API key absent.
    pub async fn get(self: &Arc<Self>, symbol: &str) -> OnchainData {
        if self.api_key.is_none() {
            return neutral(symbol);
        }
        // Check staleness under a read lock.
        let stale = {
            let inner = self.inner.read().await;
            inner.fetched_at.is_none_or(|t| t.elapsed() > CACHE_TTL)
        };
        if stale {
            self.refresh().await;
        }
        let inner = self.inner.read().await;
        let base  = base_symbol(symbol);
        inner.data.get(base)
            .cloned()
            .unwrap_or_else(|| neutral(symbol))
    }

    // ── Private ────────────────────────────────────────────────────────────

    async fn refresh(&self) {
        let Some(key) = &self.api_key else { return };
        match self.fetch(key).await {
            Ok(data) => {
                let mut inner    = self.inner.write().await;
                inner.data       = data;
                inner.fetched_at = Some(Instant::now());
                info!("✅ On-chain cache refreshed ({} assets)", inner.data.len());
            }
            Err(e) => warn!("⚠  On-chain fetch failed: {e}"),
        }
    }

    async fn fetch(&self, api_key: &str) -> Result<HashMap<String, OnchainData>> {
        let resp: CgResponse = self.client
            .get(BASE_URL)
            .header("CG-API-KEY", api_key)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let mut map = HashMap::new();
        for entry in resp.data {
            let sym  = base_symbol(&entry.symbol).to_string();
            let flow = entry.net_flow.unwrap_or(0.0);
            let bal  = entry.exchange_balance.unwrap_or(0.0);
            // Normalise as fraction of exchange balance so large-cap coins
            // (huge absolute flow) are not always louder than small-caps.
            let norm = if bal > 0.0 { flow / bal } else { 0.0 };
            let strength = normalised_to_strength(norm);
            map.insert(sym.clone(), OnchainData {
                symbol:         sym,
                netflow_24h:    flow,
                cached_strength: strength,
            });
        }
        Ok(map)
    }
}

// ─────────────────────────── Helpers ─────────────────────────────────────────

/// Strip quote suffixes so "BTC-USD" → "BTC".
fn base_symbol(s: &str) -> &str {
    s.split('-').next().unwrap_or(s)
}

/// Map a normalised netflow fraction to a [−1.0, +1.0] signal.
/// Negative flow (outflows) → positive signal (bullish).
/// Positive flow (inflows)  → negative signal (bearish).
fn normalised_to_strength(norm: f64) -> f64 {
    // Thresholds as fraction of exchange balance:
    //   0.005 = 0.5 %  (significant daily move)
    //   0.002 = 0.2 %  (moderate)
    //   0.0005= 0.05%  (noise boundary)
    let raw = if      norm >  0.0050 { -1.00 } // strong inflows  → strong bear
              else if norm >  0.0020 { -0.65 } // moderate inflows → moderate bear
              else if norm >  0.0005 { -0.30 } // mild inflows    → slight bear
              else if norm < -0.0050 {  1.00 } // strong outflows  → strong bull
              else if norm < -0.0020 {  0.65 } // moderate outflows → moderate bull
              else if norm < -0.0005 {  0.30 } // mild outflows   → slight bull
              else                   {  0.00 }; // neutral band
    raw
}

// ─────────────────────────── Tests ───────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strong_inflow_is_bearish() {
        assert!(normalised_to_strength(0.01) < -0.9);
    }

    #[test]
    fn strong_outflow_is_bullish() {
        assert!(normalised_to_strength(-0.01) > 0.9);
    }

    #[test]
    fn neutral_band() {
        assert_eq!(normalised_to_strength(0.0003), 0.0);
        assert_eq!(normalised_to_strength(-0.0003), 0.0);
    }

    #[test]
    fn base_symbol_strips_suffix() {
        assert_eq!(base_symbol("BTC-USD"), "BTC");
        assert_eq!(base_symbol("ETH"),     "ETH");
    }
}
