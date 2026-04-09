//! Structured daily trade log — JSONL format optimised for LLM ingestion.
//!
//! # Overview
//!
//! Every significant event is appended to `logs/trading_YYYY-MM-DD.jsonl` as a
//! single self-contained JSON object terminated by a newline.  The file rotates
//! automatically at midnight UTC.
//!
//! # Why JSONL?
//!
//! JSON Lines is the best format for LLM ingestion because:
//!   • Each line is a complete, parseable event with full context
//!   • No surrounding array brackets to break streaming parsers
//!   • Easy to `grep`, `jq`, or feed line-by-line into a context window
//!   • A day's log can be read in one pass or sliced by event type
//!
//! # Log directory
//!
//! Logs are written to `./logs/` relative to the binary's working directory.
//! On the VPS this is `/root/tradingbots-fun/logs/`.
//!
//! # Event types
//!
//! | event_type          | When emitted                                         |
//! |---------------------|------------------------------------------------------|
//! | `cycle_start`       | Beginning of each 30-second cycle                    |
//! | `decision`          | Every signal decision (including SKIP)               |
//! | `trade_entry`       | New position opened                                  |
//! | `trade_exit`        | Position fully closed (any reason)                   |
//! | `trade_partial`     | 1/3 profit-take at 2R or 4R milestone                |
//! | `trade_dca`         | DCA add-on to an existing position                   |
//! | `trade_pyramid`     | Pyramid add-on to a winning position                 |
//! | `circuit_breaker`   | Circuit breaker activation / deactivation            |
//! | `metrics_snapshot`  | Performance metrics snapshot (every 10 cycles)       |
//! | `day_close`         | End-of-day summary (emitted at midnight UTC)         |

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use log::warn;

// ─────────────────────────── Event types ─────────────────────────────────────

