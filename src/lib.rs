// Suppress stylistic clippy lints that don't affect correctness
#![allow(clippy::match_single_binding)]
#![allow(clippy::map_unwrap_or)]
#![allow(clippy::len_zero)]
#![allow(clippy::should_implement_trait)]
#![allow(clippy::new_without_default)]
#![allow(clippy::let_and_return)]
#![allow(clippy::manual_range_contains)]
#![allow(clippy::if_same_then_else)]
#![allow(private_interfaces)]
#![allow(clippy::unnecessary_lazy_evaluations)]
#![allow(clippy::manual_clamp)]
#![allow(clippy::or_fun_call)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::collapsible_if)]

/// tradingbots.fun — live trading bot crate.
///
/// The live binary lives in `main.rs`.  Modules that need to be accessible from
/// integration tests (`/tests/`) are re-exported here via `pub mod`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// ── Modules needed by integration tests ──────────────────────────────────────
pub mod correlation;
pub mod notifier;
pub mod onchain;
pub mod learner;
pub mod web_dashboard;
pub mod metrics;
pub mod db;
pub mod tenant;
pub mod privy;
pub mod stripe;
pub mod config;
pub mod data;
pub mod indicators;
pub mod signals;
pub mod risk;
pub mod exchange;
pub mod decision;
pub mod persistence;
pub mod sentiment;
pub mod funding;
pub mod cross_exchange;
pub mod signal_watchlist;
pub mod trade_log;
pub mod daily_analyst;
pub mod ledger;
pub mod candlestick_patterns;
pub mod chart_patterns;
pub mod ai_reviewer;
pub mod coins;
pub mod fund_tracker;
pub mod funnel;
pub mod invite;
pub mod leaderboard;
pub mod mailer;
pub mod thesis;
pub mod hl_wallet;
pub mod collective;
