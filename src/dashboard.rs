//! 📊 Terminal UI Dashboard (TUI)
//! Real-time monitoring of bot status, trades, AI thinking, sentiment
//! ncurses-like interface using ratatui
//!
//! Layout:
//! ┌────────────────────────────────────────────────────────┐
//! │ EQUITY │ W/L │ WIN% │ DRAWDOWN │ TRADES │ LAST TRADE  │
//! ├────────────────────────────────────────────────────────┤
//! │ CURRENT POSITION                                        │
//! │ Entry: $82.00 | Size: 0.134 SOL | P&L: +$45.20 (+1.2%)│
//! │ Stop: $59.40 | Target: $90.41 | Since: 2h 34m ago    │
//! ├────────────────────────────────────────────────────────┤
//! │ WHAT AI IS THINKING (Last Decision)                    │
//! │ Confidence: 82% | Signal: Mean Reversion + Divergence │
//! │ Frameworks: ✓Volatility ✓Timeframe ✓OrderFlow ✓Kelly  │
//! │ Reasoning: Multi-timeframe aligned, whale buying      │
//! ├────────────────────────────────────────────────────────┤
//! │ SENTIMENT & SIGNALS                                    │
//! │ Fear/Greed: 32 (Fear)   |   RSI: 28   |   MACD: ↑      │
//! │ Bid/Ask: 1.8x           |   ATR: 0.8% |   Confluence: 75%
//! ├────────────────────────────────────────────────────────┤
//! │ RECENT TRADES (Last 10)                                │
//! │ ✓ +$15.20 (1.5%) Mean Reversion @ 14:32     Divergence│
//! │ ✓ +$12.80 (1.2%) MACD Momentum @ 13:45      Support   │
//! │ ✗ -$8.50  (0.8%) Ichimoku @ 12:20           Volume    │
//! ├────────────────────────────────────────────────────────┤
//! │ ERROR LOG                                               │
//! │ [14:35] ⚠️  Slippage 0.15% on last trade                │
//! │ [14:20] ℹ️  Daily profit: +$52.30                       │
//! └────────────────────────────────────────────────────────┘

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc, Duration};