/// Every event includes a UTC timestamp and an event_type discriminant.
/// The `#[serde(tag = "event_type")]` ensures the type field is always first
/// in the JSON output, making it easy to filter with `jq`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum TradeEvent {
    /// Emitted at the start of each trading cycle.
    CycleStart {
        ts: String,
        cycle_number: u64,
        open_positions: usize,
        free_capital: f64,
        peak_equity: f64,
        btc_dom_pct: f64,
        btc_ret_24h: f64,
        btc_ret_4h: f64,
        candidate_count: usize,
    },

    /// Full decision record for every symbol analysed (including SKIP).
    /// Contains all indicator values so patterns can be identified.
    Decision {
        ts: String,
        symbol: String,
        action: String, // "BUY" | "SELL" | "SKIP"
        confidence: f64,
        rationale: String,
        // ── Key indicators ──────────────────────────────────
        rsi: f64,
        rsi_4h: f64,
        adx: f64,
        regime: String, // "Trending" | "Neutral" | "Ranging"
        macd: f64,
        macd_hist: f64,
        z_score: f64,
        z_score_4h: f64,
        ema_cross_pct: f64,
        atr: f64,
        atr_expansion: f64,
        bb_width_pct: f64,
        volume_ratio: f64,
        vwap_pct: f64,
        // ── Context ─────────────────────────────────────────
        sentiment_galaxy: Option<f64>,
        sentiment_bull: Option<f64>,
        funding_rate: Option<f64>,
        funding_delta: Option<f64>,
        btc_dom_pct: f64,
        asset_ret_4h: f64,
        entry_price: f64,
        stop_loss: f64,
        take_profit: f64,
        leverage: f64,
        skip_reason: Option<String>,
    },

    /// A new position was opened.
    TradeEntry {
        ts: String,
        symbol: String,
        side: String, // "LONG" | "SHORT"
        entry_price: f64,
        size_usd: f64,
        leverage: f64,
        notional_usd: f64,
        stop_loss: f64,
        take_profit: f64,
        r_risk_usd: f64,
        confidence: f64,
        rationale: String,
        in_circuit_breaker: bool,
        portfolio_heat_pct: f64,
        kelly_pct: f64,
    },

    /// A position was fully closed.
    TradeExit {
        ts: String,
        symbol: String,
        side: String,
        entry_price: f64,
        exit_price: f64,
        size_usd: f64,
        pnl_usd: f64,
        pnl_pct: f64,
        r_multiple: f64,
        reason: String, // "StopLoss" | "TakeProfit" | "TimeExit" | "SignalExit" | "AI-Close"
        cycles_held: u32,
        minutes_held: u32,
        dca_count: u8,
        tranches_closed: u8,
    },

    /// 1/3 profit taken at 2R or 4R milestone.
    TradePartial {
        ts: String,
        symbol: String,
        side: String,
        exit_price: f64,
        size_closed_usd: f64,
        pnl_usd: f64,
        r_milestone: u8, // 2 or 4
        r_at_close: f64,
    },

    /// DCA add-on to an existing losing position.
    TradeDca {
        ts: String,
        symbol: String,
        side: String,
        dca_price: f64,
        avg_entry: f64,
        add_size_usd: f64,
        dca_count: u8,
        r_before_dca: f64,
        new_stop: f64,
    },

    /// Pyramid add-on to a winning position.
    TradePyramid {
        ts: String,
        symbol: String,
        side: String,
        add_price: f64,
        add_size_usd: f64,
        r_at_pyramid: f64,
        total_size_usd: f64,
    },

    /// Circuit breaker activated or cleared.
    CircuitBreaker {
        ts: String,
        activated: bool, // true = just triggered, false = just cleared
        drawdown_pct: f64,
        threshold_pct: f64,
        size_mult: f64, // 0.35 when active, 1.0 when clear
        peak_equity: f64,
        current_equity: f64,
    },

    /// Portfolio performance snapshot (emitted every 10 cycles).
    MetricsSnapshot {
        ts: String,
        cycle_number: u64,
        total_trades: usize,
        win_rate: f64,
        expectancy_pct: f64,
        sharpe: f64,
        sortino: f64,
        max_drawdown_pct: f64,
        profit_factor: f64,
        kelly_fraction: f64,
        total_pnl_usd: f64,
        capital: f64,
        open_positions: usize,
    },

    /// AI guardrail evaluation for a close recommendation.
    AiGuardrailCheck {
        ts: String,
        symbol: String,
        action: String,
        recommendation: String,
        guardrail_score: f64,
        guardrail_components: Vec<String>,
        guardrail_note: Option<String>,
        r_multiple: f64,
        hold_minutes: u64,
        dca_remaining: u8,
        false_breakout: bool,
        momentum_stall: bool,
        entry_confidence: f64,
        signal_summary: String,
        signal_breakdown: String,
        signal_alignment_pct: f64,
        funding_phase: String,
        hours_to_settlement: f64,
        order_flow_confidence: f64,
        order_flow_direction: String,
        order_flow_snapshot: String,
        ob_sentiment: String,
        ob_adverse_cycles: u32,
        funding_rate: f64,
        funding_delta: f64,
        onchain_strength: f64,
        cex_premium_pct: f64,
        cex_mode: String,
        cross_exchange_snapshot: String,
        prompt_tokens: Option<u32>,
        completion_tokens: Option<u32>,
        total_tokens: Option<u32>,
        guardrail_allowed: bool,
    },

    /// End-of-day summary — emitted once at midnight UTC.
    DayClose {
        ts: String,
        date: String, // "YYYY-MM-DD"
        total_decisions: u64,
        total_entries: u64,
        total_exits: u64,
        total_partials: u64,
        total_dcas: u64,
        day_pnl_usd: f64,
        day_win_rate: f64,
        top_symbol: String,
        worst_symbol: String,
        cb_activations: u64,
        final_capital: f64,
        start_capital: f64,
    },
}

#[allow(dead_code)]
impl TradeEvent {
    /// ISO 8601 timestamp for this event.
    pub fn ts(&self) -> &str {
        match self {
            Self::CycleStart { ts, .. } => ts,
            Self::Decision { ts, .. } => ts,
            Self::TradeEntry { ts, .. } => ts,
            Self::TradeExit { ts, .. } => ts,
            Self::TradePartial { ts, .. } => ts,
            Self::TradeDca { ts, .. } => ts,
            Self::TradePyramid { ts, .. } => ts,
            Self::CircuitBreaker { ts, .. } => ts,
            Self::MetricsSnapshot { ts, .. } => ts,
            Self::AiGuardrailCheck { ts, .. } => ts,
            Self::DayClose { ts, .. } => ts,
        }
    }
}

// ─────────────────────────── Logger ──────────────────────────────────────────

/// Thread-safe daily JSONL logger.
///
/// One file per UTC day: `logs/trading_YYYY-MM-DD.jsonl`.  The file rotates
/// automatically — the logger checks the date on every write.
#[derive(Debug)]
pub struct TradeLogger {
    log_dir: PathBuf,
    current_date: String,
    file: Option<std::fs::File>,
    /// Running counters for the current day (used for `DayClose` summary).
    pub day_stats: DayStats,
}

/// Lightweight per-day counters maintained by the logger.
#[derive(Debug, Default, Clone)]
pub struct DayStats {
    pub decisions: u64,
    pub entries: u64,
    pub exits: u64,
    pub partials: u64,
    pub dcas: u64,
    pub cb_fires: u64,
    pub day_pnl: f64,
    pub day_wins: u64,
    pub day_losses: u64,
    pub start_capital: f64,
    pub best_symbol: String,
    pub worst_symbol: String,
    pub best_pnl: f64,
    pub worst_pnl: f64,
}

