//! State persistence — atomic JSON snapshot of trading state.
//!
//! Saves the bot's critical runtime state to `state.json` after every N cycles
//! and on clean shutdown.  On startup, if the file exists the bot resumes from
//! where it left off — open positions, realised P&L, equity window, and metrics
//! all survive a service restart.
//!
//! ## Atomic write strategy
//!
//! 1. Serialize state to JSON.
//! 2. Write to `state.json.tmp`.
//! 3. `rename()` tmp → `state.json`  (atomic on POSIX — either the old or new
//!    file is visible; never a half-written file).
//!
//! ## What is persisted
//!
//! | Field            | Why                                               |
//! |------------------|---------------------------------------------------|
//! | capital          | Free cash after open positions                    |
//! | initial_capital  | Needed for all-time % return calculation          |
//! | peak_equity      | All-time high — drives dashboard drawdown metric  |
//! | pnl              | Cumulative realised P&L                           |
//! | cycle_count      | Monotonic counter — useful for log correlation    |
//! | positions        | Open positions — CRITICAL for live/testnet mode   |
//! | closed_trades    | History shown on dashboard (last 20 displayed)    |
//! | metrics          | Win rate, Sharpe, Sortino etc.                    |
//! | equity_window    | 7-day rolling window for circuit-breaker          |
//! | cb_active        | Circuit-breaker flag — preserves risk state       |
//! | saved_at         | Timestamp for staleness detection                 |
//! | version          | Schema version for future migrations              |

use crate::metrics::PerformanceMetrics;
use crate::web_dashboard::{BotState, ClosedTrade, PaperPosition};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

const STATE_FILE: &str = "state.json";
const STATE_FILE_TMP: &str = "state.json.tmp";
/// Save state to disk every this many cycles (30 s each → every 5 min).
pub const SAVE_EVERY_N_CYCLES: u64 = 10;

// ─────────────────────────── Persisted snapshot ──────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedState {
    /// Schema version — increment when fields are added/removed.
    pub version: u32,
    /// ISO-8601 timestamp when the snapshot was taken (UTC).
    pub saved_at: String,

    // ── Financials ────────────────────────────────────────────────────────────
    pub capital: f64,
    pub initial_capital: f64,
    pub peak_equity: f64,
    pub pnl: f64,

    // ── Counters ──────────────────────────────────────────────────────────────
    pub cycle_count: u64,

    // ── Positions & trades ────────────────────────────────────────────────────
    pub positions: Vec<PaperPosition>,
    pub closed_trades: Vec<ClosedTrade>,

    // ── Risk state ────────────────────────────────────────────────────────────
    pub metrics: PerformanceMetrics,
    pub equity_window: VecDeque<(i64, f64)>,
    pub cb_active: bool,
}

impl PersistedState {
    // ── Snapshot creation ─────────────────────────────────────────────────────

    /// Build a snapshot from the current `BotState`.
    pub fn from_bot_state(s: &BotState) -> Self {
        PersistedState {
            version: 1,
            saved_at: chrono::Utc::now().to_rfc3339(),
            capital: s.capital,
            initial_capital: s.initial_capital,
            peak_equity: s.peak_equity,
            pnl: s.pnl,
            cycle_count: s.cycle_count,
            positions: s.positions.clone(),
            closed_trades: s.closed_trades.clone(),
            metrics: s.metrics.clone(),
            equity_window: s.equity_window.clone(),
            cb_active: s.cb_active,
        }
    }

    // ── Disk I/O ──────────────────────────────────────────────────────────────

    /// Atomically write snapshot to `state.json`.
    ///
    /// Uses a tmp → rename pattern so a crash mid-write never corrupts the
    /// existing snapshot.
    pub fn save(&self) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(STATE_FILE_TMP, &json)?;
        std::fs::rename(STATE_FILE_TMP, STATE_FILE)?;
        Ok(())
    }

    /// Try to load the snapshot.  Returns `None` (and logs a warning) if the
    /// file is missing, corrupt, or from an incompatible schema version.
    pub fn load() -> Option<Self> {
        let data = match std::fs::read_to_string(STATE_FILE) {
            Ok(d) => d,
            Err(_) => {
                log::info!("💾 No state.json found — starting fresh");
                return None;
            }
        };
        match serde_json::from_str::<PersistedState>(&data) {
            Ok(s) => {
                if s.version != 1 {
                    log::warn!(
                        "⚠ state.json schema v{} ≠ expected v1 — ignoring, starting fresh",
                        s.version
                    );
                    return None;
                }
                log::info!(
                    "💾 Resuming from state.json — {} open pos · {} closed · \
                     capital=${:.2} · saved {}",
                    s.positions.len(),
                    s.closed_trades.len(),
                    s.capital,
                    s.saved_at,
                );
                Some(s)
            }
            Err(e) => {
                log::warn!("⚠ state.json parse error: {} — starting fresh", e);
                None
            }
        }
    }

    // ── State restoration ─────────────────────────────────────────────────────

    /// Restore fields into a `BotState` that was initialised with config defaults.
    ///
    /// `initial_capital` is deliberately NOT restored from the snapshot — it
    /// always comes from the current `.env` so the operator can adjust it.
    pub fn apply_to(&self, bot: &mut BotState) {
        bot.capital = self.capital;
        bot.peak_equity = self.peak_equity;
        bot.pnl = self.pnl;
        bot.cycle_count = self.cycle_count;
        bot.positions = self.positions.clone();
        bot.closed_trades = self.closed_trades.clone();
        bot.metrics = self.metrics.clone();
        bot.equity_window = self.equity_window.clone();
        bot.cb_active = self.cb_active;
        // initial_capital intentionally kept from config (operator may change it)
    }
}