// ============================================================================
// DASHBOARD DATA STRUCTURES
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardMetrics {
    // Portfolio Status
    pub initial_equity: f64,
    pub current_equity: f64,
    pub daily_pnl: f64,
    pub total_pnl: f64,
    pub return_pct: f64,
    pub max_drawdown_pct: f64,

    // Trade Statistics
    pub total_trades: usize,
    pub winning_trades: usize,
    pub losing_trades: usize,
    pub win_rate: f64,
    pub average_win: f64,
    pub average_loss: f64,
    pub profit_factor: f64,

    // Current Position
    pub has_position: bool,
    pub position_symbol: Option<String>,
    pub position_entry_price: Option<f64>,
    pub position_current_price: Option<f64>,
    pub position_size: Option<f64>,
    pub position_leverage: Option<f64>,
    pub position_pnl: Option<f64>,
    pub position_pnl_pct: Option<f64>,
    pub position_time_open: Option<Duration>,
    pub position_stop_loss: Option<f64>,
    pub position_take_profit: Option<f64>,

    // Market State
    pub current_price: f64,
    pub support_level: f64,
    pub resistance_level: f64,
    pub atr_pct: f64,
    pub bid_ask_ratio: f64,
    pub fear_greed_index: i32, // 0-100, 0=extreme fear, 100=extreme greed
    pub rsi: f64,
    pub macd_signal: bool, // true=bullish, false=bearish
    pub confluence_score: f64,

    // Last Decision
    pub last_decision_time: DateTime<Utc>,
    pub last_decision_action: String, // "ENTRY", "SKIP", "EXIT", etc
    pub last_decision_confidence: f64,
    pub last_decision_reasoning: Vec<String>,
    pub last_decision_frameworks: Vec<String>, // which frameworks approved

    // System Health
    pub uptime_hours: f64,
    pub memory_usage_mb: u64,
    pub active_alerts: Vec<SystemAlert>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemAlert {
    pub level: AlertLevel,
    pub timestamp: DateTime<Utc>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum AlertLevel {
    Info,
    Warning,
    Error,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentTrade {
    pub timestamp: DateTime<Utc>,
    pub symbol: String,
    pub action: String, // "BUY", "SELL", "SHORT", "COVER"
    pub entry_price: f64,
    pub exit_price: Option<f64>,
    pub size: f64,
    pub pnl: f64,
    pub pnl_pct: f64,
    pub strategy: String,
    pub confidence: f64,
    pub status: String, // "OPEN", "CLOSED", "STOPPED", "TP_HIT"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AIThoughts {
    pub current_signal: String,
    pub technical_confidence: f64,
    pub framework_validation: Vec<(String, bool)>, // (framework_name, passed)
    pub order_flow_strength: f64,
    pub volatility_regime: String,
    pub timeframe_alignment: f64,
    pub reasoning: Vec<String>,
    pub warnings: Vec<String>,
    pub next_check_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SentimentMetrics {
    pub fear_greed_index: i32, // 0-100
    pub fear_greed_label: String, // "Extreme Fear", "Fear", "Neutral", "Greed", "Extreme Greed"
    pub market_sentiment: String, // "Bearish", "Neutral", "Bullish"
    pub volatility_regime: String,
    pub whale_activity: String, // "Selling Pressure", "Neutral", "Buying Pressure"
    pub rsi_signal: String, // "Oversold", "Neutral", "Overbought"
    pub macd_signal: String, // "Bullish", "Bearish"
}

// ============================================================================
// DASHBOARD BUILDER
// ============================================================================

pub struct DashboardBuilder;

impl DashboardBuilder {
    /// Build formatted dashboard metrics for display
    pub fn format_equity_header(metrics: &DashboardMetrics) -> String {
        format!(
            "EQUITY: ${:.2} | P&L: ${:.2} ({:.2}%) | W/L: {}/{} | Win Rate: {:.1}% | Max DD: {:.1}%",
            metrics.current_equity,
            metrics.total_pnl,
            metrics.return_pct,
            metrics.winning_trades,
            metrics.losing_trades,
            metrics.win_rate * 100.0,
            metrics.max_drawdown_pct
        )
    }

    pub fn format_position_info(metrics: &DashboardMetrics) -> String {
        match (
            &metrics.position_symbol,
            metrics.position_current_price,
            metrics.position_pnl,
            metrics.position_pnl_pct,
        ) {
            (Some(symbol), Some(price), Some(pnl), Some(pnl_pct)) => {
                let pnl_color = if pnl >= 0.0 { "🟢" } else { "🔴" };
                let time_str = match metrics.position_time_open {
                    Some(duration) => {
                        let hours = duration.num_hours();
                        let minutes = duration.num_minutes() % 60;
                        if hours > 0 {
                            format!("{}h {}m", hours, minutes)
                        } else {
                            format!("{}m", minutes)
                        }
                    }
                    None => "N/A".to_string(),
                };

                format!(
                    "POSITION: {} {} @ ${:.2} | Current: ${:.2} | {} P&L: ${:.2} ({:.2}%) | Opened: {}",
                    pnl_color,
                    symbol,
                    metrics.position_entry_price.unwrap_or(0.0),
                    price,
                    pnl_color,
                    pnl.abs(),
                    pnl_pct.abs(),
                    time_str
                )
            }
            _ => "POSITION: None (Monitoring for entry signals)".to_string(),
        }
    }

    pub fn format_market_state(metrics: &DashboardMetrics) -> String {
        let sentiment = if metrics.fear_greed_index < 25 {
            "🔴 EXTREME FEAR"
        } else if metrics.fear_greed_index < 45 {
            "🟠 FEAR"
        } else if metrics.fear_greed_index < 55 {
            "🟡 NEUTRAL"
        } else if metrics.fear_greed_index < 75 {
            "🟢 GREED"
        } else {
            "🟢 EXTREME GREED"
        };

        let rsi_status = if metrics.rsi < 30.0 {
            "🔵 Oversold"
        } else if metrics.rsi > 70.0 {
            "🔴 Overbought"
        } else {
            "⚪ Neutral"
        };

        let macd_arrow = if metrics.macd_signal { "↑ Bullish" } else { "↓ Bearish" };

        format!(
            "MARKET: Price ${:.2} | Support ${:.2} | Resistance ${:.2} | ATR: {:.2}% | Confluence: {:.0}%\n\
             SENTIMENT: {} ({}) | RSI: {:.0} {} | MACD: {} | Bid/Ask: {:.2}x",
            metrics.current_price,
            metrics.support_level,
            metrics.resistance_level,
            metrics.atr_pct,
            metrics.confluence_score * 100.0,
            sentiment,
            metrics.fear_greed_index,
            metrics.rsi,
            rsi_status,
            macd_arrow,
            metrics.bid_ask_ratio
        )
    }

    pub fn format_ai_thinking(thoughts: &AIThoughts) -> String {
        let mut output = format!(
            "AI DECISION ENGINE\n\
             Signal: {} | Confidence: {:.0}%\n\
             Frameworks:",
            thoughts.current_signal,
            thoughts.technical_confidence * 100.0
        );

        for (framework, passed) in &thoughts.framework_validation {
            let status = if *passed { "✓" } else { "✗" };
            output.push_str(&format!(" {}[{}]", status, framework));
        }

        output.push_str("\n\nReasoning:");
        for reason in &thoughts.reasoning {
            output.push_str(&format!("\n  • {}", reason));
        }

        if !thoughts.warnings.is_empty() {
            output.push_str("\n\nWarnings:");
            for warning in &thoughts.warnings {
                output.push_str(&format!("\n  ⚠️  {}", warning));
            }
        }

        output.push_str(&format!(
            "\n\nNext decision in: {:.0}s",
            thoughts.next_check_seconds
        ));

        output
    }

    pub fn format_recent_trades(trades: &[RecentTrade], limit: usize) -> String {
        let mut output = format!("RECENT TRADES (Last {})\n", limit.min(trades.len()));
        output.push_str("─────────────────────────────────────────────────────\n");

        for trade in trades.iter().rev().take(limit) {
            let status_icon = match trade.status.as_str() {
                "CLOSED" => {
                    if trade.pnl >= 0.0 {
                        "✓"
                    } else {
                        "✗"
                    }
                }
                "OPEN" => "▶",
                "STOPPED" => "⏹",
                "TP_HIT" => "🎯",
                _ => "?",
            };

            let pnl_color = if trade.pnl >= 0.0 { "🟢" } else { "🔴" };
            let time_str = trade.timestamp.format("%H:%M").to_string();

            output.push_str(&format!(
                "{} {} | {} | ${:.2} → ${:.2} ({} {:.0} lots @ {:.0}% confidence) | {}\n",
                status_icon,
                pnl_color,
                time_str,
                trade.entry_price,
                trade.exit_price.unwrap_or(trade.entry_price),
                trade.action,
                trade.size,
                trade.confidence * 100.0,
                trade.strategy
            ));
        }

        output
    }

    pub fn format_system_status(metrics: &DashboardMetrics) -> String {
        let mut output = format!(
            "SYSTEM STATUS\n\
             Uptime: {:.1}h | Memory: {}MB | Active Alerts: {}\n",
            metrics.uptime_hours,
            metrics.memory_usage_mb,
            metrics.active_alerts.len()
        );

        if !metrics.active_alerts.is_empty() {
            output.push_str("─────────────────────────────────────────────────────\n");
            for alert in metrics.active_alerts.iter().rev().take(5) {
                let icon = match alert.level {
                    AlertLevel::Info => "ℹ️ ",
                    AlertLevel::Warning => "⚠️ ",
                    AlertLevel::Error => "❌",
                    AlertLevel::Critical => "🚨",
                };
                output.push_str(&format!(
                    "{} [{}] {}\n",
                    icon,
                    alert.timestamp.format("%H:%M:%S"),
                    alert.message
                ));
            }
        }

        if let Some(error) = &metrics.last_error {
            output.push_str(&format!("Last Error: {}\n", error));
        }

        output
    }
}

// ============================================================================
// SENTIMENT ANALYZER
// ============================================================================

pub struct SentimentAnalyzer;

impl SentimentAnalyzer {
    pub fn analyze(metrics: &DashboardMetrics) -> SentimentMetrics {
        let fear_greed_label = match metrics.fear_greed_index {
            0..=25 => "Extreme Fear".to_string(),
            26..=45 => "Fear".to_string(),
            46..=54 => "Neutral".to_string(),
            55..=75 => "Greed".to_string(),
            _ => "Extreme Greed".to_string(),
        };

        let market_sentiment = match metrics.fear_greed_index {
            0..=35 => "Bearish (Opportunity zone)".to_string(),
            36..=64 => "Neutral".to_string(),
            _ => "Bullish".to_string(),
        };

        let volatility_regime = if metrics.atr_pct < 0.5 {
            "Calm".to_string()
        } else if metrics.atr_pct < 2.0 {
            "Normal".to_string()
        } else if metrics.atr_pct < 5.0 {
            "Volatile".to_string()
        } else {
            "Panic".to_string()
        };

        let whale_activity = if metrics.bid_ask_ratio > 1.8 {
            "Buying Pressure".to_string()
        } else if metrics.bid_ask_ratio < 0.6 {
            "Selling Pressure".to_string()
        } else {
            "Neutral".to_string()
        };

        let rsi_signal = if metrics.rsi < 30.0 {
            "Oversold (Entry setup)".to_string()
        } else if metrics.rsi > 70.0 {
            "Overbought (Caution)".to_string()
        } else {
            "Neutral".to_string()
        };

        let macd_signal = if metrics.macd_signal {
            "Bullish ↑".to_string()
        } else {
            "Bearish ↓".to_string()
        };

        SentimentMetrics {
            fear_greed_index: metrics.fear_greed_index,
            fear_greed_label,
            market_sentiment,
            volatility_regime,
            whale_activity,
            rsi_signal,
            macd_signal,
        }
    }

    pub fn format_sentiment(sentiment: &SentimentMetrics) -> String {
        format!(
            "MARKET SENTIMENT & ANALYSIS\n\
             ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n\
             Fear/Greed Index: {} ({}) [Opportunity Zone: <35]\n\
             Market Sentiment: {}\n\
             Volatility Regime: {}\n\
             Whale Activity: {}\n\
             RSI Signal: {}\n\
             MACD Signal: {}\n\
             ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━",
            sentiment.fear_greed_index,
            sentiment.fear_greed_label,
            sentiment.market_sentiment,
            sentiment.volatility_regime,
            sentiment.whale_activity,
            sentiment.rsi_signal,
            sentiment.macd_signal
        )
    }
}

// ============================================================================
// DASHBOARD AGGREGATOR (What gets sent to both web and TUI)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompleteDashboard {
    pub metrics: DashboardMetrics,
    pub sentiment: SentimentMetrics,
    pub ai_thoughts: AIThoughts,
    pub recent_trades: Vec<RecentTrade>,
    pub timestamp: DateTime<Utc>,
}

impl CompleteDashboard {
    pub fn build(
        metrics: DashboardMetrics,
        ai_thoughts: AIThoughts,
        recent_trades: Vec<RecentTrade>,
    ) -> Self {
        let sentiment = SentimentAnalyzer::analyze(&metrics);

        Self {
            metrics,
            sentiment,
            ai_thoughts,
            recent_trades,
            timestamp: Utc::now(),
        }
    }

    pub fn as_terminal_string(&self) -> String {
        let mut output = String::new();

        output.push_str("╔════════════════════════════════════════════════════════════════════╗\n");
        output.push_str("║              🤖 REDROBOT TRADING BOT - LIVE DASHBOARD 🤖            ║\n");
        output.push_str("╚════════════════════════════════════════════════════════════════════╝\n\n");

        // Equity header
        output.push_str(&format!("📊 {}\n\n", DashboardBuilder::format_equity_header(&self.metrics)));

        // Current position
        output.push_str(&format!("📍 {}\n\n", DashboardBuilder::format_position_info(&self.metrics)));

        // Market state
        output.push_str(&format!("📈 {}\n\n", DashboardBuilder::format_market_state(&self.metrics)));

        // Sentiment
        output.push_str(&format!("{}\n\n", SentimentAnalyzer::format_sentiment(&self.sentiment)));

        // AI thinking
        output.push_str(&format!("🧠 {}\n\n", DashboardBuilder::format_ai_thinking(&self.ai_thoughts)));

        // Recent trades
        output.push_str(&format!("{}\n\n", DashboardBuilder::format_recent_trades(&self.recent_trades, 10)));

        // System status
        output.push_str(&format!("⚙️  {}\n", DashboardBuilder::format_system_status(&self.metrics)));

        output
    }
}