pub type SharedTradeLogger = Arc<Mutex<TradeLogger>>;

impl TradeLogger {
    /// Create a new logger.  `log_dir` is created if it doesn't exist.
    pub fn new(log_dir: impl AsRef<Path>) -> Result<Self> {
        let log_dir = log_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&log_dir)?;

        let mut logger = TradeLogger {
            log_dir,
            current_date: String::new(),
            file: None,
            day_stats: DayStats::default(),
        };
        logger.rotate_if_needed()?;
        Ok(logger)
    }

    /// Wrap in `Arc<Mutex<>>` for shared async access.
    pub fn shared(log_dir: impl AsRef<Path>) -> Result<SharedTradeLogger> {
        Ok(Arc::new(Mutex::new(Self::new(log_dir)?)))
    }

    /// Append a `TradeEvent` to the current day's log file.
    ///
    /// Automatically rotates the file if the UTC date has changed since the
    /// last write.  Errors are printed to stderr rather than propagated so a
    /// log failure never crashes the trading loop.
    pub fn log(&mut self, event: &TradeEvent) {
        if let Err(e) = self.log_inner(event) {
            log::warn!("TradeLogger write error: {}", e);
        }
    }

    fn log_inner(&mut self, event: &TradeEvent) -> Result<()> {
        self.rotate_if_needed()?;

        // Update day stats
        match event {
            TradeEvent::Decision { .. } => self.day_stats.decisions += 1,
            TradeEvent::TradeEntry { .. } => self.day_stats.entries += 1,
            TradeEvent::TradeExit {
                pnl_usd, symbol, ..
            } => {
                self.day_stats.exits += 1;
                self.day_stats.day_pnl += pnl_usd;
                if *pnl_usd > 0.0 {
                    self.day_stats.day_wins += 1;
                } else {
                    self.day_stats.day_losses += 1;
                }
                if *pnl_usd > self.day_stats.best_pnl {
                    self.day_stats.best_pnl = *pnl_usd;
                    self.day_stats.best_symbol = symbol.clone();
                }
                if *pnl_usd < self.day_stats.worst_pnl {
                    self.day_stats.worst_pnl = *pnl_usd;
                    self.day_stats.worst_symbol = symbol.clone();
                }
            }
            TradeEvent::TradePartial { .. } => self.day_stats.partials += 1,
            TradeEvent::TradeDca { .. } => self.day_stats.dcas += 1,
            TradeEvent::CircuitBreaker { activated, .. } => {
                if *activated {
                    self.day_stats.cb_fires += 1;
                }
            }
            _ => {}
        }

        let line = serde_json::to_string(event)?;
        if let Some(f) = self.file.as_mut() {
            writeln!(f, "{}", line)?;
            f.flush()?;
        }
        Ok(())
    }

    /// Check if the UTC date has changed and open a new file if so.
    fn rotate_if_needed(&mut self) -> Result<()> {
        let today = Utc::now().format("%Y-%m-%d").to_string();
        if today != self.current_date {
            let path = self.log_dir.join(format!("trading_{}.jsonl", today));
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)?;
            self.file = Some(file);
            self.current_date = today.clone();
            // Reset day counters on rotation (keep start_capital from caller)
            let saved_start = self.day_stats.start_capital;
            self.day_stats = DayStats {
                start_capital: saved_start,
                ..Default::default()
            };
            log::info!("📁 Daily log opened: {}", path.display());
        }
        Ok(())
    }

    /// Returns the path of yesterday's log file (used by the daily analyst).
    #[allow(dead_code)]
    pub fn yesterday_log_path(&self) -> PathBuf {
        let yesterday = (Utc::now() - chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        self.log_dir.join(format!("trading_{}.jsonl", yesterday))
    }

    /// Returns the path of a specific day's log file.
    pub fn log_path_for(&self, date: &str) -> PathBuf {
        self.log_dir.join(format!("trading_{}.jsonl", date))
    }

    /// Returns the log directory path.
    pub fn log_dir(&self) -> &Path {
        &self.log_dir
    }
}

// ─────────────────────────── Helpers ─────────────────────────────────────────

/// ISO 8601 timestamp string in UTC with millisecond precision.
pub fn ts_now() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// Current UTC date string "YYYY-MM-DD".
pub fn date_today() -> String {
    Utc::now().format("%Y-%m-%d").to_string()
}

/// Yesterday's UTC date string "YYYY-MM-DD".
pub fn date_yesterday() -> String {
    (Utc::now() - chrono::Duration::days(1))
        .format("%Y-%m-%d")
        .to_string()
}
