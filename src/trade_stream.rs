//! Live winning-trade broadcast channel.
//!
//! Architecture
//! ────────────
//! A single `tokio::sync::broadcast` channel carries [`TradeWin`] events from
//! every tenant's `run_cycle()` to any number of SSE subscribers listening on
//! `GET /api/trade-stream`.
//!
//! The channel is initialised once at startup via [`init()`] and then shared
//! through a `OnceCell<Sender>` global.  Senders call [`emit()`]; consumers
//! (the Axum SSE handler) call [`subscribe()`].
//!
//! Capacity: 256 events — at 5 000 tenants averaging one win per ~20 cycles
//! (~600 s), the burst rate is ≪1 msg/s.  A capacity of 256 is large enough
//! that no subscriber will ever fall behind under realistic load.

use once_cell::sync::OnceCell;
use serde::Serialize;
use tokio::sync::broadcast;

// ── Event type ────────────────────────────────────────────────────────────────

/// A single winning (or partially-winning) trade close event.
///
/// Sent on every profitable partial or full close so the dashboard ticker can
/// show a live stream of "wins happening right now".
#[derive(Clone, Serialize, Debug)]
pub struct TradeWin {
    /// Trading symbol, e.g. `"BTC"`, `"ETH"`, `"SOL"`.
    pub symbol: String,

    /// Trade direction: `"LONG"` or `"SHORT"`.
    pub side: String,

    /// Realised PnL of this close in USD (always positive for a win).
    pub pnl: f64,

    /// PnL as a percentage of margin committed.
    pub pnl_pct: f64,

    /// R-multiple at close (1.0 = 1× risk, 2.0 = 2× risk, etc.).
    pub r_mult: f64,

    /// Human-readable close reason: `"Partial1.25R"`, `"TakeProfit4R"`, etc.
    pub reason: String,

    /// Display name of the wallet that made this trade, e.g. `"Bot Alpha"`,
    /// `"ScaleWallet-00042"`.  Set to `"unknown"` if not provided.
    pub wallet: String,

    /// ISO-8601 timestamp of the close (UTC).
    pub closed_at: String,
}

// ── Global channel ─────────────────────────────────────────────────────────

static TRADE_WIN_TX: OnceCell<broadcast::Sender<TradeWin>> = OnceCell::new();

/// Initialise the broadcast channel.  Must be called exactly once at startup,
/// before any tenant trading loops start.  Returns a placeholder receiver that
/// can be dropped — the channel lives for the process lifetime.
pub fn init() {
    let (tx, _rx) = broadcast::channel(256);
    // Ignore the error: if two threads race (shouldn't happen), the first wins.
    let _ = TRADE_WIN_TX.set(tx);
}

/// Emit a winning-trade event to all current SSE subscribers.
///
/// If no subscribers are connected the event is silently discarded.
/// If the channel is full (256 pending events) older events are dropped —
/// SSE subscribers are expected to be real-time, not archival.
pub fn emit(win: TradeWin) {
    if let Some(tx) = TRADE_WIN_TX.get() {
        // send() returns Err only when there are zero receivers — that's fine.
        let _ = tx.send(win);
    }
}

/// Subscribe to the winning-trade broadcast.
///
/// Returns `None` if [`init()`] has not been called yet.
pub fn subscribe() -> Option<broadcast::Receiver<TradeWin>> {
    TRADE_WIN_TX.get().map(|tx| tx.subscribe())
}