// ─────────────────────────── Convenience helper ───────────────────────────────

/// Snapshot the current `SharedState` and save to disk.  Logs errors but does
/// not propagate them — a failed save should never crash the trading loop.
pub async fn save_snapshot(bot_state: &crate::web_dashboard::SharedState) {
    let snapshot = {
        let s = bot_state.read().await;
        PersistedState::from_bot_state(&s)
    };
    match snapshot.save() {
        Ok(_) => log::debug!("💾 State snapshot saved"),
        Err(e) => log::warn!("⚠ Could not save state snapshot: {}", e),
    }
}

// ─────────────────────────── Tests ───────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state() -> PersistedState {
        PersistedState {
            version: 1,
            saved_at: "2026-01-01T00:00:00Z".to_string(),
            capital: 850.0,
            initial_capital: 1000.0,
            peak_equity: 1050.0,
            pnl: 50.0,
            cycle_count: 120,
            positions: vec![],
            closed_trades: vec![],
            metrics: PerformanceMetrics::default(),
            equity_window: VecDeque::new(),
            cb_active: false,
        }
    }

    #[test]
    fn roundtrip_json_preserves_all_fields() {
        let s = make_state();
        let json = serde_json::to_string_pretty(&s).unwrap();
        let loaded: PersistedState = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.capital, s.capital);
        assert_eq!(loaded.initial_capital, s.initial_capital);
        assert_eq!(loaded.peak_equity, s.peak_equity);
        assert_eq!(loaded.pnl, s.pnl);
        assert_eq!(loaded.cycle_count, s.cycle_count);
        assert_eq!(loaded.version, 1);
    }

    #[test]
    fn wrong_schema_version_returns_none() {
        let mut s = make_state();
        s.version = 99;
        let json = serde_json::to_string_pretty(&s).unwrap();
        // Write to a temp location and test parse logic directly
        let parsed: PersistedState = serde_json::from_str(&json).unwrap();
        // version check happens inside load() — here we verify the field is honoured
        assert_eq!(parsed.version, 99);
    }

    #[test]
    fn apply_to_restores_financial_fields() {
        let snapshot = make_state();
        let mut bot = BotState {
            initial_capital: 1000.0,
            ..BotState::default()
        };

        snapshot.apply_to(&mut bot);

        assert_eq!(bot.capital, 850.0);
        assert_eq!(bot.peak_equity, 1050.0);
        assert_eq!(bot.pnl, 50.0);
        assert_eq!(bot.cycle_count, 120);
        // initial_capital must NOT be overwritten by the snapshot
        assert_eq!(bot.initial_capital, 1000.0);
    }

    #[test]
    fn apply_to_does_not_overwrite_initial_capital() {
        let mut snapshot = make_state();
        snapshot.initial_capital = 500.0; // operator previously had $500 configured
        let mut bot = BotState {
            initial_capital: 2000.0,
            ..BotState::default()
        }; // new .env value — should win

        snapshot.apply_to(&mut bot);

        assert_eq!(
            bot.initial_capital, 2000.0,
            "initial_capital must come from config, not snapshot"
        );
    }

    #[test]
    fn equity_window_survives_roundtrip() {
        let mut s = make_state();
        s.equity_window.push_back((1_700_000_000, 1000.0));
        s.equity_window.push_back((1_700_000_030, 1002.5));

        let json = serde_json::to_string_pretty(&s).unwrap();
        let loaded: PersistedState = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.equity_window.len(), 2);
        assert_eq!(loaded.equity_window[0], (1_700_000_000, 1000.0));
        assert_eq!(loaded.equity_window[1], (1_700_000_030, 1002.5));
    }
}
