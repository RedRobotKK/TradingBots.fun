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
pub mod ai_feedback;
pub mod ai_helpers;
pub mod ai_reviewer;
pub mod bridge;
pub mod candlestick_patterns;
pub mod chart_patterns;
pub mod coins;
pub mod collective;
pub mod config;
pub mod correlation;
pub mod cross_exchange;
pub mod daily_analyst;
pub mod data;
pub mod db;
pub mod decision;
pub mod exchange;
pub mod fund_tracker;
pub mod funding;
pub mod funnel;
pub mod hl_wallet;
pub mod indicators;
pub mod invite;
pub mod leaderboard;
pub mod learner;
pub mod ledger;
pub mod mailer;
pub mod metrics;
pub mod notifier;
pub mod onchain;
pub mod pattern_insights;
pub mod persistence;
pub mod position_monitor;
pub mod price_feed;
pub mod privy;
pub mod signal_engine;
pub mod reporting;
pub mod risk;
pub mod sentiment;
pub mod signal_watchlist;
pub mod signals;
pub mod stripe;
pub mod tenant;
pub mod thesis;
pub mod trade_log;
pub mod web_dashboard;
pub mod connectors;
pub mod latency;
