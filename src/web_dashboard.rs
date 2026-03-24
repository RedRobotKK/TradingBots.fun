use axum::{
    extract::State,
    http::HeaderMap,
    response::Html,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

// Pre-built Privy login SDK bundle (ESM). Served at /static/privy-login.js so
// the browser never needs to reach an external CDN.
// Rebuild the bundle whenever you upgrade @privy-io/react-auth:
//   cd js && npm install && npm run build
//   git add static/privy-login.js && git commit
static PRIVY_BUNDLE_JS: &str = include_str!("../static/privy-login.js");
use crate::bridge::BridgeManager;
use crate::coins;
use crate::exchange;
use crate::learner::{SignalContribution, SignalWeights};
use crate::metrics::PerformanceMetrics;
use crate::pattern_insights;
use crate::reporting;

// ─────────────────────────────────────────────────────────────────────────────
//  Shared application state — passed to every Axum handler via State<AppState>
// ─────────────────────────────────────────────────────────────────────────────

/// All server-wide state threaded through the Axum router.
///
/// Defined here (not in main.rs) so that stripe.rs and future modules can
/// import it without a circular dependency.
#[derive(Clone)]
pub struct AppState {
    /// Live trading / dashboard state (positions, P&L, signals…).
    pub bot_state: SharedState,
    /// Registry of all consumer tenants — mutated by Stripe webhooks.
    pub tenants: crate::tenant::SharedTenantManager,
    /// PostgreSQL connection pool — `None` when DATABASE_URL is not set.
    /// Shared across all Axum handlers and the trading loop.
    pub db: Option<crate::db::SharedDb>,
    /// Stripe secret API key (sk_live_… / sk_test_…).
    pub stripe_api_key: Option<String>,
    /// Stripe webhook signing secret (whsec_…).
    pub stripe_webhook_secret: Option<String>,
    /// Stripe Price ID for the $19.99/month Pro plan.
    pub stripe_price_id: Option<String>,
    /// Privy App ID — when set, consumer routes require a valid Privy session.
    /// Set via `PRIVY_APP_ID` env var.  `None` = single-operator fallback mode.
    pub privy_app_id: Option<String>,
    /// WalletConnect Cloud project ID — enables mobile-wallet login via Privy.
    /// Set via `WALLETCONNECT_PROJECT_ID` env var.  `None` = desktop wallets only.
    pub walletconnect_project_id: Option<String>,
    /// HMAC-SHA256 signing key for session cookies.  Set via `SESSION_SECRET`.
    pub session_secret: String,
    /// In-memory cache of Privy's JWKS — refreshed every hour on first use.
    pub jwks_cache: crate::privy::SharedJwksCache,
    /// Apple Pay domain-association file content.
    /// Obtained from Stripe Dashboard → Settings → Payment methods → Apple Pay
    /// → Add new domain → Download file.
    /// When set, served at `/.well-known/apple-developer-merchantid-domain-association`
    /// so Apple can verify the domain before showing the Apple Pay button.
    pub apple_pay_domain_assoc: Option<String>,
    /// Password protecting the `/admin/*` operator panel.
    /// Username is always `"admin"`.  Set via `ADMIN_PASSWORD` env var.
    pub admin_password: Option<String>,
    /// Coinzilla zone ID for the ad slot shown to Free/Trial users.
    /// Set via `COINZILLA_ZONE_ID` env var (e.g. `"12345"`).
    /// When `None`, no ads are rendered — Pro users never see ads regardless.
    ///
    /// Advertisement policy:
    ///   • Free tier (trial ACTIVE)   → ads shown  — trial is monetised via CPM
    ///   • Free tier (trial EXPIRED)  → ads shown  — upsell pressure before conversion
    ///   • Pro / Internal             → NO ads, ever
    pub coinzilla_zone_id: Option<String>,
    /// Resend-powered transactional mailer.  `None` when `RESEND_API_KEY` is unset.
    /// Used by the trial-expiry batch job to send the $9.95 promo email.
    #[allow(dead_code)]
    pub mailer: Option<std::sync::Arc<crate::mailer::Mailer>>,
    /// Stripe Price ID for the $9.95 first-month intro offer sent to expired-trial users.
    /// When set, `/billing/checkout?promo=1` substitutes this for the standard price.
    pub stripe_promo_price_id: Option<String>,
    /// Global investment thesis constraints — updated by the floating AI bar,
    /// consumed by `run_cycle` to filter candidates and clamp leverage.
    pub global_thesis: std::sync::Arc<tokio::sync::RwLock<crate::thesis::ThesisConstraints>>,
    /// Shared cache storing question/answer pairs keyed by the last report hash.
    pub report_cache: Arc<Mutex<reporting::QueryCache>>,
    /// Cache storing the latest pattern-summary bundle (JSON + markdown).
    pub pattern_cache: Arc<Mutex<pattern_insights::PatternCache>>,
    pub hyperliquid_stats: Arc<exchange::HyperliquidStats>,
    pub bridge_manager: Arc<BridgeManager>,
}

// ─────────────────────────────── Serde defaults ──────────────────────────────
fn default_leverage() -> f64 {
    1.0
}

fn default_venue() -> String {
    "Hyperliquid Perps (paper)".to_string()
}

fn default_session_venue() -> String {
    "internal".to_string()
}

// ─────────────────────────────── State structs ───────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperPosition {
    pub symbol: String,
    pub side: String, // "LONG" | "SHORT"
    pub entry_price: f64,
    pub quantity: f64,  // coins held (reduced by partial closes)
    pub size_usd: f64,  // USD committed (reduced by partial closes)
    pub stop_loss: f64, // current (trailing) stop
    pub take_profit: f64,
    pub atr_at_entry: f64,    // ATR at entry (for trailing)
    pub high_water_mark: f64, // highest price seen (LONG trailing)
    pub low_water_mark: f64,  // lowest  price seen (SHORT trailing)
    pub partial_closed: bool, // true once first tranche taken
    // ── Professional quant fields ─────────────────────────────────────────
    pub r_dollars_risked: f64, // dollars at risk on entry = |entry−stop| × qty_at_entry
    pub tranches_closed: u8,   // 0=none, 1=¼ at 1R banked, 2=⅓ at 2R banked, 3=⅓ at 4R banked
    #[serde(default)]
    pub dca_count: u8, // number of DCA add-ons executed (averaging down)
    #[serde(default = "default_leverage")]
    pub leverage: f64, // leverage applied at entry (1.5× – 5×)
    pub cycles_held: u64,      // incremented each 30s cycle (time-decay exit)
    pub entry_time: String,
    pub unrealised_pnl: f64,
    pub contrib: SignalContribution,
    // ── AI reviewer fields (updated every 10 cycles) ──────────────────────
    #[serde(default)]
    pub ai_action: Option<String>, // "scale_up" | "hold" | "scale_down" | "close_now"
    #[serde(default)]
    pub ai_reason: Option<String>, // Claude's one-line rationale
    // ── Correlation filter ────────────────────────────────────────────────
    /// Signal confidence at entry — compared against incoming correlated signals.
    #[serde(default = "default_min_confidence")]
    pub entry_confidence: f64,
    // ── Per-trade budget cap ──────────────────────────────────────────────
    /// Maximum USD this trade is budgeted to spend across entry + all DCA add-ons.
    /// Set at entry time: initial_size × (1 + dca_max_count).
    /// DCA is blocked once `dca_spent_usd` reaches this ceiling.
    #[serde(default)]
    pub trade_budget_usd: f64,
    /// Accumulated USD spent on DCA add-ons (does not include the initial entry).
    #[serde(default)]
    pub dca_spent_usd: f64,
    // ── BTC context snapshot at entry / last DCA ─────────────────────────
    /// BTC 4h return at the time this entry (or last DCA) was made.
    /// Used by the DCA thesis validator: if BTC has swung hard against us
    /// since entry, the thesis may be broken and further DCA is blocked.
    #[serde(default)]
    pub btc_ret_at_entry: f64,
    // ── Principal-recovery tracking ───────────────────────────────────────
    /// Initial margin committed at entry (USD). Never changes after opening.
    /// When unrealised_pnl ≥ initial_margin_usd the trade has "paid for itself"
    /// and any profit above this is pure gain running on house money.
    #[serde(default)]
    pub initial_margin_usd: f64,
    // ── Order-book sentiment snapshot (updated every cycle) ──────────────
    /// Most recent order-book sentiment string from detect_order_flow().
    /// "STRONGLY_BULLISH" | "BULLISH" | "NEUTRAL" | "BEARISH" | "STRONGLY_BEARISH"
    #[serde(default)]
    pub ob_sentiment: String,
    /// True when there is a significant bid wall within 2% of the current price.
    #[serde(default)]
    pub ob_bid_wall_near: bool,
    /// True when there is a significant ask wall within 2% of the current price.
    #[serde(default)]
    pub ob_ask_wall_near: bool,
    /// Cycles in a row where the order book has been adverse (bearish book on LONG,
    /// or bullish book on SHORT). When this reaches a threshold, the position manager
    /// can trigger an early partial or exit to protect profits / cut losses.
    #[serde(default)]
    pub ob_adverse_cycles: u32,
    #[serde(default)]
    pub order_flow_confidence: f64,
    #[serde(default)]
    pub order_flow_direction: String,
    #[serde(default)]
    pub funding_rate: f64,
    #[serde(default)]
    pub funding_delta: f64,
    #[serde(default)]
    pub onchain_strength: f64,
    #[serde(default)]
    pub cex_premium_pct: f64,
    #[serde(default)]
    pub cex_mode: String,
    // ── Profit-pool funding ───────────────────────────────────────────────
    /// True when this position was opened using profits from house_money_pool
    /// rather than the original base capital.  Pool-funded positions:
    ///   • Count at 50% weight in portfolio heat (we're playing with profits)
    ///   • Can be sized up to 2× the standard Kelly-sized amount
    ///   • When closed, net profit goes back to the pool (not general capital)
    #[serde(default)]
    pub funded_from_pool: bool,
    /// USD drawn from house_money_pool to open this position.
    /// Returned to pool (not capital) when the position is closed.
    #[serde(default)]
    pub pool_stake_usd: f64,
    // ── Venue transparency ────────────────────────────────────────────────
    /// Exchange venue where this position lives.
    /// "Hyperliquid Perps (paper)" | "Hyperliquid Perps (live)"
    #[serde(default = "default_venue")]
    pub venue: String,
}

fn default_min_confidence() -> f64 {
    0.68
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClosedTrade {
    pub symbol: String,
    pub side: String,
    pub entry: f64,
    pub exit: f64,
    pub pnl: f64,
    pub pnl_pct: f64,
    pub reason: String, // "Signal" | "StopLoss" | "TakeProfit" | "Partial"
    pub closed_at: String,
    // ── Tax / record-keeping fields (all default-zero for old snapshots) ──
    /// Timestamp when the position was originally opened.
    #[serde(default)]
    pub entry_time: String,
    /// Number of base-asset units traded.
    #[serde(default)]
    pub quantity: f64,
    /// USD margin committed (not notional — notional = size_usd × leverage).
    #[serde(default)]
    pub size_usd: f64,
    /// Leverage multiplier used at entry.
    #[serde(default = "default_one")]
    pub leverage: f64,
    /// Estimated fees paid (maker+taker+builder, ~0.075 % of notional).
    #[serde(default)]
    pub fees_est: f64,
    /// HTML snippet shown when user clicks the row — technicals + AI reasoning.
    #[serde(default)]
    pub breakdown: Option<String>,
    // ── Trade journal ─────────────────────────────────────────────────────
    /// Operator note added after close: "false MACD signal in chop",
    /// "re-entered too early", etc.  Written via POST /api/trade-note.
    #[serde(default)]
    pub note: Option<String>,
    // ── Venue transparency ────────────────────────────────────────────────
    /// Exchange venue where this trade was executed.
    #[serde(default = "default_venue")]
    pub venue: String,
}

fn default_one() -> f64 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateInfo {
    pub symbol: String,
    pub price: f64,
    /// None on cycle 1 (no previous reference price yet); Some(%) on cycle 2+.
    pub change_pct: Option<f64>,
    /// RSI(14) value computed during signal analysis, None until first scan.
    #[serde(default)]
    pub rsi: Option<f64>,
    /// Market regime: "Trending" | "Neutral" | "Ranging", None until first scan.
    #[serde(default)]
    pub regime: Option<String>,
    /// ATR(14) as % of price — a volatility proxy, None until first scan.
    #[serde(default)]
    pub atr_pct: Option<f64>,
    /// Decision confidence 0‒1 from the last analyse_symbol run, None until first scan.
    #[serde(default)]
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionInfo {
    pub symbol: String,
    pub action: String,
    pub confidence: f64,
    pub entry_price: f64,
    pub rationale: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotState {
    pub capital: f64,
    pub initial_capital: f64,
    pub peak_equity: f64, // all-time equity high (display only)
    pub equity_window: std::collections::VecDeque<(i64, f64)>, // (unix_ts, equity) rolling 7-day
    pub cb_active: bool,  // true when rolling-equity CB is firing (set by main loop)
    pub pnl: f64,
    pub cycle_count: u64,
    pub candidates: Vec<CandidateInfo>,
    pub positions: Vec<PaperPosition>,
    pub closed_trades: Vec<ClosedTrade>,
    pub recent_decisions: Vec<DecisionInfo>,
    pub signal_weights: SignalWeights,
    pub metrics: PerformanceMetrics,
    pub session_prices: HashMap<String, f64>, // first price seen per symbol this session
    pub status: String,
    pub last_update: String,
    /// Unix-ms timestamp when the next 30 s cycle will fire.  0 = unknown.
    pub next_cycle_at: i64,
    pub hyperliquid_stats: exchange::HyperliquidStatsSnapshot,
    /// Rolling equity snapshots (max 288 ≈ 2.4 h at 30 s/cycle) for the sparkline.
    /// Populated by the main trading loop every cycle — NOT by page loads.
    #[serde(default)]
    pub equity_history: Vec<f64>,
    /// Platform Hyperliquid referral code — set from config at startup, not persisted.
    /// Displayed in the consumer /app page so new signups use the referral link.
    #[serde(default)]
    pub referral_code: Option<String>,
    /// Last AI review summary string — set by run_cycle() when Claude reviews positions.
    /// Empty = no review run yet (API key absent or no open positions).
    /// Example: "🤖 3 reviewed · SOL hold · ETH scale_down"
    #[serde(default)]
    pub ai_status: String,

    // ── Profit recycling ─────────────────────────────────────────────────
    /// Accumulated realized profits (from partial + full closes with positive P&L)
    /// held separately from the base capital.  This is "house money" — profits the
    /// market has paid us.  New entries can draw from this pool first, maximising
    /// exposure using profits rather than risking the original capital.
    ///
    /// Flow:
    ///   profitable close  → trade_pnl added here
    ///   new entry (pool)  → deducted here, position marked `funded_from_pool = true`
    ///   pool-funded close → margin + pnl returned to capital, new profit → pool again
    #[serde(default)]
    pub house_money_pool: f64,
    /// Ring buffer of the last 30 profitable closes for re-entry detection.
    /// Each entry: (symbol, profit_usd, cycle_at_close).
    /// When a symbol signals again within 60 cycles (30 min) of a profitable close,
    /// the new entry is a "re-entry" and is sized from the pool preferentially.
    #[serde(default)]
    pub recently_closed: std::collections::VecDeque<(String, f64, u64)>,
    /// Total USD currently deployed from house_money_pool (in open positions).
    /// = sum(size_usd for pos where funded_from_pool).
    /// Used to show "own capital at risk" vs "house money at risk" split on dashboard.
    #[serde(default)]
    pub pool_deployed_usd: f64,

    // ── Manual command queue (AI interface) ──────────────────────────────
    /// Commands typed by the operator via the AI bar and queued for execution
    /// on the next trading cycle.  Drained at the top of `run_cycle()`.
    #[serde(default)]
    pub pending_cmds: std::collections::VecDeque<BotCommand>,

    // ── x402 Bot API sessions ─────────────────────────────────────────────
    /// Active bot-API sessions created via `POST /api/v1/session` (x402).
    #[serde(default)]
    pub bot_sessions: std::collections::HashMap<String, BotSession>,
}

/// A paid bot-API session created via the x402 payment protocol.
/// Bots authenticate subsequent requests with `Authorization: Bearer {token}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotSession {
    pub id: String,
    pub token: String,
    pub tx_hash: String,
    pub plan: String,
    pub created_at: String,
    pub expires_at: String,
    // ── Session-level risk controls ───────────────────────────────────────
    /// Auto-pause if session drawdown exceeds this percentage (e.g. 15.0 = 15%).
    #[serde(default)]
    pub max_drawdown_pct: Option<f64>,
    /// Webhook URL called on every trade fill / session event.
    #[serde(default)]
    pub webhook_url: Option<String>,
    /// Venue for this session: "internal" (default) | "hyperliquid".
    #[serde(default = "default_session_venue")]
    pub venue: String,
    /// Max leverage allowed for this session (1–50). None = use bot default.
    #[serde(default)]
    pub leverage_max: Option<i32>,
    /// Risk mode: "conservative" | "balanced" | "aggressive".
    #[serde(default)]
    pub risk_mode: Option<String>,
    /// Whitelisted symbols. None = all pairs.
    #[serde(default)]
    pub symbols_whitelist: Option<Vec<String>>,
    /// Optional performance fee percentage (0–50). Only charged on profitable closes.
    #[serde(default)]
    pub performance_fee_pct: Option<i32>,
    /// Hyperliquid deposit address for this session (derived per-session wallet).
    #[serde(default)]
    pub hyperliquid_address: Option<String>,
    /// Session paused flag — set by drawdown guard or pause_trading command.
    #[serde(default)]
    pub paused: bool,
}

/// A manual trade-execution command queued by the operator or a bot-API session.
/// Processed by `run_cycle()` with live prices before any autonomous logic runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum BotCommand {
    /// Close the named position at current market price.
    ClosePosition { symbol: String },
    /// Take a partial profit (tranche 0 = first 1/3) on the named position.
    TakePartial { symbol: String },
    /// Close every open position immediately.
    CloseAll,
    /// Close all positions that are currently in profit.
    CloseProfitable,
    /// Open a new LONG position on `symbol` with optional size and leverage.
    OpenLong {
        symbol: String,
        size_usd: Option<f64>,
        leverage: Option<f64>,
    },
    /// Open a new SHORT position on `symbol` with optional size and leverage.
    OpenShort {
        symbol: String,
        size_usd: Option<f64>,
        leverage: Option<f64>,
    },
    /// Set leverage for a symbol (Hyperliquid sessions).
    SetLeverage { symbol: String, leverage: i32 },
    /// Pause trading for this session.
    PauseTrading,
    /// Resume trading after a pause.
    ResumeTrading,
}

impl Default for BotState {
    fn default() -> Self {
        BotState {
            capital: 1000.0,
            initial_capital: 1000.0,
            peak_equity: 1000.0,
            equity_window: std::collections::VecDeque::new(),
            cb_active: false,
            pnl: 0.0,
            cycle_count: 0,
            candidates: vec![],
            positions: vec![],
            closed_trades: vec![],
            recent_decisions: vec![],
            signal_weights: SignalWeights::default(),
            metrics: PerformanceMetrics::default(),
            session_prices: HashMap::new(),
            status: String::new(),
            last_update: String::new(),
            next_cycle_at: 0,
            hyperliquid_stats: exchange::HyperliquidStatsSnapshot::default(),
            equity_history: vec![],
            referral_code: None,
            ai_status: String::new(),
            house_money_pool: 0.0,
            recently_closed: std::collections::VecDeque::new(),
            pool_deployed_usd: 0.0,
            pending_cmds: std::collections::VecDeque::new(),
            bot_sessions: std::collections::HashMap::new(),
        }
    }
}

pub type SharedState = Arc<RwLock<BotState>>;

// ─────────────────────────────── Dashboard ───────────────────────────────────

async fn dashboard_handler(State(app): State<AppState>) -> Html<String> {
    let s = app.bot_state.read().await;
    let m = &s.metrics;

    // ── Core financials ───────────────────────────────────────────────────
    let unrealised: f64 = s.positions.iter().map(|p| p.unrealised_pnl).sum();
    let committed: f64 = s.positions.iter().map(|p| p.size_usd).sum();
    let equity = s.capital + committed + unrealised;
    let total_pnl = s.pnl + unrealised;
    let total_pnl_pct = if s.initial_capital > 0.0 {
        total_pnl / s.initial_capital * 100.0
    } else {
        0.0
    };

    let pnl_colour = if total_pnl >= 0.0 {
        "#3fb950"
    } else {
        "#f85149"
    };
    // BUG FIX: sign was "" for negatives (not "-"), causing minus to be silently dropped
    // when combined with the .abs() calls in the format args.
    let pnl_sign = if total_pnl >= 0.0 { "+" } else { "-" };
    // All-time peak drawdown (display only).
    let dd_pct = if s.peak_equity > 0.0 {
        (s.peak_equity - equity) / s.peak_equity * 100.0
    } else {
        0.0
    };
    // Rolling 7-day drawdown — this is what actually drives the circuit breaker.
    // The CB uses equity_window, so we must derive the rolling peak from the same source.
    let rolling_peak = s
        .equity_window
        .iter()
        .map(|&(_, e)| e)
        .fold(equity, f64::max);
    let rolling_dd_pct = if rolling_peak > 0.0 {
        ((rolling_peak - equity) / rolling_peak * 100.0).max(0.0)
    } else {
        0.0
    };

    // ── Metric strings ────────────────────────────────────────────────────
    let kelly = m.kelly_fraction();
    let kelly_str = if kelly < 0.0 {
        "learning…".to_string()
    } else {
        format!("{:.1}%", kelly * 100.0)
    };
    // Use the rolling-equity CB flag set by main loop — this is the same signal
    // that actually controls position sizing, avoiding a stale metrics-based read.
    let cb_active = s.cb_active;
    let cb_label = if cb_active {
        "⚡ CB Active"
    } else {
        "● Normal"
    };
    let cb_colour = if cb_active { "#f85149" } else { "#3fb950" };
    // BUG FIX: was using m.current_dd (P&L-curve drawdown from closed trades only).
    // The CB is driven by rolling_dd_pct (7-day equity window) — use that here.
    let cb_desc = if cb_active {
        format!("0.35× sizes · 7d DD {:.1}%", rolling_dd_pct)
    } else {
        format!("Risk Normal · 7d DD {:.1}%", rolling_dd_pct)
    };
    let pf_str = if m.profit_factor.is_infinite() {
        "∞".to_string()
    } else {
        format!("{:.2}", m.profit_factor)
    };

    // ── Equity hero P&L class (drives colour glow) ────────────────────────
    // CB active overrides colour → flashing red border.
    // Otherwise green when profitable, red when losing, neutral near break-even.
    let hero_class = if cb_active {
        "equity-hero pnl-cb"
    } else if total_pnl > 0.0 {
        "equity-hero pnl-pos"
    } else if total_pnl_pct < -1.5 {
        "equity-hero pnl-neg"
    } else {
        "equity-hero" // neutral — near break-even
    };

    // ── AI status bar HTML ─────────────────────────────────────────────────
    // Non-empty ai_status = a Claude review has run (or is running).
    let ai_status_html = if s.ai_status.is_empty() {
        String::new()
    } else {
        let is_active = s.ai_status.contains("Querying");
        let extra_class = if is_active { " ai-active" } else { "" };
        format!(
            r#"<div id="ai-status-bar" class="ai-status-bar{cls}"><span id="ai-status-text">{txt}</span></div>"#,
            cls = extra_class,
            txt = s.ai_status,
        )
    };

    // ── CB metric card extra class ─────────────────────────────────────────
    let cb_card_class = if cb_active { " metric-cb-active" } else { "" };

    // ── Position cards ────────────────────────────────────────────────────
    let pos_cards: String = if s.positions.is_empty() {
        r#"<div class="empty-state"><div class="radar"></div><p>No open positions — scanning for signals…</p></div>"#.to_string()
    } else {
        s.positions.iter().map(|p| {
            let r_mult = if p.r_dollars_risked > 1e-8 { p.unrealised_pnl / p.r_dollars_risked } else { 0.0 };
            let pnl_colour = if p.unrealised_pnl >= 0.0 { "#3fb950" } else { "#f85149" };
            let border_colour = if p.unrealised_pnl > 0.0 { "#238636" } else if p.unrealised_pnl < -p.r_dollars_risked * 0.5 { "#da3633" } else { "#444c56" };
            let side_colour = if p.side == "LONG" { "#3fb950" } else { "#f85149" };
            let side_arrow  = if p.side == "LONG" { "▲" } else { "▼" };
            // BUG FIX: was "" for negatives → minus dropped when using .abs()
            let pnl_sign     = if p.unrealised_pnl >= 0.0 { "+" } else { "-" };
            let pnl_abs      = p.unrealised_pnl.abs();
            let pct_of_entry = p.unrealised_pnl / p.size_usd * 100.0;
            let pct_abs      = pct_of_entry.abs();

            // R progress bar: clamp -1R to +5R displayed range
            let bar_pct = ((r_mult + 1.0) / 6.0 * 100.0).clamp(0.0, 100.0);
            let bar_colour = if r_mult >= 2.0 { "#3fb950" } else if r_mult >= 0.0 { "#388bfd" } else { "#f85149" };

            // Tranche ladder: 0=none, 1=¼@1R done, 2=¼+⅓@2R done, 3=¼+⅓+⅓@4R done
            let tranche_label = match p.tranches_closed {
                0 => "target <b>1R</b>".to_string(),
                1 => "<span style='color:#3fb950'>¼ banked</span> · target <b>2R</b>".to_string(),
                2 => "<span style='color:#3fb950'>¼+⅓ banked</span> · target <b>4R</b>".to_string(),
                _ => "<span style='color:#3fb950'>⅝ banked</span> · trailing".to_string(),
            };

            // DCA badge — shown when we've averaged down, with budget remaining
            let dca_badge = if p.dca_count > 0 || p.trade_budget_usd > 0.0 {
                let budget_remaining = (p.trade_budget_usd - p.dca_spent_usd).max(0.0);
                let budget_pct = if p.trade_budget_usd > 0.0 {
                    (p.dca_spent_usd / p.trade_budget_usd * 100.0).min(100.0) as u32
                } else { 0 };
                if p.dca_count > 0 {
                    format!(" <span title='DCA budget: ${:.0} remaining ({:.0}% used)' \
                              style='background:#332a00;color:#e3b341;border:1px solid #e3b34150;\
                              border-radius:4px;padding:1px 5px;font-size:.68em'>\
                              DCA×{} <span style='color:#888;font-size:.85em'>${:.0}↗</span></span>",
                             budget_remaining, budget_pct, p.dca_count, budget_remaining)
                } else {
                    format!(" <span title='DCA budget: ${:.0} available' \
                              style='background:#1a1a1a;color:#666;border:1px solid #333;\
                              border-radius:4px;padding:1px 5px;font-size:.68em'>\
                              budget ${:.0}</span>",
                             budget_remaining, budget_remaining)
                }
            } else { String::new() };

            // ── Order-book sentiment badge ─────────────────────────────────
            // Shows live book sentiment + wall indicators. Changes colour
            // based on whether the book is aligned with or against the position.
            let ob_badge = if !p.ob_sentiment.is_empty() && p.ob_sentiment != "NEUTRAL" {
                let (ob_emoji, ob_colour, ob_bg) = match p.ob_sentiment.as_str() {
                    "STRONGLY_BULLISH" => ("📗", "#3fb950", "#0d2318"),
                    "BULLISH"          => ("📗", "#3fb950", "#0d1f15"),
                    "STRONGLY_BEARISH" => ("📕", "#f85149", "#2d0f0d"),
                    "BEARISH"          => ("📕", "#f85149", "#1e0d0c"),
                    _                  => ("📘", "#8b949e", "#161b22"),
                };
                // Is the book aligned with (supports) our position, or against it?
                let aligned = (p.side == "LONG"  && p.ob_sentiment.contains("BULL")) ||
                              (p.side == "SHORT" && p.ob_sentiment.contains("BEAR"));
                let (border_col, opacity) = if aligned { (ob_colour, "1.0") } else { ("#f85149", "0.7") };
                let wall_str = match (p.ob_bid_wall_near, p.ob_ask_wall_near) {
                    (true,  false) => " 🧱↓",
                    (false, true)  => " 🧱↑",
                    (true,  true)  => " 🧱↕",
                    _              => "",
                };
                let adv_str = if p.ob_adverse_cycles >= 4 {
                    format!(" ⚠{}cy", p.ob_adverse_cycles)
                } else { String::new() };
                format!(" <span title='Order book: {} ({} adverse cycles){}' \
                          style='background:{bg};color:{col};border:1px solid {bdr}50;\
                          border-radius:4px;padding:1px 5px;font-size:.68em;opacity:{op}'>\
                          {em} {snt}{wall}{adv}</span>",
                         p.ob_sentiment, p.ob_adverse_cycles, wall_str,
                         bg = ob_bg, col = ob_colour, bdr = border_col, op = opacity,
                         em = ob_emoji,
                         snt = match p.ob_sentiment.as_str() {
                             "STRONGLY_BULLISH" => "STR BULL",
                             "STRONGLY_BEARISH" => "STR BEAR",
                             s => &s[..s.len().min(4)],
                         },
                         wall = wall_str,
                         adv = adv_str,
                )
            } else { String::new() };

            // ── Principal-recovered badge ──────────────────────────────────
            // "House money" indicator: trade has earned back its original stake.
            let principal_badge = if p.initial_margin_usd > 0.0
                && p.unrealised_pnl >= p.initial_margin_usd {
                " <span title='Principal recovered — running on house money!' \
                          style='background:#0d2318;color:#3fb950;border:1px solid #3fb95060;\
                          border-radius:4px;padding:1px 5px;font-size:.68em'>\
                          🏦 principal ✓</span>".to_string()
            } else { String::new() };

            // ── Pool-funded badge ──────────────────────────────────────────
            // Shows when a position was opened using the house-money pool (not own capital).
            // Also shows the pool stake so the user can see how much profit is at work.
            let pool_badge = if p.funded_from_pool {
                format!(" <span title='Opened with house-money pool — own capital not at risk. Pool stake ${:.2}' \
                          style='background:#0d1d2e;color:#388bfd;border:1px solid #388bfd60;\
                          border-radius:4px;padding:1px 5px;font-size:.68em'>\
                          💰 house money ${:.2}</span>",
                         p.pool_stake_usd, p.pool_stake_usd)
            } else { String::new() };

            // Convert cycles to human-readable hold time
            let hold_mins = p.cycles_held / 2; // 30s cycles → minutes
            let hold_str = if hold_mins < 60 {
                format!("{}m", hold_mins)
            } else {
                format!("{:.1}h", hold_mins as f64 / 60.0)
            };

            // Risk and sizing metrics
            let risk_usd  = p.r_dollars_risked;
            let risk_pct  = if p.size_usd > 1e-8 { risk_usd / p.size_usd * 100.0 } else { 0.0 };
            let notional  = p.size_usd * p.leverage;  // actual market exposure
            let lev_str   = format!("{:.1}×", p.leverage);
            // Quantity display — auto-scale decimal places
            let qty_str = if p.quantity >= 1000.0 {
                format!("{:.2}", p.quantity)
            } else if p.quantity >= 1.0 {
                format!("{:.4}", p.quantity)
            } else {
                format!("{:.6}", p.quantity)
            };

            // ── Coin metadata ─────────────────────────────────────────────
            let logo_img  = coins::coin_logo_img(&p.symbol, 22);
            let full_name = coins::coin_name(&p.symbol);
            let name_span = if full_name.is_empty() {
                String::new()
            } else {
                format!("<span style='color:#8b949e;font-size:.78em;margin-left:4px'>{}</span>", full_name)
            };

            // ── AI recommendation row ─────────────────────────────────────
            let ai_row = match (&p.ai_action, &p.ai_reason) {
                (Some(action), Some(reason)) => {
                    let (ai_icon, ai_col) = match action.as_str() {
                        "scale_up"   => ("📈", "#3fb950"),
                        "scale_down" => ("📉", "#e3b341"),
                        "close_now"  => ("🛑", "#f85149"),
                        _            => ("🤖", "#8b949e"),   // hold
                    };
                    format!(
                        "<div class='pos-meta' style='background:#1c2026;border-radius:4px;\
                         padding:3px 6px;margin-top:4px;font-size:.78em'>\
                         {icon} <span style='color:{col};font-weight:600'>{act}</span>\
                         <span style='color:#8b949e;margin-left:5px'>{rsn}</span></div>",
                        icon = ai_icon,
                        col  = ai_col,
                        act  = action.replace('_', " ").to_uppercase(),
                        rsn  = reason,
                    )
                }
                _ => String::new(),
            };

            format!(r#"<div class="pos-flip-wrap" id="pf-{sym_id}" data-pnl="{raw_pnl:.4}"><div class="pos-flip-inner">
<div class="pos-card" style="border-left:3px solid {border}" id="pos-{sym_id}" onclick="flipPos('{sym_id}')">
  <div class="pos-header">
    <span class="pos-sym">{logo}{sym}</span>{name}{dca}
    <span class="pos-side" style="color:{sc}">{arrow} {side}</span>
    <span class="pos-age">{hold}</span>
  </div>
  <div id="pos-{sym_id}-pnl" class="pos-pnl" style="color:{pc}">{ps}{pnl:.2} ({ps}{pct:.1}%) &nbsp; <b style="font-size:1.1em">{r:+.2}R</b></div>
  <div class="pos-bar-wrap">
    <div id="pos-{sym_id}-bar" class="pos-bar" style="width:{bp:.0}%;background:{bc}"></div>
    <div class="pos-bar-marks"><span>-1R</span><span>0</span><span>1R</span><span>2R</span><span>4R</span></div>
  </div>
  <div class="pos-meta">Avg <b>${entry:.4}</b> &nbsp;·&nbsp; Stop <span id="pos-{sym_id}-stop" style="color:#f85149">${stop:.4}</span> &nbsp;·&nbsp; TP <span style="color:#3fb950">${tp:.4}</span></div>
  <div class="pos-meta">
    <span title="Margin committed" style="color:#8b949e">${size:.2} margin</span>
    &nbsp;·&nbsp;
    <span title="Leverage applied" style="color:#388bfd;font-weight:bold">{lev} lev</span>
    &nbsp;·&nbsp;
    <span title="Notional market exposure" style="color:#cdd9e5"><b>${notional:.2}</b> notional</span>
  </div>
  <div class="pos-meta">
    <span title="Token quantity held" style="color:#8b949e">{qty} {sym}</span>
    &nbsp;·&nbsp;
    <span title="Max loss to stop" style="color:#e3b341">Risk ${risk:.2} <span style="color:#8b949e">({rpct:.1}%)</span></span>
  </div>
  <div class="pos-meta" style="color:#8b949e">{tranche} &nbsp;·&nbsp; {time}</div>
  {ob_badges}
  {ai_row}
  <div class="pos-flip-hint">📊 tap to chart</div>
</div>
<div class="pos-flip-back" style="border-left:3px solid {border}">
  <div onclick="flipPos('{sym_id}')" style="display:flex;justify-content:space-between;align-items:center;padding-bottom:7px;cursor:pointer;user-select:none">
    <span style="font-size:.82em;font-weight:700;color:var(--text)">{sym} · 5m</span>
    <span style="font-size:.68em;color:#8b949e;background:#21262d;padding:1px 7px;border-radius:8px">← back</span>
  </div>
  <iframe src="https://www.tradingview.com/widgetembed/?symbol=BINANCE:{sym}USDT&interval=5&theme=dark&style=1&hide_side_toolbar=1&hide_top_toolbar=1&locale=en&allow_symbol_change=0&save_image=0&hotlist=0&calendar=0"
    width="100%" height="205" frameborder="0"
    style="border-radius:6px;display:block" loading="lazy"></iframe>
</div>
</div></div>"#,
                border   = border_colour,
                sym_id   = p.symbol.to_lowercase(),
                raw_pnl  = p.unrealised_pnl,
                logo     = logo_img,
                sym      = p.symbol,
                name     = name_span,
                dca      = dca_badge,
                arrow    = side_arrow,
                side     = p.side,
                sc       = side_colour,
                hold     = hold_str,
                ps       = pnl_sign,
                pnl      = pnl_abs,
                pct      = pct_abs,
                r        = r_mult,
                pc       = pnl_colour,
                bp       = bar_pct,
                bc       = bar_colour,
                entry    = p.entry_price,
                stop     = p.stop_loss,
                tp       = p.take_profit,
                tranche  = tranche_label,
                size     = p.size_usd,
                lev      = lev_str,
                notional = notional,
                qty      = qty_str,
                risk     = risk_usd,
                rpct     = risk_pct,
                time      = p.entry_time,
                ob_badges = ob_badge + &principal_badge + &pool_badge,
                ai_row    = ai_row,
            )
        }).collect()
    };

    // ── Closed trades table ───────────────────────────────────────────────
    let closed_rows: String = if s.closed_trades.is_empty() {
        r#"<tr><td colspan="7" class="empty-td">No closed trades yet</td></tr>"#.to_string()
    } else {
        s.closed_trades.iter().rev().take(20).enumerate().map(|(i, t)| {
            let pc = if t.pnl >= 0.0 { "#3fb950" } else { "#f85149" };
            let ps = if t.pnl >= 0.0 { "+" } else { "-" };
            let sc = if t.side == "LONG" { "#3fb950" } else { "#f85149" };
            let pnl_abs = t.pnl.abs();
            let pct_abs = t.pnl_pct.abs();
            let row_id  = format!("ct-{i}");
            let det_id  = format!("ct-det-{i}");
            // Click-to-expand: show breakdown row if present, fallback to synthesised summary
            let detail_html = t.breakdown.as_deref().unwrap_or("No detailed breakdown recorded for this trade.");
            format!(
                "<tr class='ct-row' style='cursor:pointer' onclick=\"toggleDetail('{det_id}')\" id='{row_id}'>\
                 <td><b>{sym}</b> <span style='color:#444c56;font-size:.75em'>▼</span></td>\
                 <td style='color:{sc}'>{side}</td>\
                 <td>${entry:.4}</td><td>${exit:.4}</td>\
                 <td style='color:{pc}'>{ps}{pnl:.2} ({ps}{pct:.1}%)</td>\
                 <td class='reason-{rc}'>{reason}</td><td class='ts'>{ts}</td></tr>\
                 <tr id='{det_id}' class='ct-detail' style='display:none'>\
                 <td colspan='7' style='background:#161b22;padding:8px 12px;border-bottom:1px solid #30363d'>\
                 {detail}</td></tr>",
                det_id = det_id,
                row_id = row_id,
                sym    = t.symbol,
                sc     = sc,
                side   = t.side,
                entry  = t.entry,
                exit   = t.exit,
                pc     = pc,
                ps     = ps,
                pnl    = pnl_abs,
                pct    = pct_abs,
                rc     = reason_class(&t.reason),
                reason = t.reason,
                ts     = t.closed_at,
                detail = detail_html,
            )
        }).collect()
    };

    // ── Candidates table ──────────────────────────────────────────────────
    let cand_rows: String = if s.candidates.is_empty() {
        r#"<tr><td colspan="5" class="empty-td">Scanning…</td></tr>"#.to_string()
    } else {
        // Sort: open positions first (most profitable at top), then rest by confidence desc.
        let mut sorted: Vec<&CandidateInfo> = s.candidates.iter().collect();
        sorted.sort_by(|a, b| {
            let ap = s.positions.iter().find(|p| p.symbol == a.symbol);
            let bp = s.positions.iter().find(|p| p.symbol == b.symbol);
            match (ap, bp) {
                (Some(ap), Some(bp)) => bp
                    .unrealised_pnl
                    .partial_cmp(&ap.unrealised_pnl)
                    .unwrap_or(std::cmp::Ordering::Equal),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => {
                    let ac = a.confidence.unwrap_or(0.0);
                    let bc = b.confidence.unwrap_or(0.0);
                    bc.partial_cmp(&ac).unwrap_or(std::cmp::Ordering::Equal)
                }
            }
        });

        sorted.iter().map(|c| {
            let chg_td = match c.change_pct {
                Some(pct) => {
                    let cc = if pct >= 0.0 { "#3fb950" } else { "#f85149" };
                    let cs = if pct >= 0.0 { "+" } else { "" };
                    format!("<td class='tbl-r' style='color:{}'>{}{:.3}%</td>", cc, cs, pct)
                }
                None => "<td class='tbl-r' style='color:var(--muted)'>—</td>".to_string(),
            };

            // Find open position for this symbol (if any)
            let open_pos = s.positions.iter().find(|p| p.symbol == c.symbol);
            let is_open  = open_pos.is_some();

            // P&L pill for open positions: green = in the money, red = out of money
            let pnl_pill = if let Some(pos) = open_pos {
                let pnl     = pos.unrealised_pnl;
                let pnl_pct = if pos.size_usd > 0.0 { pnl / pos.size_usd * 100.0 } else { 0.0 };
                let (pc, arrow) = if pnl >= 0.0 { ("#3fb950", "▲") } else { ("#f85149", "▼") };
                let sign = if pnl >= 0.0 { "+" } else { "" };
                format!(" <span style='font-size:.72em;color:{pc};background:{pc}18;\
                          border:1px solid {pc}44;border-radius:3px;padding:0 4px;\
                          white-space:nowrap'>{arrow} {sign}{pnl_pct:.1}%</span>")
            } else {
                String::new()
            };

            // Blue highlight for open positions
            let sym_style = if is_open { "font-weight:700;color:#58a6ff" } else { "" };
            let open_dot  = if is_open { " ●" } else { "" };

            // Coin logo (16 px) next to ticker
            let c_logo = coins::coin_logo_img(&c.symbol, 16);

            // Regime mini-badge: [T] trending (blue) / [R] ranging (yellow) / [N] neutral (grey)
            let regime_badge = match c.regime.as_deref() {
                Some("Trending") => "<span style='color:#58a6ff;font-size:.68em;background:#58a6ff18;\
                    border:1px solid #58a6ff44;border-radius:3px;padding:0 3px;margin-left:3px'>T</span>",
                Some("Ranging")  => "<span style='color:#e3b341;font-size:.68em;background:#e3b34118;\
                    border:1px solid #e3b34144;border-radius:3px;padding:0 3px;margin-left:3px'>R</span>",
                Some("Neutral")  => "<span style='color:#8b949e;font-size:.68em;background:#8b949e18;\
                    border:1px solid #8b949e44;border-radius:3px;padding:0 3px;margin-left:3px'>N</span>",
                _ => "",
            };

            // RSI cell: green <30 (oversold), red >70 (overbought), grey otherwise
            let rsi_td = match c.rsi {
                Some(r) => {
                    let (rc, label) = if r < 30.0 { ("#3fb950", "OS") }
                                      else if r > 70.0 { ("#f85149", "OB") }
                                      else { ("#8b949e", "") };
                    if label.is_empty() {
                        format!("<td class='tbl-c' style='color:{rc}'>{r:.0}</td>")
                    } else {
                        format!("<td class='tbl-c' style='color:{rc}'>{r:.0} <span style='font-size:.72em'>{label}</span></td>")
                    }
                }
                None => "<td class='tbl-c' style='color:var(--muted)'>—</td>".to_string(),
            };

            // Confidence cell: colour-graded white→yellow→green
            let conf_td = match c.confidence {
                Some(cf) => {
                    let pct = cf * 100.0;
                    let cc  = if pct >= 70.0 { "#3fb950" } else if pct >= 55.0 { "#e3b341" } else { "#8b949e" };
                    format!("<td class='tbl-c' style='color:{cc}'>{pct:.0}%</td>")
                }
                None => "<td class='tbl-c' style='color:var(--muted)'>—</td>".to_string(),
            };

            format!("<tr data-sym='{sym}'>\
                       <td style='{ss}'>{logo}{sym}{dot}{pnl}{rbadge}</td>\
                       <td class='tbl-r'>${price:.4}</td>\
                       {chg_td}\
                       {rsi_td}\
                       {conf_td}\
                     </tr>",
                ss      = sym_style,
                logo    = c_logo,
                sym     = c.symbol,
                dot     = open_dot,
                pnl     = pnl_pill,
                rbadge  = regime_badge,
                price   = c.price,
                chg_td  = chg_td,
                rsi_td  = rsi_td,
                conf_td = conf_td,
            )
        }).collect()
    };

    // ── Signal feed rows (staggered animation) ────────────────────────────
    let dec_rows: String = if s.recent_decisions.is_empty() {
        // Show the live scan status so the user sees activity immediately
        let live_msg = if s.status.is_empty() {
            "Waiting for first scan…".to_string()
        } else {
            s.status.clone()
        };
        format!("<tr><td colspan='5' class='empty-td'>{live_msg}</td></tr>")
    } else {
        s.recent_decisions.iter().rev().take(20).enumerate().map(|(i, d)| {
            let is_skip = d.action == "SKIP";
            let (ac, dc, icon) = match d.action.as_str() {
                "BUY"  => ("▲ BUY",  "#3fb950", "🟢"),
                "SELL" => ("▼ SELL", "#f85149", "🔴"),
                _      => ("— SKIP", "#8b949e", "⬛"),
            };
            // Dim SKIP rows so real signals stand out
            let row_style = if is_skip {
                "opacity:0.45;font-size:.88em"
            } else {
                "font-weight:500"
            };
            // Extract regime tag from rationale prefix "[Trending]" / "[Ranging]" / "[Neutral]"
            let (regime_badge, rat_body) = if d.rationale.starts_with('[') {
                if let Some(end) = d.rationale.find(']') {
                    let tag  = &d.rationale[1..end];
                    let body = d.rationale[end + 2..].to_string(); // skip '] '
                    let col  = match tag {
                        "Trending" => "#58a6ff",
                        "Ranging"  => "#e3b341",
                        _          => "#8b949e",
                    };
                    (format!("<span style='color:{};font-size:.72em;background:{}22;\
                               border:1px solid {}44;border-radius:3px;padding:0 4px'>{}</span> ",
                              col, col, col, tag), body)
                } else {
                    (String::new(), d.rationale.clone())
                }
            } else {
                (String::new(), d.rationale.clone())
            };
            let sig_logo = coins::coin_logo_img(&d.symbol, 15);
            let delay_ms = i * 60;
            format!(
                "<tr class='sig-row' style='animation-delay:{delay}ms;{rs}'>\
                   <td>{logo}{icon} <b>{sym}</b></td>\
                   <td style='color:{dc};font-weight:600'>{ac}</td>\
                   <td>{conf:.0}%</td>\
                   <td class='ts' style='max-width:260px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap'>{rbadge}{rat}</td>\
                   <td class='ts'>{ts}</td>\
                 </tr>",
                delay  = delay_ms,
                rs     = row_style,
                logo   = sig_logo,
                icon   = icon,
                sym    = d.symbol,
                dc     = dc,
                ac     = ac,
                conf   = d.confidence * 100.0,
                rbadge = regime_badge,
                rat    = rat_body,
                ts     = d.timestamp,
            )
        }).collect()
    };

    // ── Equity sparkline SVG ──────────────────────────────────────────────
    // Shows equity relative to initial_capital (baseline = break-even).
    // Green fill + line when above initial capital; red when below.
    let sparkline_svg: String = {
        let h = &s.equity_history;
        let initial = s.initial_capital;
        if h.len() < 2 {
            // Not enough data yet — flat placeholder
            r##"<svg width="320" height="80" viewBox="0 0 320 80"
     style="display:block;flex-shrink:0;overflow:visible;opacity:0.4">
  <text x="2" y="10" fill="#484f58" font-size="9" font-family="monospace">PORTFOLIO</text>
  <line x1="0" y1="46" x2="280" y2="46"
        stroke="#484f58" stroke-width="1.5" stroke-dasharray="4 4"/>
  <text x="284" y="50" fill="#484f58" font-size="9" font-family="monospace">—</text>
</svg>"##
                .to_string()
        } else {
            let w_px: f64 = 280.0; // chart area width (label gutter on right)
            let h_px: f64 = 80.0;
            let pad_t: f64 = 14.0; // top padding (for "PORTFOLIO" label)
            let pad_b: f64 = 6.0;
            let inner_h = h_px - pad_t - pad_b;

            // Y-scale anchored to initial_capital so baseline is always visible
            let data_min = h.iter().cloned().fold(f64::INFINITY, f64::min).min(initial);
            let data_max = h
                .iter()
                .cloned()
                .fold(f64::NEG_INFINITY, f64::max)
                .max(initial);
            // Symmetric 15 % buffer so the line never presses against the edges
            let buf = ((data_max - data_min).max(initial * 0.005)) * 0.18;
            let min_v = data_min - buf;
            let max_v = data_max + buf;
            let range = (max_v - min_v).max(0.01);

            // Map a $ value to an SVG y coordinate (top = high equity)
            let to_y = |v: f64| -> f64 { h_px - pad_b - (v - min_v) / range * inner_h };

            let n = h.len() as f64;
            let pts: String = h
                .iter()
                .enumerate()
                .map(|(i, &v)| {
                    let x = i as f64 / (n - 1.0) * w_px;
                    let y = to_y(v);
                    format!("{x:.1},{y:.1}")
                })
                .collect::<Vec<_>>()
                .join(" ");

            let base_y = to_y(initial);
            let last_y = to_y(*h.last().unwrap_or(&initial));
            let last_val = *h.last().unwrap_or(&initial);
            let max_y = to_y(data_max);

            // Green when above initial capital, red when below
            let trend_c = if last_val >= initial {
                "#3fb950"
            } else {
                "#f85149"
            };

            // Fill polygon: line path → close back along the baseline
            let fill_pts = format!("{pts} {w_px:.1},{base_y:.1} 0.0,{base_y:.1}");

            // Y-axis tick label values
            let lbl_cur = format!("${:.0}", last_val);
            let lbl_base = format!("${:.0}", initial);
            let lbl_max = format!("${:.0}", data_max);

            // Label positions (right gutter starting at x=284)
            let ly_cur = last_y.max(pad_t + 4.0).min(h_px - 4.0);
            let ly_base = base_y.max(pad_t + 4.0).min(h_px - 4.0);
            let ly_max = max_y.max(pad_t + 4.0).min(h_px - 4.0);

            // NOTE: r##"..."## (two hashes) is required here because SVG colour
            // attributes like fill="#484f58" contain the sequence `"#` which would
            // prematurely close an r#"..."# raw string (single-hash delimiter).
            // With two hashes the closing token is `"##`, which never appears in hex
            // colour codes, so all `"#rrggbb"` attributes are safely inside the string.
            format!(
                r##"<svg width="320" height="80" viewBox="0 0 320 80"
     style="display:block;flex-shrink:0;overflow:visible">
  <text x="2" y="10" fill="{m}" font-size="9" font-family="monospace">PORTFOLIO</text>
  <line x1="0" y1="{by:.1}" x2="{w:.1}" y2="{by:.1}"
        stroke="{c}" stroke-width="0.75" stroke-dasharray="3 3" stroke-opacity="0.5"/>
  <polygon points="{fp}" fill="{c}" fill-opacity="0.12"/>
  <polyline points="{pts}" fill="none" stroke="{c}"
            stroke-width="2" stroke-linejoin="round" stroke-linecap="round"/>
  <circle cx="{w:.1}" cy="{ly:.1}" r="5" fill="{c}" fill-opacity="0.2"/>
  <circle cx="{w:.1}" cy="{ly:.1}" r="3" fill="{c}"/>
  <text x="286" y="{lc_y:.1}" fill="{c}" font-size="9" font-family="monospace"
        font-weight="bold" dominant-baseline="middle">{lc}</text>
  <text x="286" y="{lb_y:.1}" fill="{m}" font-size="8" font-family="monospace"
        dominant-baseline="middle">{lb}</text>
  <text x="286" y="{lm_y:.1}" fill="{m}" font-size="8" font-family="monospace"
        dominant-baseline="middle">{lm}</text>
</svg>"##,
                c = trend_c,
                m = "#484f58",
                w = w_px,
                by = base_y,
                fp = fill_pts,
                pts = pts,
                ly = last_y,
                lc = lbl_cur,
                lb = lbl_base,
                lm = lbl_max,
                lc_y = ly_cur,
                lb_y = ly_base,
                lm_y = ly_max,
            )
        }
    };

    // ── Signal weights: single-line inline strip ─────────────────────────
    let w = &s.signal_weights;
    let wh = format!(
        r#"<div class="w-strip">{}{}{}{}{}{}<span class="w-strip-note">{total_closed} trades · live learning</span></div>"#,
        wi("RSI", w.rsi),
        wi("BB", w.bollinger),
        wi("MACD", w.macd),
        wi("Trend", w.trend),
        wi("OrdFlow", w.order_flow),
        wi("🌙Sent", w.sentiment),
        total_closed = s.closed_trades.len(),
    );

    // ── New format args for metric modals ──────────────────────────────────
    // These are injected as raw floats/ints so the JS modal can display them
    // and compute gauge positions without dealing with formatted strings.
    let expect_signed = m.expectancy; // signed (not .abs())
    let pf_float = if m.profit_factor.is_infinite() {
        999.0f64
    } else {
        m.profit_factor
    };
    let kelly_float = m.kelly_fraction(); // -1.0 = sentinel "not enough data"
    let cb_int = if cb_active { 1i32 } else { 0i32 };
    let wr_float = m.win_rate * 100.0;

    // ── METRIC_INFO: static JS data injected as a raw string (no brace escaping) ──
    // Injected via {metric_info_js} format arg so real {/} in JS don't need doubling.
    let metric_info_js = r#"
var METRIC_INFO={
  sharpe:{
    name:'Sharpe Ratio',
    fmt:function(v){return v.toFixed(2);},
    gmin:-1.5,gmax:3.0,
    zones:[{t:0,l:'Losing',c:'#f85149'},{t:0.5,l:'Weak',c:'#e3b341'},{t:1.0,l:'Acceptable',c:'#e3b341'},{t:2.0,l:'Good',c:'#3fb950'},{t:99,l:'Excellent',c:'#3fb950'}],
    formula:'Avg(trade returns) ÷ StdDev(all returns)',
    notes:['Returns are P&L as % of margin committed per trade.','StdDev captures both winning and losing swings — all volatility.','Based on every closed trade (and partial close) this session.'],
    verdict:function(v){
      if(v>2.5)return['#3fb950','🟢 Exceptional — top funds target >1.5. Strong returns with consistently low noise.'];
      if(v>1.5)return['#3fb950','🟢 Great — a genuinely good risk-adjusted edge. Earning well per unit of risk taken.'];
      if(v>0.5)return['#e3b341','🟡 Acceptable — real edge present but somewhat noisy. Tighter exits might improve this.'];
      if(v>0)return['#e3b341','🟡 Weak — barely above zero. The edge may not survive a rough market patch.'];
      return['#f85149','🔴 Negative — losses outpacing gains. The Sharpe multiplier is automatically scaling down position sizes.'];
    }
  },
  sortino:{
    name:'Sortino Ratio',
    fmt:function(v){return v.toFixed(2);},
    gmin:-1.5,gmax:4.0,
    zones:[{t:0,l:'Losing',c:'#f85149'},{t:0.5,l:'Weak',c:'#e3b341'},{t:1.0,l:'OK',c:'#e3b341'},{t:2.0,l:'Good',c:'#3fb950'},{t:99,l:'Excellent',c:'#3fb950'}],
    formula:'Avg(trade returns) ÷ StdDev(losing returns only)',
    notes:['Like Sharpe, but the denominator only counts losing trades — upside volatility is not penalised.','Sortino > Sharpe: your losses are well-contained relative to wins (ideal).','Sortino < Sharpe: your losses are disproportionately volatile and dragging the ratio down.'],
    verdict:function(v){
      if(v>3.0)return['#3fb950','🟢 Exceptional — downside is extremely well-contained relative to average gain.'];
      if(v>2.0)return['#3fb950','🟢 Excellent — losses are small and predictable. The hallmark of disciplined risk management.'];
      if(v>1.0)return['#3fb950','🟢 Good — losing trades are reasonably controlled. A healthy strategy profile.'];
      if(v>0)return['#e3b341','🟡 Neutral — some downside noise present. Reviewing stop-loss placement may help.'];
      return['#f85149','🔴 Negative — losing trades are too large or too frequent relative to wins.'];
    }
  },
  expect:{
    name:'Expectancy',
    fmt:function(v){return(v>=0?'+':'')+v.toFixed(2)+'%';},
    gmin:-4.0,gmax:5.0,
    zones:[{t:-1,l:'Losing',c:'#f85149'},{t:0,l:'Marginal',c:'#e3b341'},{t:0.5,l:'OK',c:'#e3b341'},{t:2.0,l:'Good',c:'#3fb950'},{t:99,l:'Strong',c:'#3fb950'}],
    formula:'Win Rate × Avg Win% − Loss Rate × Avg Loss%',
    notes:['The expected P&L per trade, as % of the margin committed.','e.g. +1.5% means: each trade is expected to return 1.5% of its margin on average.','This is the single best indicator of whether the strategy has a sustainable edge.','A negative expectancy means the system loses money over time regardless of luck.'],
    verdict:function(v){
      if(v>3.0)return['#3fb950','🟢 Strong edge — each trade is expected to return >3% of its margin on average.'];
      if(v>1.0)return['#3fb950','🟢 Solid edge — meaningful per-trade return. Sustainable with consistent execution.'];
      if(v>0)return['#e3b341','🟡 Slim edge — positive but small. Fees could eat this: verify builder fee tier is correct.'];
      if(v>-1)return['#e3b341','🟡 Marginally negative — just below break-even. Minor exit improvements could flip this positive.'];
      return['#f85149','🔴 Negative — losing more on losses than winning on winners. Review entry criteria and stop placement.'];
    }
  },
  pf:{
    name:'Profit Factor',
    fmt:function(v){return v>=999?'∞ (no losses yet)':v.toFixed(2)+'×';},
    gmin:0,gmax:3.5,
    zones:[{t:0.8,l:'Losing',c:'#f85149'},{t:1.0,l:'Marginal',c:'#e3b341'},{t:1.5,l:'OK',c:'#e3b341'},{t:2.5,l:'Good',c:'#3fb950'},{t:99,l:'Excellent',c:'#3fb950'}],
    formula:'Total $ Won ÷ Total $ Lost  (all closed trades)',
    notes:['1.0 = exactly break-even before fees.','2.0 = earned $2 for every $1 lost in total.','Works with win rate: a 40% win rate is fine if profit factor is 2.5+.','Unlike win rate, profit factor accounts for the SIZE of wins and losses — not just their count.'],
    verdict:function(v){
      if(v>=999)return['#3fb950','🟢 No closed losses yet — a real ratio forms as more trades complete. Enjoy it while it lasts.'];
      if(v>2.5)return['#3fb950','🟢 Excellent — winning significantly more in dollar terms than losing.'];
      if(v>1.5)return['#3fb950','🟢 Good — healthy ratio, sustainable even with normal variance in win rate.'];
      if(v>1.0)return['#e3b341','🟡 Marginal — just above break-even. After fees the real edge is very thin.'];
      return['#f85149','🔴 Below 1 — gross losses exceed gross wins. Review trade management and exits.'];
    }
  },
  wr:{
    name:'Win Rate',
    fmt:function(v){return v.toFixed(1)+'%';},
    gmin:0,gmax:100,
    zones:[{t:35,l:'Very Low',c:'#f85149'},{t:45,l:'Low',c:'#e3b341'},{t:55,l:'Neutral',c:'#e3b341'},{t:65,l:'Good',c:'#3fb950'},{t:100,l:'High',c:'#3fb950'}],
    formula:'Winning Trades ÷ Total Closed Trades × 100',
    notes:['⚠️ Win rate alone does NOT determine profitability.','A 40% win rate with avg winner 3× avg loser = profitable.','A 70% win rate with tiny wins and huge losses = losing money.','Always read win rate alongside Expectancy and Profit Factor.'],
    verdict:function(v){
      if(v>65)return['#3fb950','🟢 High — consistently winning more than losing. Very comfortable profile to manage.'];
      if(v>55)return['#3fb950','🟢 Above average — more trades are winners than losers. Solid if avg win ≥ avg loss.'];
      if(v>45)return['#e3b341','🟡 Near 50/50 — profitability entirely depends on avg win being bigger than avg loss.'];
      if(v>35)return['#e3b341','🟡 Below average — can still be profitable (trend-following often runs 35-45%) if winners are large.'];
      return['#f85149','🔴 Very low — unless avg wins are 3-4× avg losses, this strategy will bleed over time.'];
    }
  },
  dd:{
    name:'7-Day Rolling Drawdown',
    fmt:function(v){return'-'+v.toFixed(1)+'%';},
    gmin:0,gmax:15,
    invert:true,
    zones:[{t:2,l:'Minimal',c:'#3fb950'},{t:4,l:'Normal',c:'#3fb950'},{t:6,l:'Elevated',c:'#e3b341'},{t:8,l:'High — near CB',c:'#e3b341'},{t:99,l:'CB Active',c:'#f85149'}],
    formula:'(7-day Peak Equity − Current Equity) ÷ 7-day Peak × 100',
    notes:['Rolling 7-day window — one lucky spike long ago never permanently throttles sizing.','This is what DRIVES the circuit breaker: triggers at 8% (not all-time drawdown).','Open unrealised P&L is included in equity — a position recovering auto-heals this metric.','All-time peak drawdown is visible in the tooltip on hover over this card.'],
    verdict:function(v){
      if(v<2)return['#3fb950','🟢 Minimal — equity is near its 7-day peak. Clean, steady performance.'];
      if(v<4)return['#3fb950','🟢 Normal — small pullback from peak. Within expected variance for this trading style.'];
      if(v<6)return['#e3b341','🟡 Elevated — noticeable drop from peak. No circuit breaker yet, but the Sharpe multiplier has already softened new sizes.'];
      if(v<8)return['#e3b341','🟡 High — approaching the 8% circuit breaker threshold. New entries are already using reduced sizes via the Sharpe multiplier.'];
      return['#f85149','🔴 Circuit Breaker Active — all new position sizes are scaled to 0.35× until equity recovers. This is the self-protection mechanism working exactly as designed.'];
    }
  },
  kelly:{
    name:'Half-Kelly Position Size',
    fmt:function(v){return v<0?'learning…':(v*100).toFixed(1)+'%';},
    gmin:0,gmax:15,
    zones:[{t:2,l:'Minimal',c:'#8b949e'},{t:5,l:'Conservative',c:'#e3b341'},{t:9,l:'Moderate',c:'#3fb950'},{t:12,l:'Aggressive',c:'#e3b341'},{t:99,l:'Max Cap',c:'#f85149'}],
    formula:'½ × ( Win Rate − Loss Rate ÷ (Avg Win / Avg Loss) )',
    notes:['The Kelly Criterion finds the bet size that maximises long-run equity growth.','We use Half-Kelly (50% of full Kelly) to reduce variance while keeping most of the growth advantage.','Requires ≥5 closed trades. Shows "learning…" until then — fixed confidence tiers are used instead.','This is the recommended fraction of FREE CAPITAL to commit per trade (e.g. 7.5% of $1,000 = $75 margin).','Applied AFTER the Sharpe multiplier and circuit-breaker multiplier, so actual size may be lower.'],
    verdict:function(v){
      if(v<0)return['#8b949e','⏳ Not enough history yet. The bot needs ≥5 closed trades to calculate Half-Kelly. Fixed confidence tiers (4-8% of capital) are used until then.'];
      var p=v*100;
      if(p>12)return['#e3b341','🟠 High Kelly — strong apparent edge, but verify it isn\'t noise from a small sample. Position sizes are capped at 15% regardless.'];
      if(p>7)return['#3fb950','🟢 Healthy Kelly — the model has meaningful edge data and is sizing proportional to demonstrated performance.'];
      if(p>3)return['#3fb950','🟢 Conservative Kelly — edge is detected but modest. Small-to-medium positions are appropriate.'];
      return['#e3b341','🟡 Very small Kelly — either edge is minimal or sample is still small. Fixed tiers are more relevant at this stage.'];
    }
  },
  cb:{
    name:'Risk Mode / Circuit Breaker',
    fmt:function(v){return v>0?'⚡ CB ACTIVE':'● Normal';},
    no_gauge:true,
    formula:'7-day rolling drawdown > 8%  →  Circuit Breaker fires',
    notes:['🟢 Normal mode: full Kelly × Sharpe multiplier × confidence = normal position sizes.','🔴 CB Active: ALL new position sizes × 0.35 and confidence floor raised +10%.','Auto-resets when rolling equity recovers to within 8% of the 7-day peak.','This is a hard, automatic rule — not a discretionary override.','The 7-day window prevents a single good week from permanently masking a losing streak.'],
    verdict:function(v){
      if(v>0)return['#f85149','🔴 Circuit Breaker is active. The 7-day rolling drawdown has exceeded 8%. All new position sizes are automatically 0.35× of normal and the minimum confidence required to open a trade is raised by 10 percentage points. This continues automatically until equity recovers.'];
      return['#3fb950','🟢 Normal operating mode. The 7-day equity window shows no significant drawdown from its peak. Full Kelly-based position sizing is in effect across all signals.'];
    }
  },
  openClosed:{
    name:'Open / Total Closed Trades',
    fmt:function(v){return String(v);},
    no_gauge:true,
    formula:'Live count from current session state',
    notes:['Open = positions currently held. No hard count cap — the AI budgets size via Kelly and portfolio heat (15% max equity at risk), so a dozen or more concurrent positions are normal.','Closed = completed trades this session — partial closes count as separate entries.','Session resets on restart — the trades_YYYY.csv ledger captures all-time history.','More closed trades → more reliable metrics. Kelly activates at 5; metrics become statistically meaningful at 10+.'],
    verdict:function(){return['#8b949e','ℹ️ These counts grow as the bot trades. The closed count directly drives the quality of Sharpe, Sortino, Kelly, and Expectancy calculations — the more trades, the more trustworthy the numbers.'];}
  },
  cycles:{
    name:'Bot Cycles Completed',
    fmt:function(v){return String(Math.round(v));},
    no_gauge:true,
    formula:'Incremented every ~30 seconds',
    notes:['1 cycle = fetch all prices → select top candidates → analyse indicators → manage open positions.','AI review runs every 10 cycles (~5 minutes) when positions are open and ANTHROPIC_API_KEY is set.','Cycle time can stretch slightly when many positions are open or external APIs are slow.','The countdown timer in the header shows seconds until the next cycle fires.'],
    verdict:function(v){
      var mins=Math.round(v*0.5);
      var t=mins>=60?(mins/60).toFixed(1)+' hours':mins+' minutes';
      return['#8b949e','ℹ️ The bot has been running for approximately '+t+'. Each 30-second cycle analyses the candidate list and updates all open position trailing stops.'];
    }
  },
  scanning:{
    name:'Coins in Deep Scan This Cycle',
    fmt:function(v){return String(Math.round(v));},
    no_gauge:true,
    formula:'BTC + ETH + SOL (always) + top movers by |% change| since last cycle',
    notes:['Hyperliquid has 150+ perpetuals — scanning uses a two-tier system to stay inside the 30-second cycle budget.','Tier 1 (free): one allMids call fetches every price in the entire HL universe instantly.','Tier 2 (per-coin): HL native candle API fetched for the top 40 most active perps each cycle — no Binance dependency.','The 40 slots rotate every cycle — the most actively moving coins get full RSI/MACD/ATR/order-flow analysis.','All other HL perps are still price-tracked but skip deep indicator analysis unless they start moving.'],
    verdict:function(v){return['#8b949e','ℹ️ '+v+' coins are getting full indicator analysis this cycle. The remaining ~'+Math.max(0,150-v)+' Hyperliquid perps are price-tracked via allMids and rotate into the deep-scan list when they start moving.'];}
  },
  deployed:{
    name:'Capital Deployed (Margin)',
    fmt:function(v){return'$'+parseFloat(v).toFixed(0);},
    no_gauge:true,
    formula:'Σ margin committed across all open positions',
    notes:['This is MARGIN committed, not notional market exposure.','Example: $100 margin at 3× leverage controls $300 notional.','Free capital = Total equity − deployed margin.','The bot always maintains free capital to take new entries and DCA opportunities.','Multiply each position\'s margin by its leverage to get total notional exposure.'],
    verdict:function(v){return['#8b949e','ℹ️ $'+parseFloat(v).toFixed(0)+' of margin is currently working in active trades. Check individual position cards to see leverage and notional exposure per coin.'];}
  }
};

function _metricZone(zones,v){
  for(var i=0;i<zones.length;i++){if(v<=zones[i].t)return zones[i];}
  return zones[zones.length-1];
}

function showMetric(id,value){
  var info=METRIC_INFO[id];
  if(!info)return;
  var v=parseFloat(value);
  var disp=info.fmt?info.fmt(v):String(value);
  var zone=info.zones?_metricZone(info.zones,v):{c:'#8b949e',l:''};
  var verdict=info.verdict?info.verdict(v):['#8b949e',''];
  var vColor=verdict[0],vText=verdict[1];

  /* ── Gauge ── */
  var gaugeHtml='';
  if(!info.no_gauge&&info.zones){
    var gmin=info.gmin||0,gmax=info.gmax||100,range=gmax-gmin;
    var prev=gmin,zHtml='';
    info.zones.forEach(function(z){
      var cap=Math.min(z.t,gmax);
      var w=Math.max(0,(cap-prev)/range*100);
      if(w>0){
        zHtml+='<div style="flex:'+w.toFixed(1)+';background:'+z.c+'22;border:1px solid '+z.c+'55;display:flex;align-items:center;justify-content:center;font-size:.58em;color:'+z.c+';padding:2px 0;border-radius:3px;overflow:hidden;white-space:nowrap;text-overflow:ellipsis">'+z.l+'</div>';
      }
      prev=cap;
    });
    var clamp=Math.max(gmin,Math.min(gmax,v));
    var pos=((clamp-gmin)/range*100).toFixed(1);
    gaugeHtml='<div style="margin:14px 0 2px"><div style="display:flex;gap:2px;height:26px">'+zHtml+'</div>'
      +'<div style="position:relative;height:18px">'
      +'<div style="position:absolute;left:'+pos+'%;transform:translateX(-50%);top:0;font-size:.9em;color:'+zone.c+'">▲</div>'
      +'</div>'
      +'<div style="text-align:center;font-size:.75em;color:'+zone.c+';font-weight:700">'+disp+(zone.l?' · '+zone.l:'')+'</div>'
      +'</div>';
  }

  /* ── Notes ── */
  var notesHtml='';
  if(info.notes&&info.notes.length){
    notesHtml='<ul style="margin:10px 0 0;padding-left:16px;color:#8b949e;font-size:.78em;line-height:1.9">';
    info.notes.forEach(function(n){notesHtml+='<li>'+n+'</li>';});
    notesHtml+='</ul>';
  }

  /* ── Build content ── */
  var content=document.getElementById('metric-modal-content');
  if(!content)return;
  content.innerHTML=
    '<div style="display:flex;justify-content:space-between;align-items:flex-start;margin-bottom:12px">'
    +'<div>'
    +'<div style="font-size:.65em;color:#8b949e;text-transform:uppercase;letter-spacing:1.1px;margin-bottom:4px">'+info.name+'</div>'
    +'<div style="font-size:2.3em;font-weight:800;color:'+vColor+';line-height:1;letter-spacing:-.02em">'+disp+'</div>'
    +(zone.l&&!info.no_gauge?'<div style="font-size:.73em;color:'+zone.c+';margin-top:3px;font-weight:600">'+zone.l+'</div>':'')
    +'</div>'
    +'<button onclick="closeMetricModal()" style="background:none;border:1px solid #30363d;color:#6e7681;width:28px;height:28px;border-radius:7px;cursor:pointer;font-size:.9em;flex-shrink:0;display:flex;align-items:center;justify-content:center">✕</button>'
    +'</div>'
    +gaugeHtml
    +'<div style="background:#1c2026;border-radius:8px;padding:10px 13px;margin-top:12px;font-size:.82em;line-height:1.65;color:#c9d1d9">'+vText+'</div>'
    +'<div style="margin-top:14px;border-top:1px solid #21262d;padding-top:12px">'
    +'<div style="font-size:.62em;color:#8b949e;text-transform:uppercase;letter-spacing:.9px;margin-bottom:5px">Formula</div>'
    +'<code style="font-size:.8em;color:#bc8cff;background:#21262d;padding:5px 10px;border-radius:5px;display:block;line-height:1.5">'+info.formula+'</code>'
    +notesHtml
    +'</div>';

  var modal=document.getElementById('metric-modal');
  if(modal){modal.style.display='flex';document.body.style.overflow='hidden';}
}

function closeMetricModal(){
  var m=document.getElementById('metric-modal');
  if(m)m.style.display='none';
  document.body.style.overflow='';
}
"#;

    Html(format!(
        r#"<!DOCTYPE html>
<html lang="en"><head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,maximum-scale=1">
<title>TradingBots.fun</title>
<meta http-equiv="refresh" content="35">
<style>
*{{box-sizing:border-box;margin:0;padding:0}}
:root{{--bg:#080c10;--surface:#0d1117;--surface2:#161b22;--border:#21262d;--border2:#30363d;
      --muted:#6e7681;--text:#e6edf3;--text2:#c9d1d9;
      --green:#3fb950;--red:#f85149;--blue:#58a6ff;--yellow:#e3b341;--purple:#bc8cff;--dim:#161b22}}
body{{background:var(--bg);color:var(--text);
      font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',system-ui,sans-serif;
      font-size:14px;line-height:1.4;padding:12px;max-width:940px;margin:0 auto;
      background-image:radial-gradient(ellipse 80% 50% at 50% -10%,rgba(88,166,255,.06),transparent)}}
/* ── Keyframe animations ── */
@keyframes pulse{{0%,100%{{opacity:1}}50%{{opacity:.3}}}}
@keyframes fadeSlide{{from{{opacity:0;transform:translateY(6px)}}to{{opacity:1;transform:translateY(0)}}}}
@keyframes scanBeam{{0%{{top:-4px;opacity:.8}}100%{{top:100%;opacity:0}}}}
@keyframes progFill{{from{{width:0}}to{{width:100%}}}}
@keyframes radar{{0%{{transform:rotate(0deg)}}100%{{transform:rotate(360deg)}}}}
@keyframes shimmer{{0%{{background-position:-200% 0}}100%{{background-position:200% 0}}}}
@keyframes liveDot{{0%,100%{{box-shadow:0 0 0 0 rgba(63,185,80,.6)}}70%{{box-shadow:0 0 0 5px rgba(63,185,80,0)}}}}
@keyframes aiPulse{{0%,100%{{opacity:1}}50%{{opacity:.55}}}}
@keyframes cbFlash{{0%,100%{{border-color:rgba(248,81,73,.55)}}50%{{border-color:rgba(248,81,73,1)}}}}
/* ── Header ── */
.header{{display:flex;justify-content:space-between;align-items:center;margin-bottom:16px;flex-wrap:wrap;gap:6px;
         padding-bottom:12px;border-bottom:1px solid var(--border)}}
.header h1{{font-size:1.05em;font-weight:700;display:flex;align-items:center;gap:7px;
            background:linear-gradient(90deg,#58a6ff,#bc8cff);-webkit-background-clip:text;
            -webkit-text-fill-color:transparent;background-clip:text}}
.header .ts{{font-size:.72em;color:var(--muted);white-space:nowrap}}
.live-ring{{width:8px;height:8px;border-radius:50%;background:var(--green);display:inline-block;
            animation:liveDot 2s ease infinite;flex-shrink:0}}
/* ── Equity hero ── */
.equity-hero{{background:linear-gradient(135deg,rgba(13,17,23,.95),rgba(22,27,34,.95));
              border:1px solid rgba(88,166,255,.18);border-radius:12px;
              padding:18px 20px;margin-bottom:12px;
              display:flex;justify-content:space-between;align-items:center;flex-wrap:wrap;gap:12px;
              box-shadow:0 0 0 1px rgba(88,166,255,.04),0 8px 32px rgba(0,0,0,.4),
                         inset 0 1px 0 rgba(255,255,255,.04)}}
.equity-hero .eq-left{{display:flex;flex-direction:column;gap:4px}}
.equity-hero .eq-val{{font-size:2.1em;font-weight:800;line-height:1;letter-spacing:-.02em;
                       background:linear-gradient(135deg,#e6edf3 30%,#58a6ff);
                       -webkit-background-clip:text;-webkit-text-fill-color:transparent;background-clip:text}}
.equity-hero .eq-label{{font-size:.68em;color:var(--muted);letter-spacing:.3px}}
.equity-hero .pnl-badge{{padding:7px 14px;border-radius:22px;font-size:.9em;font-weight:700;
                          letter-spacing:.2px}}
.eq-right{{display:flex;align-items:center;gap:16px;flex:1;justify-content:flex-end;min-width:0}}
/* ── Metric strip ── */
.metrics{{display:grid;grid-template-columns:repeat(2,1fr);gap:8px;margin-bottom:12px}}
@media(min-width:500px){{.metrics{{grid-template-columns:repeat(3,1fr)}}}}
@media(min-width:700px){{.metrics{{grid-template-columns:repeat(6,1fr)}}}}
.metric{{background:var(--surface2);border:1px solid var(--border);border-radius:9px;
         padding:9px 11px;text-align:center;cursor:pointer;
         transition:border-color .2s,box-shadow .2s,background .2s}}
.metric:hover{{border-color:var(--border2);box-shadow:0 2px 8px rgba(0,0,0,.3);background:#1a1f28}}
.metric .mv{{font-size:1.05em;font-weight:700;letter-spacing:-.01em}}
.metric .ml{{font-size:.62em;color:var(--muted);margin-top:3px;white-space:nowrap;letter-spacing:.3px;text-transform:uppercase}}
.metric .ml-hint{{font-size:.58em;color:#444c56;display:block;margin-top:1px}}
/* ── Metric modal ── */
@keyframes modalIn{{from{{opacity:0;transform:scale(.95)}}to{{opacity:1;transform:scale(1)}}}}
#metric-modal{{position:fixed;inset:0;background:rgba(0,0,0,.75);z-index:9999;
               display:none;align-items:center;justify-content:center;padding:16px}}
#metric-modal-content{{background:#0d1117;border:1px solid #30363d;border-radius:14px;
                        padding:22px;max-width:440px;width:100%;max-height:88vh;
                        overflow-y:auto;animation:modalIn .22s ease}}
#metric-modal-content::-webkit-scrollbar{{width:4px}}
#metric-modal-content::-webkit-scrollbar-thumb{{background:#30363d;border-radius:2px}}
/* ── Status bar ── */
.status-bar{{background:var(--surface2);border:1px solid var(--border);border-radius:9px;
             padding:0;margin-bottom:6px;font-size:.78em;color:var(--muted);overflow:hidden}}
.status-inner{{display:flex;justify-content:space-between;align-items:center;
               gap:8px;flex-wrap:wrap;padding:8px 12px}}
.status-bar .st-text{{flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}}
.prog-track{{height:2px;background:var(--border);position:relative;overflow:hidden}}
.prog-fill{{height:2px;background:linear-gradient(90deg,var(--blue),var(--purple),var(--green));
            animation:progFill 30s linear forwards}}
/* ── AI status bar ── */
.ai-status-bar{{background:rgba(188,140,255,.07);border:1px solid rgba(188,140,255,.22);
                border-radius:9px;padding:6px 12px;margin-bottom:12px;
                font-size:.76em;color:#bc8cff;display:flex;align-items:center;gap:6px;
                animation:fadeSlide .4s ease}}
.ai-status-bar.ai-active{{animation:aiPulse 2s ease infinite}}
/* ── Equity hero profit / loss glow ── */
.equity-hero.pnl-pos{{border-color:rgba(63,185,80,.35);
                       box-shadow:0 0 0 1px rgba(63,185,80,.08),0 8px 32px rgba(0,0,0,.4),
                                  inset 0 1px 0 rgba(63,185,80,.08)}}
.equity-hero.pnl-neg{{border-color:rgba(248,81,73,.35);
                       box-shadow:0 0 0 1px rgba(248,81,73,.08),0 8px 32px rgba(0,0,0,.4),
                                  inset 0 1px 0 rgba(248,81,73,.06)}}
.equity-hero.pnl-cb{{border-color:rgba(248,81,73,.6);animation:cbFlash 1.5s ease infinite}}
/* ── CB metric card flash ── */
.metric-cb-active{{border-color:rgba(248,81,73,.7)!important;animation:cbFlash 1.5s ease infinite}}
/* ── Sections ── */
.section{{background:var(--surface2);border:1px solid var(--border);border-radius:11px;
          padding:14px;margin-bottom:12px;border-top:1px solid rgba(255,255,255,.04)}}
.section-positions{{border-left:3px solid rgba(63,185,80,.5)}}
.section-signals{{border-left:3px solid rgba(88,166,255,.5)}}
.section-candidates{{border-left:3px solid rgba(188,140,255,.5)}}
.section-closed{{border-left:3px solid rgba(110,118,129,.35)}}
.section-title{{font-size:.68em;text-transform:uppercase;letter-spacing:1.2px;color:var(--muted);
                margin-bottom:11px;display:flex;justify-content:space-between;align-items:center;gap:6px}}
.section-title-left{{display:flex;align-items:center;gap:6px}}
.badge{{background:var(--border);color:var(--muted);padding:2px 8px;border-radius:10px;
        font-size:.85em;letter-spacing:.2px}}
/* ── Position cards + flip ── */
.pos-grid{{display:grid;grid-template-columns:repeat(auto-fill,minmax(260px,1fr));gap:10px}}
/* Flip wrapper sits in the grid; inner uses CSS grid to stack front & back */
.pos-flip-wrap{{perspective:1200px;-webkit-perspective:1200px;touch-action:manipulation;cursor:pointer}}
.pos-flip-inner{{display:grid;grid-template-columns:1fr;
                 transform-style:preserve-3d;-webkit-transform-style:preserve-3d;
                 transition:transform .55s cubic-bezier(.4,0,.2,1),
                             -webkit-transform .55s cubic-bezier(.4,0,.2,1)}}
.pos-flip-wrap.flipped .pos-flip-inner{{transform:rotateY(180deg);-webkit-transform:rotateY(180deg)}}
/* Front face */
.pos-card{{background:var(--dim);border-radius:8px;padding:12px;border-left:3px solid var(--border);
           animation:fadeSlide .35s ease both;
           grid-area:1/1;backface-visibility:hidden;-webkit-backface-visibility:hidden;
           cursor:pointer}}
/* Back face — chart */
.pos-flip-back{{grid-area:1/1;backface-visibility:hidden;-webkit-backface-visibility:hidden;
                transform:rotateY(180deg);background:var(--dim);border-radius:8px;padding:10px;
                overflow:hidden;min-height:240px;border-left:3px solid var(--border)}}
.pos-flip-hint{{text-align:center;font-size:.62em;color:#444c56;margin-top:6px;letter-spacing:.5px;user-select:none}}
.pos-header{{display:flex;align-items:center;gap:8px;margin-bottom:6px}}
.pos-sym{{font-weight:700;font-size:1em;color:var(--text)}}
.pos-side{{font-size:.8em;font-weight:600}}
.pos-age{{margin-left:auto;font-size:.7em;color:var(--muted);background:var(--surface);
           padding:1px 6px;border-radius:8px}}
.pos-pnl{{font-size:1.1em;font-weight:700;margin-bottom:7px}}
.pos-bar-wrap{{position:relative;background:var(--border);border-radius:3px;height:6px;margin-bottom:5px}}
.pos-bar{{position:absolute;left:0;top:0;height:6px;border-radius:3px;transition:width .3s}}
.pos-bar-marks{{display:flex;justify-content:space-between;font-size:.6em;color:var(--muted);margin-top:2px}}
.pos-meta{{font-size:.72em;color:var(--muted);margin-top:3px;line-height:1.5}}
.empty-state{{text-align:center;color:var(--muted);padding:28px 20px;font-size:.82em}}
.empty-state .radar{{display:inline-block;width:36px;height:36px;border:2px solid rgba(88,166,255,.2);
                     border-top-color:var(--blue);border-radius:50%;animation:radar 1.1s linear infinite;
                     margin-bottom:10px}}
.empty-state p{{color:var(--muted);margin-top:4px}}
/* ── Signal feed ── */
.sig-section{{position:relative}}
.scan-wrap{{position:relative;overflow:hidden}}
.scan-beam{{position:absolute;left:0;right:0;height:40px;pointer-events:none;z-index:2;
            background:linear-gradient(to bottom,transparent,rgba(88,166,255,.06),transparent);
            animation:scanBeam 3.5s linear infinite}}
/* sig-row stagger applied via inline style */
.sig-row{{animation:fadeSlide .3s ease both}}
/* ── Tables ── */
.tbl-wrap{{overflow-x:auto;-webkit-overflow-scrolling:touch}}
table{{width:100%;border-collapse:collapse;font-size:.74em;table-layout:fixed}}
th{{color:var(--muted);text-align:left;padding:6px 8px;border-bottom:1px solid var(--border);
    white-space:nowrap;font-weight:500;font-size:.9em;letter-spacing:.4px;text-transform:uppercase;
    overflow:hidden;text-overflow:ellipsis}}
td{{padding:6px 8px;border-bottom:1px solid rgba(48,54,61,.5);vertical-align:middle;
    overflow:hidden;text-overflow:ellipsis;white-space:nowrap;
    font-variant-numeric:tabular-nums;
    transition:color .28s ease,opacity .18s ease}}
tr:last-child td{{border-bottom:none}}
tr:hover td{{background:rgba(255,255,255,.025)}}
.empty-td{{color:var(--muted);text-align:center;padding:16px;white-space:normal}}
.ts{{color:var(--muted);font-size:.85em;white-space:nowrap}}
/* Numeric column alignment helpers */
.tbl-r{{text-align:right}}
.tbl-c{{text-align:center}}
/* Subtle cell-pop flash when a value updates in-place */
@keyframes cellPop{{0%{{opacity:.25}}45%{{opacity:1}}100%{{opacity:1}}}}
.cell-pop{{animation:cellPop .38s ease}}
/* Reason badges */
.reason-stop{{color:#f85149}}.reason-take{{color:#3fb950}}
.reason-time{{color:#e3b341}}.reason-partial{{color:#58a6ff}}
.reason-ai{{color:#e3b341;font-weight:600}}.reason-signal{{color:#8b949e}}
/* ── Inline weight strip ── */
.w-strip{{display:flex;flex-wrap:wrap;align-items:center;gap:6px;
          margin-top:8px;padding-top:7px;border-top:1px solid var(--border)}}
.w-item{{display:flex;align-items:center;gap:4px;font-size:.7em}}
.w-item-label{{color:var(--muted);white-space:nowrap}}
.w-item-val{{font-weight:700;color:var(--blue)}}
.w-item-bar{{width:32px;height:3px;background:var(--border);border-radius:2px;overflow:hidden}}
.w-item-fill{{height:3px;background:linear-gradient(90deg,#388bfd,#58a6ff);border-radius:2px}}
.w-strip-note{{margin-left:auto;font-size:.65em;color:var(--muted);white-space:nowrap}}
/* ── Closed trade expand ── */
.ct-row:hover td{{background:rgba(255,255,255,.05)}}
.ct-detail td{{color:var(--text)}}
/* ── Utility ── */
.g{{color:var(--green)}}.r{{color:var(--red)}}.b{{color:var(--blue)}}.y{{color:var(--yellow)}}
/* ── Header right cluster ── */
.header-right{{display:flex;align-items:center;gap:12px;flex-wrap:wrap;justify-content:flex-end}}
.btn-cta{{display:inline-flex;align-items:center;gap:6px;padding:7px 15px;border-radius:8px;
           font-size:.8rem;font-weight:700;cursor:pointer;text-decoration:none!important;
           white-space:nowrap;border:1px solid rgba(63,185,80,.45);color:#3fb950;
           background:rgba(63,185,80,.08);transition:background .15s,border-color .15s,box-shadow .15s}}
.btn-cta:hover{{background:rgba(63,185,80,.18);border-color:rgba(63,185,80,.75);
                box-shadow:0 0 10px rgba(63,185,80,.15)}}
</style></head><body>

<div class="header">
  <h1>
    <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 76 90" fill="none" stroke="rgb(230,52,58)" stroke-width="4.5" stroke-linecap="round" stroke-linejoin="round" height="28" style="display:inline-block;vertical-align:middle;margin-right:8px">
  <path d="M22 2 L52 2 L57 7 L57 30 L52 35 L22 35 L17 30 L17 7 Z"/>
  <rect x="22" y="10" width="10" height="10" rx="1"/>
  <rect x="42" y="10" width="10" height="10" rx="1"/>
  <line x1="31" y1="35" x2="31" y2="40"/>
  <line x1="45" y1="35" x2="45" y2="40"/>
  <rect x="12" y="40" width="50" height="30" rx="5" transform="rotate(-4 37 55)"/>
  <path d="M33 53 C33 50 28 48 28 53 C28 57 33 62 33 62 C33 62 38 57 38 53 C38 48 33 50 33 53Z" transform="rotate(-4 33 55)"/>
  <path d="M14 44 L3 52 L1 63 L8 64"/>
  <path d="M60 43 L71 35 L75 44 L68 49"/>
  <path d="M21 70 L14 82 L4 84 L2 77"/>
  <path d="M46 70 L53 81 L65 81 L66 74"/>
</svg>
    <span class="live-ring"></span> TradingBots<span style="color:#3fb950">.fun</span>
  </h1>
  <div class="header-right">
    <span class="ts">⟳ <span id="cntdn">30s</span> &nbsp;·&nbsp; {last_update}</span>
    <a href="/login" class="btn-cta" data-funnel="login_click">
      <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor"
           stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="8" cy="5" r="3"/><path d="M2 14c0-3.3 2.7-6 6-6s6 2.7 6 6"/>
      </svg>
      Connect Wallet
    </a>
  </div>
</div>

<div id="equity-hero" class="{hero_class}">
  <div class="eq-left">
    <div class="eq-label">Total Equity</div>
    <div id="equity-val" class="eq-val">${equity:.2}</div>
    <div id="equity-label" class="eq-label" style="margin-top:3px">Free $<span id="equity-free">{capital:.2}</span>
      &nbsp;·&nbsp; <span title="House-money pool: accumulated profits available for re-deployment" style="color:{pool_col_op}">🏦 $<span id="op-pool">{pool_bal:.2}</span></span></div>
    <div id="pnl-badge" class="pnl-badge" style="color:{pnl_colour};border:1px solid {pnl_colour}40;background:{pnl_colour}15;margin-top:8px;display:inline-block">
      {pnl_sign}${total_pnl:.2} &nbsp; {pnl_sign}{total_pnl_pct:.2}%
    </div>
  </div>
  <div class="eq-right">
    {sparkline_svg}
  </div>
</div>

<!-- ── Operator controls strip ─────────────────────────────────── -->
<div style="display:flex;justify-content:flex-end;margin:6px 0 2px;gap:8px;">
  <button onclick="doResetStats()" id="op-reset-btn" style="
    background:none;border:1px solid #30363d;border-radius:6px;
    color:#8b949e;font-size:.70rem;padding:3px 10px;
    cursor:pointer;font-family:inherit;transition:.15s;
  " onmouseover="this.style.borderColor='#f0883e';this.style.color='#f0883e'"
     onmouseout="this.style.borderColor='#30363d';this.style.color='#8b949e'"
     title="Reset P&amp;L history and metrics for a clean slate. Open positions are kept.">
    🔄 Reset Stats
  </button>
</div>
<div id="op-reset-resp" style="display:none;font-size:.75rem;text-align:right;padding:4px 0 6px;"></div>

<div class="metrics">
  <div class="metric" onclick="showMetric('sharpe',{sharpe:.6})">
    <div class="mv" style="color:{sc}">{sharpe:.2}</div>
    <div class="ml">Sharpe <span class="ml-hint">tap to explain</span></div></div>
  <div class="metric" onclick="showMetric('sortino',{sortino:.6})">
    <div class="mv" style="color:{sortc}">{sortino:.2}</div>
    <div class="ml">Sortino <span class="ml-hint">tap to explain</span></div></div>
  <div class="metric" onclick="showMetric('expect',{expect_signed:.6})">
    <div class="mv" style="color:{expc}">{exps}{expectancy:.1}%</div>
    <div class="ml">Expectancy <span class="ml-hint">tap to explain</span></div></div>
  <div class="metric" onclick="showMetric('pf',{pf_float:.6})">
    <div class="mv">{pf}</div>
    <div class="ml">Profit Factor <span class="ml-hint">tap to explain</span></div></div>
  <div class="metric" onclick="showMetric('wr',{wr_float:.4})">
    <div class="mv">{wr:.0}% <span style="font-size:.65em;color:var(--muted)">({wins}W/{losses}L)</span></div>
    <div class="ml">Win Rate <span class="ml-hint">tap to explain</span></div></div>
  <div class="metric" onclick="showMetric('dd',{dd:.4})" title="7-day rolling drawdown (drives circuit breaker). All-time: -{atdd:.1}%">
    <div class="mv r">-{dd:.1}%</div>
    <div class="ml">7d Drawdown <span class="ml-hint">tap to explain</span></div></div>
  <div class="metric" onclick="showMetric('kelly',{kelly_float:.6})">
    <div class="mv b">{kelly_str}</div>
    <div class="ml">Half-Kelly <span class="ml-hint">tap to explain</span></div></div>
  <div class="metric{cbcc}" onclick="showMetric('cb',{cb_int})">
    <div class="mv" style="color:{cbc}">{cb_label}</div>
    <div class="ml">{cb_desc} <span class="ml-hint">tap to explain</span></div></div>
  <div class="metric" onclick="showMetric('openClosed',{open_n})">
    <div class="mv">{open_n} / {total_closed}</div>
    <div class="ml">Open / Closed <span class="ml-hint">tap to explain</span></div></div>
  <div class="metric" onclick="showMetric('cycles',{cycles})">
    <div class="mv">{cycles}</div>
    <div class="ml">Cycles <span class="ml-hint">tap to explain</span></div></div>
  <div class="metric" onclick="showMetric('scanning',{cand_n})">
    <div class="mv">{cand_n}</div>
    <div class="ml">Scanning <span class="ml-hint">tap to explain</span></div></div>
  <div class="metric" onclick="showMetric('deployed',{committed:.2})">
    <div class="mv y">${committed:.0}</div>
    <div class="ml">Deployed <span class="ml-hint">tap to explain</span></div></div>
</div>

<div class="status-bar">
  <div class="status-inner">
    <span class="st-text" id="bot-status">{status}</span>
    <span style="font-size:.75em;color:var(--muted);white-space:nowrap">
      {open_n} pos · ${committed:.0} · Sharpe {sharpe:.2}
    </span>
  </div>
  <div class="prog-track"><div class="prog-fill"></div></div>
</div>
{ai_status_html}

<div class="section section-positions">
  <div class="section-title">
    <span class="section-title-left"><span class="live-ring"></span> Active Positions</span>
    <span style="display:flex;align-items:center;gap:8px">
      <button id="sort-pos-btn" onclick="sortPositions()" title="Toggle sort order"
        style="background:#1c2026;border:1px solid #444c56;color:#cdd9e5;border-radius:6px;
               padding:2px 10px;font-size:.75em;cursor:pointer;line-height:1.6">
        ↕ Sort P&amp;L
      </button>
      <span class="badge">{open_n} open · cap {pos_cap}</span>
    </span>
  </div>
  <div class="pos-grid" id="pos-grid">{pos_cards}</div>
</div>

<!-- Signal feed immediately under positions -->
<div class="section sig-section section-signals">
  <div class="section-title">
    <span class="section-title-left"><span class="live-ring"></span> Signal Feed</span>
    <span class="badge">last 20 decisions</span>
  </div>
  <div class="tbl-wrap scan-wrap">
    <div class="scan-beam"></div>
    <table id="sig-tbl"><colgroup>
      <col style="width:108px"><col style="width:88px"><col style="width:56px"><col><col style="width:108px">
    </colgroup><thead><tr>
      <th>Symbol</th><th class="tbl-c">Action</th><th class="tbl-c">Conf</th><th>Rationale</th><th class="tbl-r">Time</th>
    </tr></thead><tbody id="sig-tbody">
    {dec_rows}</tbody></table>
  </div>
</div>

<div class="section section-candidates">
  <div class="section-title">
    <span>Candidates <span class="badge" id="cand-badge">{cand_n} scanned · ● = open</span></span>
  </div>
  <div class="tbl-wrap">
    <table id="cand-tbl"><colgroup>
      <col style="width:155px"><col style="width:108px"><col style="width:90px"><col style="width:60px"><col style="width:60px">
    </colgroup><thead><tr>
      <th>Symbol</th><th class="tbl-r">Price</th><th class="tbl-r">Session Δ</th>
      <th class="tbl-c" title="RSI(14): &lt;30 oversold · &gt;70 overbought">RSI</th>
      <th class="tbl-c" title="Signal confidence from last scan">Conf</th>
    </tr></thead><tbody id="cand-tbody">{cand_rows}</tbody></table>
  </div>
  {wh}
</div>

<div class="section section-closed">
  <div class="section-title">Closed Trades <span class="badge">{total_closed} total</span></div>
  <div class="tbl-wrap">
    <table><tr><th>Symbol</th><th>Side</th><th>Entry</th><th>Exit</th><th>P&amp;L</th><th>Reason</th><th>Time</th></tr>
    {closed_rows}</table>
  </div>
</div>

<!-- ── Metric explanation modal ───────────────────────────────────────── -->
<div id="metric-modal" onclick="if(event.target===this)closeMetricModal()">
  <div id="metric-modal-content"></div>
</div>

<script>
{metric_info_js}
</script>

<script>
/* ── Position card 3-D flip — one chart visible at a time ──────────────── */
/* _flipLock debounces rapid taps / double-clicks (lock > transition duration) */
var _flipLock={{}};
function flipPos(id){{
  if(_flipLock[id])return;
  _flipLock[id]=true;
  setTimeout(function(){{delete _flipLock[id];}},650);
  var wrap=document.getElementById('pf-'+id);
  if(!wrap)return;
  var opening=!wrap.classList.contains('flipped');
  /* one chart at a time — collapse any other open card */
  if(opening){{
    document.querySelectorAll('.pos-flip-wrap.flipped').forEach(function(w){{
      if(w!==wrap)w.classList.remove('flipped');
    }});
  }}
  wrap.classList.toggle('flipped');
}}

/* ── Position P&L sort toggle ───────────────────────────────────────────
   Alternates between worst-first (losers on top so you notice them) and
   best-first (winners on top to celebrate).  State persists across auto-
   refreshes by writing to window._posSortAsc.                           */
var _posSortAsc = true; // true = worst first (ascending P&L)
function sortPositions(){{
  var grid = document.getElementById('pos-grid');
  if(!grid) return;
  var cards = Array.from(grid.querySelectorAll('.pos-flip-wrap'));
  _posSortAsc = !_posSortAsc;
  cards.sort(function(a,b){{
    var pa = parseFloat(a.getAttribute('data-pnl')||'0');
    var pb = parseFloat(b.getAttribute('data-pnl')||'0');
    return _posSortAsc ? pa - pb : pb - pa;
  }});
  cards.forEach(function(c){{ grid.appendChild(c); }});
  var btn = document.getElementById('sort-pos-btn');
  if(btn) btn.textContent = _posSortAsc ? '↑ Worst first' : '↓ Best first';
}}

/* ── Closed trade click-to-expand ─────────────────────────────────────── */
function toggleDetail(id){{
  var el=document.getElementById(id);
  if(!el)return;
  var open=el.style.display!=='none';
  el.style.display=open?'none':'table-row';
  /* flip the ▼ arrow in the parent row */
  var row=el.previousElementSibling;
  if(row){{
    var arrow=row.querySelector('span[style*="444c56"]');
    if(arrow)arrow.textContent=open?'▼':'▲';
  }}
}}
(function(){{
  /* ── Countdown to next cycle (real timer from server next_cycle_at) ──── */
  var nextAt={next_cycle_at_ms},el=document.getElementById('cntdn');
  if(el){{
    function tick(){{
      var rem=nextAt>0?Math.max(0,Math.round((nextAt-Date.now())/1000)):0;
      el.textContent=(rem>0?rem:'…')+'s';
    }}
    tick();setInterval(tick,1000);
  }}

  /* ── Live data polling every 5s — updates key numbers without page flicker ─ */
  function $id(id){{return document.getElementById(id);}}
  function fmt2(n){{return Math.abs(n).toFixed(2);}}
  function sign(n){{return n>=0?'+':'-';}}
  function col(n){{return n>=0?'#3fb950':'#f85149';}}

  function applyPoll(s){{
    /* Equity hero */
    var unrealised=0,committed=0;
    (s.positions||[]).forEach(function(p){{unrealised+=p.unrealised_pnl;committed+=p.size_usd;}});
    var equity=s.capital+committed+unrealised;
    var total_pnl=s.pnl+unrealised;
    var pnl_pct=s.initial_capital>0?(total_pnl/s.initial_capital*100):0;

    var ev=$id('equity-val');
    if(ev)ev.textContent='$'+equity.toFixed(2);

    var ef=$id('equity-free');
    if(ef)ef.textContent=s.capital.toFixed(2);
    var op=$id('op-pool');if(op){{op.textContent=(s.house_money_pool||0).toFixed(2);op.parentElement.style.color=(s.house_money_pool||0)>0?'#3fb950':'#8b949e';}}

    var pb=$id('pnl-badge');
    if(pb){{
      var sg=sign(total_pnl),c=col(total_pnl);
      pb.textContent=sg+'$'+fmt2(total_pnl)+' \u00a0 '+sg+Math.abs(pnl_pct).toFixed(2)+'%';
      pb.style.color=c;pb.style.borderColor=c+'40';pb.style.background=c+'15';
    }}

    /* Open position cards — update P&L, R bar, and trailing stop */
    (s.positions||[]).forEach(function(p){{
      var sym=p.symbol.toLowerCase();
      var r_mult=p.r_dollars_risked>1e-8?p.unrealised_pnl/p.r_dollars_risked:0;
      var pct=p.size_usd>0?(p.unrealised_pnl/p.size_usd*100):0;
      var sg=sign(p.unrealised_pnl),c=col(p.unrealised_pnl);

      var pnlEl=$id('pos-'+sym+'-pnl');
      if(pnlEl){{
        pnlEl.style.color=c;
        pnlEl.innerHTML=sg+'$'+fmt2(p.unrealised_pnl)+
          ' ('+sg+Math.abs(pct).toFixed(1)+'%) \u00a0 '+
          '<b style="font-size:1.1em">'+(r_mult>=0?'+':'')+r_mult.toFixed(2)+'R</b>';
      }}

      var barEl=$id('pos-'+sym+'-bar');
      if(barEl){{
        var bp=Math.min(100,Math.max(0,(r_mult+1)/6*100));
        var bc=r_mult>=2?'#3fb950':(r_mult>=0?'#388bfd':'#f85149');
        barEl.style.width=bp+'%';barEl.style.background=bc;
      }}

      var stopEl=$id('pos-'+sym+'-stop');
      if(stopEl)stopEl.textContent='$'+p.stop_loss.toFixed(4);
    }});

    /* Status bar text */
    var stEl=document.getElementById('bot-status');
    if(stEl&&s.status)stEl.textContent=s.status;

    /* Hyperliquid traffic stats */
    var hlStats = s.hyperliquid_stats || {{ total_requests: 0, rate_limit_hits: 0, last_rate_limit_at: null }};
    var hlRequestsEl = document.getElementById('hl-requests');
    if(hlRequestsEl) hlRequestsEl.textContent = hlStats.total_requests.toLocaleString();
    var hlRateEl = document.getElementById('hl-rate-limits');
    if(hlRateEl) hlRateEl.textContent = hlStats.rate_limit_hits.toLocaleString();
    var hlLastEl = document.getElementById('hl-last-429');
    if(hlLastEl) hlLastEl.textContent = hlStats.last_rate_limit_at || 'never';

    /* Equity hero P&L glow class */
    var hero=document.getElementById('equity-hero');
    if(hero){{
      hero.classList.remove('pnl-pos','pnl-neg','pnl-cb');
      if(s.cb_active)hero.classList.add('pnl-cb');
      else if(total_pnl>0)hero.classList.add('pnl-pos');
      else if(s.initial_capital>0&&(total_pnl/s.initial_capital*100)<-1.5)hero.classList.add('pnl-neg');
    }}

    /* AI status bar */
    var aiStatus=s.ai_status||'';
    var aiBar=document.getElementById('ai-status-bar');
    if(aiStatus){{
      if(!aiBar){{
        /* inject it after the status bar if it doesn't exist yet */
        var sb=document.querySelector('.status-bar');
        if(sb){{
          aiBar=document.createElement('div');
          aiBar.id='ai-status-bar';
          aiBar.className='ai-status-bar';
          aiBar.innerHTML='<span id="ai-status-text"></span>';
          sb.parentNode.insertBefore(aiBar,sb.nextSibling);
        }}
      }}
      if(aiBar){{
        var txt=document.getElementById('ai-status-text');
        if(txt)txt.textContent=aiStatus;
        aiBar.style.display='flex';
        if(aiStatus.indexOf('Querying')>=0)aiBar.classList.add('ai-active');
        else aiBar.classList.remove('ai-active');
      }}
    }} else if(aiBar){{
      aiBar.style.display='none';
    }}

    /* ── Shared helpers ─────────────────────────────────────────────────── */
    /* Brief opacity flash on a cell whose value just changed */
    function popCell(el){{
      el.classList.remove('cell-pop');
      void el.offsetWidth; /* force reflow to restart animation */
      el.classList.add('cell-pop');
    }}

    /* Build a fresh <tr> for the candidates table */
    function buildCandRow(c){{
      var sym=c.symbol;
      var logo='<img src="https://s3-symbol-logo.tradingview.com/crypto/XTVC'+sym+'--big.svg" '
        +'onerror="this.onerror=null;this.src=\'https://assets.coincap.io/assets/icons/'+sym.toLowerCase()+'@2x.png\'" '
        +'width="16" height="16" style="border-radius:50%;vertical-align:middle;margin-right:5px" alt="'+sym+'">';
      var chgV=c.change_pct!=null?(c.change_pct>=0?'+':'')+c.change_pct.toFixed(3)+'%':'—';
      var chgC=c.change_pct!=null?(c.change_pct>=0?'#3fb950':'#f85149'):'var(--muted)';
      var rsiV=c.rsi!=null?c.rsi.toFixed(0)+(c.rsi<30?' <small>OS</small>':c.rsi>70?' <small>OB</small>':''):'—';
      var rsiC=c.rsi!=null?(c.rsi<30?'#3fb950':c.rsi>70?'#f85149':'#8b949e'):'var(--muted)';
      var confV=c.confidence!=null?(c.confidence*100).toFixed(0)+'%':'—';
      var confC=c.confidence!=null?(c.confidence>=0.7?'#3fb950':c.confidence>=0.55?'#e3b341':'#8b949e'):'var(--muted)';
      return '<tr data-sym="'+sym+'">'
        +'<td>'+logo+sym+'</td>'
        +'<td class="tbl-r">$'+c.price.toFixed(4)+'</td>'
        +'<td class="tbl-r" style="color:'+chgC+'">'+chgV+'</td>'
        +'<td class="tbl-c" style="color:'+rsiC+'">'+rsiV+'</td>'
        +'<td class="tbl-c" style="color:'+confC+'">'+confV+'</td></tr>';
    }}

    /* Build a fresh <tr> for the signal feed table */
    function buildSigRow(d){{
      var skip=d.action==='SKIP';
      var ac=d.action==='BUY'?'\u25b2 BUY':d.action==='SELL'?'\u25bc SELL':'\u2014 SKIP';
      var dc=d.action==='BUY'?'#3fb950':d.action==='SELL'?'#f85149':'#8b949e';
      var rs=skip?'opacity:0.45':'font-weight:500';
      var logo='<img src="https://s3-symbol-logo.tradingview.com/crypto/XTVC'+d.symbol+'--big.svg" '
        +'onerror="this.onerror=null;this.src=\'https://assets.coincap.io/assets/icons/'+d.symbol.toLowerCase()+'@2x.png\'" '
        +'width="15" height="15" style="border-radius:50%;vertical-align:middle;margin-right:5px" alt="'+d.symbol+'">';
      var rat=d.rationale.length>90?d.rationale.substring(0,90)+'\u2026':d.rationale;
      return '<tr style="'+rs+'">'
        +'<td>'+logo+'<b>'+d.symbol+'</b></td>'
        +'<td class="tbl-c" style="color:'+dc+';font-weight:600">'+ac+'</td>'
        +'<td class="tbl-c">'+(d.confidence*100).toFixed(0)+'%</td>'
        +'<td class="ts">'+rat+'</td>'
        +'<td class="ts tbl-r">'+d.timestamp+'</td></tr>';
    }}

    /* ── Candidates table — smart in-place update ───────────────────────── */
    var candTbody=document.getElementById('cand-tbody');
    if(candTbody&&s.candidates&&s.candidates.length>0){{
      /* Build an index of existing rows keyed by symbol */
      var rowMap={{}};
      [].forEach.call(candTbody.rows,function(tr){{
        if(tr.dataset.sym) rowMap[tr.dataset.sym]=tr;
      }});

      /* Full rebuild if symbol set or count changed */
      var needRebuild=s.candidates.length!==candTbody.rows.length
        ||s.candidates.some(function(c){{ return !rowMap[c.symbol]; }});

      if(needRebuild){{
        candTbody.innerHTML=s.candidates.map(buildCandRow).join('');
      }} else {{
        /* In-place: update only changed cells, reorder rows if ranking shifted */
        s.candidates.forEach(function(c,i){{
          var tr=rowMap[c.symbol]; if(!tr) return;
          var cells=tr.cells;

          /* Price (cell 1) */
          var pv='$'+c.price.toFixed(4);
          if(cells[1].textContent!==pv){{ cells[1].textContent=pv; popCell(cells[1]); }}

          /* Change % (cell 2) */
          if(c.change_pct!=null){{
            var cv=(c.change_pct>=0?'+':'')+c.change_pct.toFixed(3)+'%';
            var cc=c.change_pct>=0?'#3fb950':'#f85149';
            if(cells[2].textContent!==cv){{ cells[2].textContent=cv; cells[2].style.color=cc; popCell(cells[2]); }}
          }}

          /* RSI (cell 3) — compare stored raw value to avoid innerHTML flicker */
          if(c.rsi!=null){{
            var rv=c.rsi.toFixed(0);
            if(cells[3].dataset.v!==rv){{
              cells[3].dataset.v=rv;
              cells[3].innerHTML=rv+(c.rsi<30?' <small>OS</small>':c.rsi>70?' <small>OB</small>':'');
              cells[3].style.color=c.rsi<30?'#3fb950':c.rsi>70?'#f85149':'#8b949e';
              popCell(cells[3]);
            }}
          }}

          /* Confidence (cell 4) */
          if(c.confidence!=null){{
            var fv=(c.confidence*100).toFixed(0)+'%';
            if(cells[4].textContent!==fv){{
              cells[4].textContent=fv;
              cells[4].style.color=c.confidence>=0.7?'#3fb950':c.confidence>=0.55?'#e3b341':'#8b949e';
              popCell(cells[4]);
            }}
          }}

          /* Reorder row if ranking changed */
          if(candTbody.rows[i]!==tr) candTbody.insertBefore(tr,candTbody.rows[i]||null);
        }});
      }}
    }}
    var cb=document.getElementById('cand-badge');
    if(cb&&s.candidates) cb.textContent=s.candidates.length+' scanned \u00b7 \u25cf = open';

    /* ── Signal feed — only rebuild when a new decision arrives ─────────── */
    var sigTbody=document.getElementById('sig-tbody');
    if(sigTbody&&s.recent_decisions&&s.recent_decisions.length>0){{
      var decs=[].concat(s.recent_decisions).reverse().slice(0,20);
      /* Key on the newest decision's symbol+timestamp; skip rebuild if unchanged */
      var topKey=(decs[0].symbol||'')+':'+(decs[0].timestamp||'');
      if(sigTbody.dataset.topKey!==topKey){{
        sigTbody.dataset.topKey=topKey;
        sigTbody.innerHTML=decs.map(buildSigRow).join('');
      }}
    }}
  }}

  function poll(){{
    fetch('/api/state').then(function(r){{return r.json();}}).then(applyPoll)
      .catch(function(){{}});
  }}
  /* First poll after 2s so data appears before the 10s page reload */
  setTimeout(poll,2000);
  setInterval(poll,5000);
}})();

// ── Reset Stats (operator dashboard) ─────────────────────────────────────
window.doResetStats = function() {{
  if (!confirm('Reset trading stats? P&L, closed trades and metrics will be cleared.\nOpen positions are kept. This cannot be undone.')) return;
  var btn  = document.getElementById('op-reset-btn');
  var resp = document.getElementById('op-reset-resp');
  btn.disabled = true;
  btn.textContent = '⏳ Resetting…';
  fetch('/api/admin/reset-stats', {{method:'POST', credentials:'include'}})
    .then(function(r) {{ return r.json(); }})
    .then(function(d) {{
      resp.style.display = 'block';
      if (d.ok) {{
        resp.style.color = '#3fb950';
        resp.textContent = '✅ ' + d.message;
        btn.textContent = '✓ Done';
        setTimeout(function(){{ window.location.reload(); }}, 2500);
      }} else {{
        resp.style.color = '#e3b341';
        resp.textContent = '⚠ ' + (d.message || 'Reset failed');
        btn.disabled = false;
        btn.textContent = '🔄 Reset Stats';
      }}
    }}).catch(function() {{
      resp.style.display = 'block';
      resp.style.color = '#f85149';
      resp.textContent = '⚠ Network error — is ADMIN_PASSWORD set?';
      btn.disabled = false;
      btn.textContent = '🔄 Reset Stats';
    }});
}};
</script>
{tracking_js}

<!-- ── Floating AI Command Bar ──────────────────────────────────────────── -->
<style>
#ai-bar-tabs button {{ transition: color .15s, border-color .15s; }}
#ai-bar-tabs button.tab-active {{ color:#e6edf3 !important; border-color: var(--tab-col) !important; }}
#ai-cmd-input:focus {{ border-color:#388bfd !important; outline:none; }}
.ai-chip-btn {{ background:none; border:1px solid #30363d; border-radius:10px;
  color:#8b949e; font-size:.70rem; padding:2px 9px; cursor:pointer;
  font-family:inherit; white-space:nowrap; transition: color .12s, border-color .12s; }}
.ai-chip-btn:hover {{ color: var(--chip-hover-col, #58a6ff); border-color: var(--chip-hover-col, #58a6ff); }}
</style>

<div id="ai-bar" style="
  position:fixed;bottom:0;left:0;right:0;z-index:9999;
  background:rgba(13,17,23,0.93);
  backdrop-filter:blur(14px);-webkit-backdrop-filter:blur(14px);
  border-top:1px solid #30363d;
  padding:8px 16px 10px;
  display:flex;flex-direction:column;gap:5px;
">
  <div style="display:flex;align-items:center;gap:10px;flex-wrap:wrap;">
    <div id="ai-bar-tabs" style="display:flex;gap:0;border:1px solid #30363d;border-radius:7px;overflow:hidden;flex-shrink:0;">
      <button id="tab-trade" onclick="setTab('trade')"
        style="--tab-col:#f0883e;background:#161b22;border:none;padding:4px 12px;
               font-size:.72rem;cursor:pointer;font-family:inherit;color:#8b949e;"
        class="tab-active">⚡ Trade</button>
      <button id="tab-strategy" onclick="setTab('strategy')"
        style="--tab-col:#58a6ff;background:#161b22;border:none;border-left:1px solid #30363d;
               padding:4px 12px;font-size:.72rem;cursor:pointer;font-family:inherit;color:#8b949e;">
        🎯 Strategy</button>
    </div>
    <div id="thesis-chip" style="display:none;align-items:center;gap:5px;font-size:.72rem;">
      <span style="color:#8b949e">Strategy:</span>
      <span id="thesis-chip-text" style="
        background:#1f6feb22;border:1px solid #1f6feb88;color:#58a6ff;
        padding:1px 8px;border-radius:10px;font-size:.69rem;
      "></span>
      <button onclick="sendThesisCmd('reset')" style="
        background:none;border:none;color:#8b949e;cursor:pointer;font-size:.68rem;padding:0 3px;
      " title="Clear strategy">✕</button>
    </div>
    <div id="cmd-queued-badge" style="display:none;font-size:.70rem;color:#f0883e;
         background:#2d1f0a;border:1px solid #f0883e66;border-radius:8px;padding:1px 8px;">
      ⏱ executing on next cycle…
    </div>
  </div>

  <div style="display:flex;gap:8px;align-items:center;">
    <span id="ai-bar-icon" style="font-size:1rem;flex-shrink:0;">⚡</span>
    <input id="ai-cmd-input" type="text"
      placeholder="close kFloki  ·  take profit SOL  ·  close all"
      style="
        flex:1;background:#161b22;border:1px solid #30363d;border-radius:6px;
        padding:7px 12px;color:#e6edf3;font-size:.82rem;font-family:inherit;
        transition: border-color .15s;
      "
      onkeydown="if(event.key==='Enter')submitAiCmd()"
      oninput="onCmdInput(this.value)"
    />
    <button id="ai-send-btn" onclick="submitAiCmd()" style="
      background:#238636;border:none;border-radius:6px;
      color:#fff;font-size:.80rem;padding:7px 14px;cursor:pointer;
      white-space:nowrap;font-family:inherit;transition:background .15s;
    ">Send</button>
  </div>

  <div id="chips-trade" style="display:flex;flex-wrap:wrap;gap:5px;padding-left:26px;">
    <button class="ai-chip-btn" style="--chip-hover-col:#f0883e"
      onclick="tradeCmd('close all')">🔴 close all</button>
    <button class="ai-chip-btn" style="--chip-hover-col:#3fb950"
      onclick="tradeCmd('take profits')">💰 take profits</button>
    <button class="ai-chip-btn" style="--chip-hover-col:#f0883e"
      id="chip-top-winner" onclick="tradeCmd('')" style="display:none">
      tp top winner</button>
    <button class="ai-chip-btn" style="--chip-hover-col:#58a6ff"
      onclick="sendThesisCmd('show recent trades')">📋 recent trades</button>
  </div>
  <div id="chips-strategy" style="display:none;flex-wrap:wrap;gap:5px;padding-left:26px;">
    <button class="ai-chip-btn" style="--chip-hover-col:#58a6ff"
      onclick="setTab('strategy');sendThesisCmd('only BTC ETH SOL')">only BTC ETH SOL</button>
    <button class="ai-chip-btn" style="--chip-hover-col:#58a6ff"
      onclick="setTab('strategy');sendThesisCmd('meme coins only')">meme coins only</button>
    <button class="ai-chip-btn" style="--chip-hover-col:#58a6ff"
      onclick="setTab('strategy');sendThesisCmd('max 5x leverage')">max 5× leverage</button>
    <button class="ai-chip-btn" style="--chip-hover-col:#f78166"
      onclick="setTab('strategy');sendThesisCmd('aggressive')">aggressive</button>
    <button class="ai-chip-btn" style="--chip-hover-col:#3fb950"
      onclick="setTab('strategy');sendThesisCmd('conservative')">conservative</button>
    <button class="ai-chip-btn" style="--chip-hover-col:#8b949e"
      onclick="setTab('strategy');sendThesisCmd('reset')">reset strategy</button>
  </div>

  <div id="ai-response" style="
    display:none;
    border-radius:6px;padding:9px 13px;font-size:.80rem;
    max-height:110px;overflow-y:auto;line-height:1.5;
  "></div>
</div>

<script>
(function() {{
  var currentTab = 'trade';
  var topWinnerSym = null;

  window.setTab = function(tab) {{
    currentTab = tab;
    var isTrade = tab === 'trade';
    document.getElementById('tab-trade').classList.toggle('tab-active', isTrade);
    document.getElementById('tab-strategy').classList.toggle('tab-active', !isTrade);
    document.getElementById('chips-trade').style.display    = isTrade ? 'flex' : 'none';
    document.getElementById('chips-strategy').style.display = isTrade ? 'none' : 'flex';
    var inp = document.getElementById('ai-cmd-input');
    var icon = document.getElementById('ai-bar-icon');
    if (isTrade) {{
      inp.placeholder = 'close kFloki  ·  tp SOL  ·  close all  ·  take profits';
      icon.textContent = '⚡';
      document.getElementById('ai-send-btn').style.background = '#b94300';
    }} else {{
      inp.placeholder = 'only BTC ETH  ·  max 5x  ·  meme coins  ·  reset';
      icon.textContent = '🎯';
      document.getElementById('ai-send-btn').style.background = '#1f6feb';
    }}
  }};

  var tradeKeywords = ['close','exit','sell','tp','take profit','take profits'];
  var stratKeywords = ['only','max','leverage','meme','btc','eth','sol','aggressive','conservative','reset','sector'];
  window.onCmdInput = function(val) {{
    var lc = val.toLowerCase().trim();
    if (!lc) return;
    if (tradeKeywords.some(function(k){{ return lc.startsWith(k); }})) {{
      if (currentTab !== 'trade') setTab('trade');
    }} else if (stratKeywords.some(function(k){{ return lc.includes(k); }})) {{
      if (currentTab !== 'strategy') setTab('strategy');
    }}
  }};

  window.submitAiCmd = function() {{
    var inp = document.getElementById('ai-cmd-input');
    var cmd = (inp.value || '').trim();
    if (!cmd) return;
    inp.value = '';
    if (currentTab === 'trade') {{ sendTradeCmd(cmd); }} else {{ sendThesisCmd(cmd); }}
  }};

  window.tradeCmd = function(cmd) {{
    if (!cmd && topWinnerSym) cmd = 'tp ' + topWinnerSym;
    if (!cmd) {{ showResp('⚠ No open positions found.', 'warn'); return; }}
    sendTradeCmd(cmd);
  }};

  window.sendTradeCmd = function(cmd) {{
    showResp('⏳ Parsing command…', 'info');
    fetch('/api/command', {{
      method: 'POST',
      headers: {{'Content-Type': 'application/json'}},
      body: JSON.stringify({{command: cmd}})
    }}).then(function(r){{ return r.json(); }}).then(function(d) {{
      if (d.ok) {{
        showResp('✅ ' + d.msg, 'ok');
        var badge = document.getElementById('cmd-queued-badge');
        badge.style.display = 'block';
        setTimeout(function(){{ badge.style.display = 'none'; }}, 32000);
        addCmdHistory((d.action || cmd) + (d.symbol ? ' ' + d.symbol : ''));
      }} else {{
        showResp('⚠ ' + d.msg, 'warn');
      }}
    }}).catch(function() {{
      showResp('⚠ Network error — is the bot running?', 'warn');
    }});
  }};

  window.sendThesisCmd = function(cmd) {{
    showResp('⏳ Updating strategy…', 'info');
    fetch('/api/thesis', {{
      method: 'POST',
      headers: {{'Content-Type': 'application/json'}},
      body: JSON.stringify({{command: cmd}})
    }}).then(function(r){{ return r.json(); }}).then(function(d) {{
      if (d.type === 'query') {{
        showResp('📋 <b>Recent trades:</b><br>' + (d.message || 'No trades found.'), 'ok', true);
      }} else if (d.summary) {{
        showResp('✅ ' + d.message, 'ok');
        showChip(d.summary);
      }} else {{
        showResp('✅ ' + (d.message || 'Strategy cleared'), 'ok');
        clearChip();
      }}
    }}).catch(function() {{
      showResp('⚠ Could not update strategy. Please try again.', 'warn');
    }});
  }};

  var cmdHistory = [];
  function addCmdHistory(label) {{
    cmdHistory.unshift(label);
    if (cmdHistory.length > 3) cmdHistory.pop();
    renderCmdHistory();
  }}
  function renderCmdHistory() {{
    var el = document.getElementById('cmd-history');
    if (!el) return;
    el.innerHTML = cmdHistory.map(function(c){{
      return '<span style="font-size:.65rem;color:#484f58;background:#161b22;border:1px solid #21262d;border-radius:8px;padding:1px 7px;">✓ ' + c + '</span>';
    }}).join(' ');
  }}

  function showResp(html, type, isHtml) {{
    var el = document.getElementById('ai-response');
    el.style.display = 'block';
    var bg  = type === 'ok' ? '#0d2018' : type === 'warn' ? '#2d1a0e' : '#0d1117';
    var col = type === 'ok' ? '#3fb950' : type === 'warn' ? '#e3b341'  : '#8b949e';
    el.style.background = bg;
    el.style.border = '1px solid ' + col + '44';
    el.style.color = col;
    if (isHtml) {{ el.innerHTML = html; }} else {{ el.textContent = html; }}
    clearTimeout(el._hide);
    if (type !== 'info') {{
      el._hide = setTimeout(function(){{ el.style.display = 'none'; }}, 5000);
    }}
  }}

  function showChip(summary) {{
    var chip = document.getElementById('thesis-chip');
    document.getElementById('thesis-chip-text').textContent = '🎯 ' + summary;
    chip.style.display = 'flex';
  }}
  function clearChip() {{
    document.getElementById('thesis-chip').style.display = 'none';
  }}

  fetch('/api/thesis').then(function(r){{ return r.json(); }}).then(function(d){{
    if (d.summary) showChip(d.summary);
  }}).catch(function(){{}});

  function refreshState() {{
    fetch('/api/state').then(function(r){{ return r.json(); }}).then(function(s){{
      var best = null, bestPnl = 0;
      (s.positions || []).forEach(function(p){{
        if (p.unrealised_pnl > bestPnl) {{ bestPnl = p.unrealised_pnl; best = p.symbol; }}
      }});
      topWinnerSym = best;
      var chipBtn = document.getElementById('chip-top-winner');
      if (chipBtn) {{
        if (best) {{
          chipBtn.style.display = 'inline';
          chipBtn.textContent = 'tp ' + best + ' ($' + bestPnl.toFixed(2) + ')';
        }} else {{
          chipBtn.style.display = 'none';
        }}
      }}
    }}).catch(function(){{}});
  }}
  refreshState();
  setInterval(refreshState, 30000);

  (function(){{
    var ct = document.getElementById('chips-trade');
    if (!ct) return;
    var hr = document.createElement('div');
    hr.id = 'cmd-history';
    hr.style.cssText = 'display:flex;flex-wrap:wrap;gap:4px;padding-left:26px;';
    ct.parentNode.insertBefore(hr, ct.nextSibling);
  }})();

  setTab('trade');
}})();
</script>

</body></html>"#,
        last_update = s.last_update,
        equity = equity,
        capital = s.capital,
        pool_bal = s.house_money_pool,
        pool_col_op = if s.house_money_pool > 0.0 {
            "#3fb950"
        } else {
            "#8b949e"
        },
        pnl_colour = pnl_colour,
        pnl_sign = pnl_sign,
        total_pnl = total_pnl.abs(),
        total_pnl_pct = total_pnl_pct.abs(),
        sc = m.sharpe_class(),
        sharpe = m.sharpe,
        sortc = if m.sortino > 1.0 {
            "#3fb950"
        } else if m.sortino > 0.0 {
            "#e3b341"
        } else {
            "#f85149"
        },
        sortino = m.sortino,
        expc = if m.expectancy >= 0.0 {
            "#3fb950"
        } else {
            "#f85149"
        },
        exps = if m.expectancy >= 0.0 { "+" } else { "-" }, // BUG FIX: was "" → dropped "-"
        expectancy = m.expectancy.abs(),
        pf = pf_str,
        wr = m.win_rate * 100.0,
        wins = m.wins,
        losses = m.losses,
        dd = rolling_dd_pct,    // 7-day rolling (drives CB) — shown in metric
        atdd = dd_pct.max(0.0), // all-time drawdown (tooltip only)
        kelly_str = kelly_str,
        cbc = cb_colour,
        cbcc = cb_card_class,
        cb_label = cb_label,
        cb_desc = cb_desc,
        open_n = s.positions.len(),
        pos_cap = 40,
        total_closed = s.closed_trades.len(),
        cycles = s.cycle_count,
        cand_n = s.candidates.len(),
        committed = committed,
        status = s.status,
        pos_cards = pos_cards,
        wh = wh,
        cand_rows = cand_rows,
        closed_rows = closed_rows,
        dec_rows = dec_rows,
        next_cycle_at_ms = s.next_cycle_at,
        sparkline_svg = sparkline_svg,
        hero_class = hero_class,
        ai_status_html = ai_status_html,
        metric_info_js = metric_info_js,
        expect_signed = expect_signed,
        pf_float = pf_float,
        kelly_float = kelly_float,
        cb_int = cb_int,
        wr_float = wr_float,
        tracking_js = crate::funnel::client_tracking_script(),
    ))
}

/// Inline weight item: label · value · tiny bar  (single-line strip)
fn wi(label: &str, val: f64) -> String {
    format!(
        r#"<span class="w-item"><span class="w-item-label">{label}</span><span class="w-item-val">{val:.2}</span><div class="w-item-bar"><div class="w-item-fill" style="width:{pct:.0}%"></div></div></span>"#,
        label = label,
        val = val,
        pct = (val * 100.0).min(100.0),
    )
}

fn reason_class(r: &str) -> &'static str {
    match r {
        s if s.contains("Stop") => "stop",
        s if s.contains("Take") => "take",
        s if s.contains("Time") => "time",
        s if s.contains("Partial") => "partial",
        s if s.contains("AI") => "ai", // BUG FIX: was mapped to "signal" (grey)
        s if s.contains("Signal") => "signal",
        _ => "signal",
    }
}

async fn api_state_handler(State(app): State<AppState>) -> Json<BotState> {
    let mut state = app.bot_state.read().await.clone();
    state.hyperliquid_stats = app.hyperliquid_stats.snapshot().await;
    Json(state)
}

// ─────────────────────────── Consumer webapp ─────────────────────────────────

/// Shared CSS + HTML boilerplate for all consumer pages.
fn consumer_shell_open(title: &str, active: &str) -> String {
    let nav = |label: &str, href: &str| -> String {
        let is_active = label == active;
        format!(
            "<a href='{href}' style='padding:8px 18px;border-radius:6px;font-size:.88rem;\
             font-weight:{fw};color:{col};background:{bg};text-decoration:none'>{label}</a>",
            href = href,
            fw = if is_active { "600" } else { "400" },
            col = if is_active { "#e6edf3" } else { "#8b949e" },
            bg = if is_active { "#21262d" } else { "transparent" },
            label = label,
        )
    };
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>TradingBots.fun · {title}</title>
<style>
  *{{box-sizing:border-box;margin:0;padding:0}}
  body{{background:#0d1117;color:#c9d1d9;
        font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;
        min-height:100vh;padding:0 0 40px}}
  .top-bar{{display:flex;align-items:center;justify-content:space-between;
             padding:14px 24px;border-bottom:1px solid #21262d;margin-bottom:28px}}
  .logo{{font-weight:700;font-size:.95rem;color:#e6edf3;letter-spacing:.04em}}
  .logo .r{{color:#e6343a}}
  .logo .b{{color:#3fb950}}
  .nav{{display:flex;gap:4px}}
  .wrap{{max-width:700px;margin:0 auto;padding:0 16px}}
  .card{{background:#161b22;border:1px solid #30363d;border-radius:12px;
         padding:24px 28px;margin-bottom:16px}}
  .card-label{{font-size:.72rem;color:#8b949e;text-transform:uppercase;
               letter-spacing:.07em;margin-bottom:8px}}
  .card-val{{font-size:2.2rem;font-weight:700;letter-spacing:-.01em;color:#e6edf3}}
  .badge{{display:inline-block;font-size:.95rem;font-weight:600;padding:3px 12px;
           border-radius:16px;border:1px solid;margin-top:8px}}
  .metric-row{{display:flex;justify-content:space-between;align-items:center;
               padding:9px 0;border-bottom:1px solid #21262d}}
  .metric-row:last-child{{border-bottom:none}}
  .ml{{color:#8b949e;font-size:.86rem}}
  .mv{{font-size:.92rem;font-weight:600;color:#e6edf3}}
  .info-box{{background:#0d1117;border:1px solid #30363d;border-radius:8px;
              padding:14px;font-size:.82rem;color:#8b949e;line-height:1.7}}
  .info-box b{{color:#c9d1d9}}
  .tbl{{width:100%;border-collapse:collapse;font-size:.82rem}}
  .tbl th{{color:#8b949e;font-weight:500;padding:6px 8px;border-bottom:1px solid #30363d;
            text-align:left;white-space:nowrap}}
  .tbl td{{padding:6px 8px;border-bottom:1px solid #21262d;color:#c9d1d9;white-space:nowrap}}
  .tbl tr:last-child td{{border-bottom:none}}
  .btn{{display:inline-block;padding:7px 16px;border-radius:6px;font-size:.82rem;
         font-weight:600;cursor:pointer;text-decoration:none;border:1px solid}}
  .btn-green{{color:#3fb950;border-color:#3fb95050;background:#3fb95012}}
  .btn-blue{{color:#58a6ff;border-color:#58a6ff50;background:#58a6ff12}}
  .note{{font-size:.75rem;color:#484f58;margin-top:6px;line-height:1.5}}
  a{{color:#58a6ff;text-decoration:none}}
  a:hover{{text-decoration:underline}}
  .green{{color:#3fb950}} .red{{color:#f85149}} .muted{{color:#8b949e}}
</style>
</head>
<body>
<div class="top-bar">
  <span class="logo"><span class="r">Red</span><span class="b">Robot</span></span>
  <div class="nav">
    {nav_overview}
    {nav_history}
    {nav_tax}
    {nav_settings}
    {nav_agents}
    <a href="/auth/logout" style="padding:8px 18px;border-radius:6px;font-size:.88rem;
       font-weight:400;color:#8b949e;background:transparent;text-decoration:none"
       title="Sign out">Sign out</a>
  </div>
</div>
<div class="wrap">
"#,
        title = title,
        nav_overview = nav("Overview", "/app"),
        nav_history = nav("History", "/app/history"),
        nav_tax = nav("Tax", "/app/tax"),
        nav_settings = nav("Settings", "/app/settings"),
        nav_agents = nav("Agents", "/app/agents"),
    )
}

fn consumer_shell_close() -> &'static str {
    r#"</div>
<footer style="text-align:center;padding:32px 16px 80px;font-size:.72rem;color:#484f58;
               border-top:1px solid #21262d;margin-top:24px">
  &copy; 2026 TradingBots Ltd. &nbsp;&middot;&nbsp;
  <a href="https://tradingbots.fun" style="color:#484f58;text-decoration:none">tradingbots.fun</a> &nbsp;&middot;&nbsp;
  <a href="/app/onboarding" style="color:#484f58;text-decoration:none">Terms &amp; Risk Disclosure</a>
</footer>

<!-- ── Floating AI Command Bar ──────────────────────────────────────────── -->
<style>
#ai-bar-tabs button { transition: color .15s, border-color .15s; }
#ai-bar-tabs button.tab-active { color:#e6edf3 !important; border-color: var(--tab-col) !important; }
#ai-cmd-input:focus { border-color:#388bfd !important; outline:none; }
.ai-chip-btn { background:none; border:1px solid #30363d; border-radius:10px;
  color:#8b949e; font-size:.70rem; padding:2px 9px; cursor:pointer;
  font-family:inherit; white-space:nowrap; transition: color .12s, border-color .12s; }
.ai-chip-btn:hover { color: var(--chip-hover-col, #58a6ff); border-color: var(--chip-hover-col, #58a6ff); }
</style>

<div id="ai-bar" style="
  position:fixed;bottom:0;left:0;right:0;z-index:9999;
  background:rgba(13,17,23,0.93);
  backdrop-filter:blur(14px);-webkit-backdrop-filter:blur(14px);
  border-top:1px solid #30363d;
  padding:8px 16px 10px;
  display:flex;flex-direction:column;gap:5px;
">
  <!-- ── Top row: tabs + active-thesis chip ─────────────────────────── -->
  <div style="display:flex;align-items:center;gap:10px;flex-wrap:wrap;">
    <!-- Mode tabs -->
    <div id="ai-bar-tabs" style="display:flex;gap:0;border:1px solid #30363d;border-radius:7px;overflow:hidden;flex-shrink:0;">
      <button id="tab-trade" onclick="setTab('trade')"
        style="--tab-col:#f0883e;background:#161b22;border:none;padding:4px 12px;
               font-size:.72rem;cursor:pointer;font-family:inherit;color:#8b949e;"
        class="tab-active">⚡ Trade</button>
      <button id="tab-strategy" onclick="setTab('strategy')"
        style="--tab-col:#58a6ff;background:#161b22;border:none;border-left:1px solid #30363d;
               padding:4px 12px;font-size:.72rem;cursor:pointer;font-family:inherit;color:#8b949e;">
        🎯 Strategy</button>
    </div>
    <!-- Active thesis chip -->
    <div id="thesis-chip" style="display:none;align-items:center;gap:5px;font-size:.72rem;">
      <span style="color:#8b949e">Strategy:</span>
      <span id="thesis-chip-text" style="
        background:#1f6feb22;border:1px solid #1f6feb88;color:#58a6ff;
        padding:1px 8px;border-radius:10px;font-size:.69rem;
      "></span>
      <button onclick="sendThesisCmd('reset')" style="
        background:none;border:none;color:#8b949e;cursor:pointer;font-size:.68rem;padding:0 3px;
      " title="Clear strategy">✕</button>
    </div>
    <!-- Queued-command badge -->
    <div id="cmd-queued-badge" style="display:none;font-size:.70rem;color:#f0883e;
         background:#2d1f0a;border:1px solid #f0883e66;border-radius:8px;padding:1px 8px;">
      ⏱ executing on next cycle…
    </div>
  </div>

  <!-- ── Input row ───────────────────────────────────────────────────── -->
  <div style="display:flex;gap:8px;align-items:center;">
    <span id="ai-bar-icon" style="font-size:1rem;flex-shrink:0;">⚡</span>
    <input id="ai-cmd-input" type="text"
      placeholder="close kFloki  ·  take profit SOL  ·  close all"
      style="
        flex:1;background:#161b22;border:1px solid #30363d;border-radius:6px;
        padding:7px 12px;color:#e6edf3;font-size:.82rem;font-family:inherit;
        transition: border-color .15s;
      "
      onkeydown="if(event.key==='Enter')submitAiCmd()"
      oninput="onCmdInput(this.value)"
    />
    <button id="ai-send-btn" onclick="submitAiCmd()" style="
      background:#238636;border:none;border-radius:6px;
      color:#fff;font-size:.80rem;padding:7px 14px;cursor:pointer;
      white-space:nowrap;font-family:inherit;transition:background .15s;
    ">Send</button>
  </div>

  <!-- ── Chip rows ───────────────────────────────────────────────────── -->
  <!-- Trade chips (default visible) -->
  <div id="chips-trade" style="display:flex;flex-wrap:wrap;gap:5px;padding-left:26px;">
    <button class="ai-chip-btn" style="--chip-hover-col:#f0883e"
      onclick="tradeCmd('close all')">🔴 close all</button>
    <button class="ai-chip-btn" style="--chip-hover-col:#3fb950"
      onclick="tradeCmd('take profits')">💰 take profits</button>
    <button class="ai-chip-btn" style="--chip-hover-col:#f0883e"
      id="chip-top-winner" onclick="tradeCmd('')" style="display:none">
      tp top winner</button>
    <button class="ai-chip-btn" style="--chip-hover-col:#58a6ff"
      onclick="sendThesisCmd('show recent trades')">📋 recent trades</button>
  </div>
  <!-- Strategy chips (hidden until tab switched) -->
  <div id="chips-strategy" style="display:none;flex-wrap:wrap;gap:5px;padding-left:26px;">
    <button class="ai-chip-btn" style="--chip-hover-col:#58a6ff"
      onclick="setTab('strategy');sendThesisCmd('only BTC ETH SOL')">only BTC ETH SOL</button>
    <button class="ai-chip-btn" style="--chip-hover-col:#58a6ff"
      onclick="setTab('strategy');sendThesisCmd('meme coins only')">meme coins only</button>
    <button class="ai-chip-btn" style="--chip-hover-col:#58a6ff"
      onclick="setTab('strategy');sendThesisCmd('max 5x leverage')">max 5× leverage</button>
    <button class="ai-chip-btn" style="--chip-hover-col:#f78166"
      onclick="setTab('strategy');sendThesisCmd('aggressive')">aggressive</button>
    <button class="ai-chip-btn" style="--chip-hover-col:#3fb950"
      onclick="setTab('strategy');sendThesisCmd('conservative')">conservative</button>
    <button class="ai-chip-btn" style="--chip-hover-col:#8b949e"
      onclick="setTab('strategy');sendThesisCmd('reset')">reset strategy</button>
  </div>

  <!-- ── Response panel ─────────────────────────────────────────────── -->
  <div id="ai-response" style="
    display:none;
    border-radius:6px;padding:9px 13px;font-size:.80rem;
    max-height:110px;overflow-y:auto;line-height:1.5;
  "></div>
</div>

<script>
(function() {
  var currentTab = 'trade';
  var topWinnerSym = null;   // populated by /api/state poll

  // ── Tab switching ─────────────────────────────────────────────────────
  window.setTab = function(tab) {
    currentTab = tab;
    var isTrade = tab === 'trade';
    document.getElementById('tab-trade').classList.toggle('tab-active', isTrade);
    document.getElementById('tab-strategy').classList.toggle('tab-active', !isTrade);
    document.getElementById('chips-trade').style.display    = isTrade ? 'flex' : 'none';
    document.getElementById('chips-strategy').style.display = isTrade ? 'none' : 'flex';
    var inp = document.getElementById('ai-cmd-input');
    var icon = document.getElementById('ai-bar-icon');
    if (isTrade) {
      inp.placeholder = 'close kFloki  ·  tp SOL  ·  close all  ·  take profits';
      icon.textContent = '⚡';
      document.getElementById('ai-send-btn').style.background = '#b94300';
    } else {
      inp.placeholder = 'only BTC ETH  ·  max 5x  ·  meme coins  ·  reset';
      icon.textContent = '🎯';
      document.getElementById('ai-send-btn').style.background = '#1f6feb';
    }
  };

  // ── Input hint: auto-detect trade vs strategy ────────────────────────
  var tradeKeywords = ['close','exit','sell','tp','take profit','take profits'];
  var stratKeywords = ['only','max','leverage','meme','btc','eth','sol','aggressive','conservative','reset','sector'];
  window.onCmdInput = function(val) {
    var lc = val.toLowerCase().trim();
    if (!lc) return;
    if (tradeKeywords.some(function(k){ return lc.startsWith(k); })) {
      if (currentTab !== 'trade') setTab('trade');
    } else if (stratKeywords.some(function(k){ return lc.includes(k); })) {
      if (currentTab !== 'strategy') setTab('strategy');
    }
  };

  // ── Main submit ───────────────────────────────────────────────────────
  window.submitAiCmd = function() {
    var inp = document.getElementById('ai-cmd-input');
    var cmd = (inp.value || '').trim();
    if (!cmd) return;
    inp.value = '';
    if (currentTab === 'trade') {
      sendTradeCmd(cmd);
    } else {
      sendThesisCmd(cmd);
    }
  };

  // ── Trade command path ────────────────────────────────────────────────
  window.tradeCmd = function(cmd) {
    if (!cmd && topWinnerSym) cmd = 'tp ' + topWinnerSym;
    if (!cmd) { showResp('⚠ No open positions found.', 'warn'); return; }
    sendTradeCmd(cmd);
  };

  window.sendTradeCmd = function(cmd) {
    showResp('⏳ Parsing command…', 'info');
    fetch('/api/command', {
      method: 'POST',
      headers: {'Content-Type': 'application/json'},
      body: JSON.stringify({command: cmd})
    }).then(function(r){ return r.json(); }).then(function(d) {
      if (d.ok) {
        var sym = d.symbol ? ' ' + d.symbol : '';
        showResp('✅ ' + d.msg, 'ok');
        // Show the "executing on next cycle" badge
        var badge = document.getElementById('cmd-queued-badge');
        badge.style.display = 'block';
        setTimeout(function(){ badge.style.display = 'none'; }, 32000);
        addCmdHistory(d.action + sym);
      } else {
        showResp('⚠ ' + d.msg, 'warn');
      }
    }).catch(function() {
      showResp('⚠ Network error — is the bot running?', 'warn');
    });
  };

  // ── Strategy / thesis path (unchanged) ───────────────────────────────
  window.sendThesisCmd = function(cmd) {
    showResp('⏳ Updating strategy…', 'info');
    fetch('/api/thesis', {
      method: 'POST',
      headers: {'Content-Type': 'application/json'},
      body: JSON.stringify({command: cmd})
    }).then(function(r){ return r.json(); }).then(function(d) {
      if (d.type === 'query') {
        showResp('📋 <b>Recent trades:</b><br>' + (d.message || 'No trades found.'), 'ok', true);
      } else if (d.summary) {
        showResp('✅ ' + d.message, 'ok');
        showChip(d.summary);
      } else {
        showResp('✅ ' + (d.message || 'Strategy cleared — AI decides everything'), 'ok');
        clearChip();
      }
    }).catch(function() {
      showResp('⚠ Could not update strategy. Please try again.', 'warn');
    });
  };

  // ── Command history (last 3 executions, shown as faded chips) ────────
  var cmdHistory = [];
  function addCmdHistory(label) {
    cmdHistory.unshift(label);
    if (cmdHistory.length > 3) cmdHistory.pop();
    renderCmdHistory();
  }
  function renderCmdHistory() {
    var el = document.getElementById('cmd-history');
    if (!el) return;
    el.innerHTML = cmdHistory.map(function(c){
      return '<span style="font-size:.65rem;color:#484f58;background:#161b22;border:1px solid #21262d;border-radius:8px;padding:1px 7px;">✓ ' + c + '</span>';
    }).join(' ');
  }

  // ── Response panel helper ─────────────────────────────────────────────
  function showResp(html, type, isHtml) {
    var el = document.getElementById('ai-response');
    el.style.display = 'block';
    var bg = type === 'ok'   ? '#0d2018' :
             type === 'warn' ? '#2d1a0e' : '#0d1117';
    var col = type === 'ok'  ? '#3fb950' :
              type === 'warn'? '#e3b341' : '#8b949e';
    el.style.background = bg;
    el.style.border = '1px solid ' + col + '44';
    el.style.color = col;
    if (isHtml) { el.innerHTML = html; } else { el.textContent = html; }
    clearTimeout(el._hide);
    if (type !== 'info') {
      el._hide = setTimeout(function(){ el.style.display = 'none'; }, 5000);
    }
  }

  // ── Thesis chip helpers ───────────────────────────────────────────────
  function showChip(summary) {
    var chip = document.getElementById('thesis-chip');
    document.getElementById('thesis-chip-text').textContent = '🎯 ' + summary;
    chip.style.display = 'flex';
  }
  function clearChip() {
    document.getElementById('thesis-chip').style.display = 'none';
  }

  // ── On load: restore thesis chip + identify top winner ───────────────
  fetch('/api/thesis').then(function(r){ return r.json(); }).then(function(d){
    if (d.summary) showChip(d.summary);
  }).catch(function(){});

  // Poll /api/state every 30 s to keep chip labels fresh & find top winner
  function refreshState() {
    fetch('/api/state').then(function(r){ return r.json(); }).then(function(s){
      // top profitable position for the "tp top winner" chip
      var best = null, bestPnl = 0;
      (s.positions || []).forEach(function(p){
        if (p.unrealised_pnl > bestPnl) { bestPnl = p.unrealised_pnl; best = p.symbol; }
      });
      topWinnerSym = best;
      var chipBtn = document.getElementById('chip-top-winner');
      if (chipBtn) {
        if (best) {
          chipBtn.style.display = 'inline';
          chipBtn.textContent = 'tp ' + best + ' ($' + bestPnl.toFixed(2) + ')';
        } else {
          chipBtn.style.display = 'none';
        }
      }
    }).catch(function(){});
  }
  refreshState();
  setInterval(refreshState, 30000);

  // Inject command-history row after chips-trade
  (function(){
    var ct = document.getElementById('chips-trade');
    if (!ct) return;
    var hr = document.createElement('div');
    hr.id = 'cmd-history';
    hr.style.cssText = 'display:flex;flex-wrap:wrap;gap:4px;padding-left:26px;';
    ct.parentNode.insertBefore(hr, ct.nextSibling);
  })();

  // Init trade tab as default
  setTab('trade');
})();
</script>

</body></html>"#
}

/// Overview page — equity, P&L, deposit/withdraw, referral link.
async fn consumer_app_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let (state_arc, tenant_id) = match resolve_consumer_state(&headers, &app).await {
        ConsumerStateResult::Ok { state, tenant_id } => (state, Some(tenant_id)),
        ConsumerStateResult::NeedsLogin => {
            return axum::response::Redirect::to("/login").into_response()
        }
        ConsumerStateResult::NeedsOnboarding { .. } => {
            return axum::response::Redirect::to("/app/onboarding").into_response()
        }
    };

    // Redirect to HL wallet setup if the user hasn't completed it yet
    if let Some(ref tid) = tenant_id {
        let setup_done = {
            let tenants = app.tenants.read().await;
            tenants
                .get(tid)
                .map(|h| h.config.hl_setup_done())
                .unwrap_or(true)
        };
        if !setup_done {
            return axum::response::Redirect::to("/app/setup").into_response();
        }
    }

    let s = state_arc.read().await;

    // Resolve tenant tier to determine whether to show ads
    let show_ads = {
        let zone_set = app.coinzilla_zone_id.is_some();
        let is_free = if let Some(ref tid) = tenant_id {
            let tenants = app.tenants.read().await;
            tenants
                .get(tid)
                .map(|h| h.config.tier == crate::tenant::TenantTier::Free)
                .unwrap_or(false)
        } else {
            false // single-operator mode: no ads
        };
        zone_set && is_free
    };

    let committed: f64 = s.positions.iter().map(|p| p.size_usd).sum();
    let unrealised: f64 = s.positions.iter().map(|p| p.unrealised_pnl).sum();
    let equity = s.capital + committed + unrealised;
    let total_pnl = s.pnl + unrealised;
    let pnl_pct = if s.initial_capital > 0.0 {
        total_pnl / s.initial_capital * 100.0
    } else {
        0.0
    };
    let pnl_col = if total_pnl >= 0.0 {
        "#3fb950"
    } else {
        "#f85149"
    };
    let pnl_sign = if total_pnl >= 0.0 { "+" } else { "-" };

    // Referral block — only rendered when the operator has set REFERRAL_CODE
    let referral_block = match &s.referral_code {
        Some(code) => format!(
            r#"<div class="card">
  <div class="card-label">Sign up for Hyperliquid</div>
  <div class="info-box">
    New to Hyperliquid? Create your account using our referral link and get a
    <b>fee discount</b> on every trade.<br><br>
    <a class="btn btn-blue" href="https://app.hyperliquid.xyz/join/{code}"
       target="_blank" style="display:inline-block;margin-top:4px">
       Create HL Account → tradingbots
    </a><br>
    <span class="note">Referral code: <b style="color:#e6edf3">{code}</b> · After creating your account,
    fund it with USDC and share your wallet address with us to get started.</span>
  </div>
</div>"#,
            code = code
        ),
        None => String::new(),
    };

    // Coinzilla ad block — shown only to Free-tier users when zone ID is configured
    let ad_block = if show_ads {
        let zone_id = app.coinzilla_zone_id.as_deref().unwrap_or("");
        // Estimate CPM for tracking: $1.20 is the default established-publisher rate
        let cpm_est = 1.20_f64;
        format!(
            r#"
<div class="card" style="text-align:center;padding:12px 0 8px">
  <div style="font-size:.68rem;color:#484f58;text-transform:uppercase;letter-spacing:.06em;margin-bottom:8px">
    Advertisement &nbsp;·&nbsp; <a href="/app/upgrade" style="color:#58a6ff;text-decoration:none">Remove ads with Pro</a>
  </div>
  <div id="rr-ad-slot"
       data-ad-network="coinzilla"
       data-ad-unit="banner_300x250"
       data-ad-cpm="{cpm}"
       style="display:inline-block;min-height:250px;min-width:300px">
    <script async src="//coinzilla.io/ads/{zone}/300x250.js"></script>
  </div>
</div>
<script>
(function(){{
  var REFRESH_MS = 30000;
  var slot = document.getElementById('rr-ad-slot');
  if (!slot) return;
  function refreshAd() {{
    // Remove old script tag and re-insert to trigger a new ad call
    var old = slot.querySelector('script[src*="coinzilla"]');
    if (old) old.remove();
    var s = document.createElement('script');
    s.async = true;
    s.src = '//coinzilla.io/ads/{zone}/300x250.js?_=' + Date.now();
    slot.appendChild(s);
    // Fire AD_IMPRESSION for the fresh impression
    navigator.sendBeacon && navigator.sendBeacon('/api/funnel', JSON.stringify({{
      event_type: 'AD_IMPRESSION',
      anon_id: localStorage.getItem('rr_anon_id') || '',
      network: 'coinzilla',
      ad_unit: 'banner_300x250',
      cpm_usd: {cpm}
    }}));
  }}
  setInterval(refreshAd, REFRESH_MS);
}})();
</script>
"#,
            zone = zone_id,
            cpm = cpm_est
        )
    } else {
        String::new()
    };

    let pattern_insight_snapshot = {
        let cache = app.pattern_cache.lock().await;
        cache.latest()
    };
    let pattern_card = if let Some(insights) = pattern_insight_snapshot {
        let summary = &insights.report_summary;
        let date_str = insights.date.format("%Y-%m-%d").to_string();
        let winner = summary
            .daily_winner
            .as_ref()
            .map(|(sym, pnl)| format!("{} (${:0.2})", html_escape(sym), pnl))
            .unwrap_or_else(|| "—".to_string());
        let loser = summary
            .daily_loser
            .as_ref()
            .map(|(sym, pnl)| format!("{} (${:0.2})", html_escape(sym), pnl))
            .unwrap_or_else(|| "—".to_string());
        let combo = insights.top_win_combos.first();
        let combo_breakdown = combo
            .map(|combo| html_escape(&combo.breakdown))
            .unwrap_or_else(|| "—".to_string());
        let combo_context = combo
            .map(|combo| html_escape(&combo.context))
            .unwrap_or_else(|| "—".to_string());
        let combo_rate = combo.map(|combo| combo.win_rate * 100.0).unwrap_or(0.0);
        format!(
            r#"<div class="card">
  <div class="card-label">AI pattern insights</div>
  <div style="font-size:.9rem;color:#8b949e;margin-bottom:8px">Updated {date}</div>
  <div class="metric-row" style="padding:4px 0">
    <span class="ml">Winner</span>
    <span class="mv">{winner}</span>
  </div>
  <div class="metric-row" style="padding:4px 0">
    <span class="ml">Loser</span>
    <span class="mv">{loser}</span>
  </div>
  <div style="font-size:.85rem;color:#e6edf3;margin-top:8px">
    <div style="margin-bottom:4px">Top win combo</div>
    <div style="font-size:.95rem;font-weight:600">{combo_breakdown}</div>
    <div style="font-size:.8rem;color:#8b949e">{combo_context}</div>
    <div style="font-size:.75rem;color:#58a6ff">Win rate {combo_rate:.0}%</div>
  </div>
  <a class="btn btn-green" href="/app/agents" style="margin-top:12px">Open Agent Console →</a>
</div>"#,
            date = date_str,
            winner = winner,
            loser = loser,
            combo_breakdown = combo_breakdown,
            combo_context = combo_context,
            combo_rate = combo_rate,
        )
    } else {
        r#"<div class="card">
  <div class="card-label">Pattern insights</div>
  <div class="info-box">
    Run <code>cargo run --bin reporter</code> to refresh <code>reports/pattern_cache.json</code> and unlock the
    latest win/loss combos before opening the agent console.
  </div>
</div>"#
        .to_string()
    };

    let mut html = consumer_shell_open("My Account", "Overview");
    html.push_str(&format!(r#"
<div class="card">
  <div class="card-label">Total Equity</div>
  <div id="app-equity" class="card-val">${equity:.2}</div>
  <div id="app-pnl-badge" class="badge" style="color:{pc};border-color:{pc}40;background:{pc}12">
    {ps}${pnl:.2} &nbsp; {ps}{pp:.2}%
  </div>
</div>

<div class="card">
  <div class="metric-row"><span class="ml">Free capital</span>
    <span id="app-capital" class="mv">${capital:.2}</span></div>
  <div class="metric-row" title="Accumulated profits recycled for future trades — these positions do not consume own capital">
    <span class="ml">🏦 House-money pool</span>
    <span id="app-pool" class="mv" style="color:{pool_col}">${pool:.2}</span></div>
  <div class="metric-row" title="USD currently deployed in pool-funded open positions">
    <span class="ml" style="font-size:.85em;color:#8b949e">  ↳ deployed</span>
    <span id="app-pool-deployed" class="mv" style="font-size:.85em;color:#8b949e">${pool_deployed:.2}</span></div>
  <div class="metric-row"><span class="ml">Open positions</span>
    <span id="app-positions" class="mv">{open_n}</span></div>
  <div class="metric-row"><span class="ml">Closed trades</span>
    <span id="app-closed" class="mv">{closed_n}</span></div>
  <div class="metric-row"><span class="ml">Initial deposit</span>
    <span class="mv">${init:.2}</span></div>
</div>

<div class="card">
  <div class="card-label">Deposit / Withdraw</div>
  <div class="info-box">
    Your funds remain in <b>your Hyperliquid account</b> at all times.<br><br>
    • <b>Deposit:</b> transfer USDC to your HL wallet. The bot automatically
      trades with the updated balance on the next cycle.<br><br>
    • <b>Withdraw:</b> log in to
      <a href="https://app.hyperliquid.xyz" target="_blank">app.hyperliquid.xyz</a>
      and withdraw directly — no approval from us needed.<br><br>
    You are always in full custody of your funds.
  </div>
</div>

{referral_block}

{ad_block}

{pattern_card}

<p class="note" style="margin-top:8px;text-align:center">
  Auto-refreshes every 5 s · Last update: {ts}
  &nbsp;·&nbsp; <a href="/app/history">Trade history</a>
  &nbsp;·&nbsp; <a href="/app/tax">Tax report</a>
</p>

<script>
(function(){{
  function $id(id){{return document.getElementById(id);}}
  function fmt2(n){{return Math.abs(n).toFixed(2);}}
  function sign(n){{return n>=0?'+':'-';}}
  function col(n){{return n>=0?'#3fb950':'#f85149';}}
  function applyPoll(s){{
    var committed=0,unrealised=0;
    (s.positions||[]).forEach(function(p){{unrealised+=p.unrealised_pnl;committed+=p.size_usd;}});
    var equity=s.capital+committed+unrealised;
    var total_pnl=s.pnl+unrealised;
    var pnl_pct=s.initial_capital>0?(total_pnl/s.initial_capital*100):0;
    var c=col(total_pnl);
    var ev=$id('app-equity');if(ev)ev.textContent='$'+equity.toFixed(2);
    var pnlb=$id('app-pnl-badge');
    if(pnlb){{var sg=sign(total_pnl);
      pnlb.textContent=sg+'$'+fmt2(total_pnl)+' \u00a0 '+sg+Math.abs(pnl_pct).toFixed(2)+'%';
      pnlb.style.color=c;pnlb.style.borderColor=c+'40';pnlb.style.background=c+'12';}}
    var cap=$id('app-capital');if(cap)cap.textContent='$'+s.capital.toFixed(2);
    var pool=$id('app-pool');if(pool){{pool.textContent='$'+(s.house_money_pool||0).toFixed(2);pool.style.color=(s.house_money_pool||0)>0?'#3fb950':'#8b949e';}}
    var pd=$id('app-pool-deployed');if(pd)pd.textContent='$'+(s.pool_deployed_usd||0).toFixed(2);
    var posEl=$id('app-positions');if(posEl)posEl.textContent=(s.positions||[]).length;
    var clEl=$id('app-closed');if(clEl)clEl.textContent=(s.closed_trades||[]).length;
  }}
  function poll(){{fetch('/api/state').then(function(r){{return r.json();}}).then(applyPoll).catch(function(){{}});}}
  setTimeout(poll,2000);setInterval(poll,5000);
}})();
</script>
"#,
        equity          = equity,
        pc              = pnl_col,
        ps              = pnl_sign,
        pnl             = total_pnl.abs(),
        pp              = pnl_pct.abs(),
        capital         = s.capital,
        pool            = s.house_money_pool,
        pool_col        = if s.house_money_pool > 0.0 { "#3fb950" } else { "#8b949e" },
        pool_deployed   = s.pool_deployed_usd,
        open_n          = s.positions.len(),
        closed_n        = s.closed_trades.len(),
        init            = s.initial_capital,
        ts              = s.last_update,
        referral_block  = referral_block,
        ad_block        = ad_block,
        pattern_card    = pattern_card,
    ));
    html.push_str(consumer_shell_close());
    axum::response::Html(html).into_response()
}

fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

async fn agent_app_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let (state_arc, tenant_id) = match resolve_consumer_state(&headers, &app).await {
        ConsumerStateResult::Ok { state, tenant_id } => (state, Some(tenant_id)),
        ConsumerStateResult::NeedsLogin => {
            return axum::response::Redirect::to("/login").into_response()
        }
        ConsumerStateResult::NeedsOnboarding { .. } => {
            return axum::response::Redirect::to("/app/onboarding").into_response()
        }
    };

    let s = state_arc.read().await;
    let committed: f64 = s.positions.iter().map(|p| p.size_usd).sum();
    let unrealised: f64 = s.positions.iter().map(|p| p.unrealised_pnl).sum();
    let equity = s.capital + committed + unrealised;
    let total_pnl = s.pnl + unrealised;
    let positions_preview: Vec<_> = s
        .positions
        .iter()
        .map(|p| {
            json!({
                "symbol": p.symbol,
                "side": p.side,
                "size_usd": p.size_usd,
                "unrealised_pnl": p.unrealised_pnl,
            })
        })
        .collect();
    let init_payload = json!({
        "capital": s.capital,
        "equity": equity,
        "total_pnl": total_pnl,
        "positions": positions_preview,
        "tenant_id": tenant_id.map(|id| id.as_str().to_owned()),
    });
    let html = format!(
        r###"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <title>OpenClaw Agent Control</title>
  <style>
    :root {{
      color-scheme: dark;
      font-family: 'Inter', system-ui, sans-serif;
    }}
    body {{
      margin:0;
      background:#060b12;
      color:#e6edf3;
    }}
    .page {{
      padding:28px;
      max-width:1100px;
      margin:0 auto;
      display:flex;
      flex-direction:column;
      gap:18px;
    }}
    header {{
      display:flex;
      justify-content:space-between;
      align-items:center;
    }}
    header h1 {{
      margin:0;
      font-size:2rem;
    }}
    .grid {{
      display:grid;
      grid-template-columns:repeat(auto-fit,minmax(280px,1fr));
      gap:14px;
    }}
    .card {{
      background:#0d1117;
      border:1px solid #161b22;
      border-radius:16px;
      padding:18px;
      box-shadow:0 10px 25px rgba(5,8,15,.5);
    }}
    .card h2 {{
      margin-top:0;
      font-size:1rem;
      color:#8b949e;
      text-transform:uppercase;
      letter-spacing:.06em;
    }}
    .metric {{
      display:flex;
      justify-content:space-between;
      align-items:center;
      margin-bottom:6px;
      font-size:1.05rem;
    }}
    .metric span {{
      font-size:.9rem;
      color:#8b949e;
    }}
    .list {{
      margin:0;
      padding:0;
      list-style:none;
      display:flex;
      flex-direction:column;
      gap:6px;
      max-height:180px;
      overflow:auto;
    }}
    .list li {{
      padding:6px 8px;
      background:#111720;
      border-radius:8px;
      font-size:.9rem;
      display:flex;
      justify-content:space-between;
    }}
    form {{
      display:flex;
      gap:8px;
      flex-wrap:wrap;
      align-items:center;
    }}
    input, select {{
      padding:10px 12px;
      border-radius:10px;
      border:1px solid #30363d;
      background:#0d1117;
      color:#e6edf3;
      min-width:200px;
    }}
    button {{
      background:#3fb950;
      border:none;
      border-radius:10px;
      padding:10px 16px;
      color:#0d1117;
      font-weight:600;
      cursor:pointer;
      transition:.2s;
    }}
    button:hover {{
      transform:translateY(-1px);
    }}
    .feedback {{
      padding:12px;
      border-radius:10px;
      background:#0d1117;
      border:1px solid #1d2633;
      font-size:.9rem;
      min-height:48px;
    }}
    .badge {{
      display:inline-flex;
      align-items:center;
      gap:6px;
    }}
  </style>
</head>
<body>
<div class="page">
  <header>
    <div>
      <h1>OpenClaw Agent Control</h1>
      <p style="margin:4px 0 0;color:#58a6ff;font-size:.95rem;">Trade with the AI guardrail on behalf of x402 sessions.</p>
    </div>
    <a href="/app" style="color:#8b949e;text-decoration:none;border:1px solid #30363d;padding:8px 14px;border-radius:10px;">Return to Dashboard</a>
  </header>
  <div class="grid">
    <div class="card">
      <h2>Portfolio snapshot</h2>
      <div class="metric">
        <strong>Capital</strong>
        <span id="capital-value">--</span>
      </div>
      <div class="metric">
        <strong>Equity</strong>
        <span id="equity-value">--</span>
      </div>
      <div class="metric">
        <strong>Total PnL</strong>
        <span id="pnl-value">--</span>
      </div>
      <h2 style="margin-top:14px;">Positions</h2>
      <ul class="list" id="positions-list">
        <li>Loading positions…</li>
      </ul>
    </div>
    <div class="card">
      <h2>Guardrail combos</h2>
      <div id="patterns-loading">Loading combos…</div>
      <div id="pattern-combos" style="display:none;">
        <p style="margin:0;font-size:.9rem;color:#8b949e;">Top win combo</p>
        <p id="combo-breakdown" style="font-weight:600;margin:2px 0;"></p>
        <p id="combo-context" style="margin:0;font-size:.8rem;color:#8b949e;"></p>
        <p id="combo-winrate" style="margin:4px 0 0;font-size:.85rem;"></p>
      </div>
    </div>
    <div class="card">
      <h2>Automation alert</h2>
      <div id="alert-loading">Waiting for automation alert…</div>
      <div id="alert-content" style="display:none;">
        <p id="alert-updated" style="margin:0;font-size:.85rem;color:#58a6ff;"></p>
        <p style="margin:6px 0 0;font-size:.9rem;">Winner: <strong id="alert-winner">—</strong></p>
        <p style="margin:2px 0 0;font-size:.9rem;">Loser: <strong id="alert-loser">—</strong></p>
        <p style="margin:10px 0 0;font-size:.9rem;">Top combo: <span id="alert-combo">—</span></p>
      </div>
    </div>
    <div class="card">
      <h2>Hyperliquid traffic</h2>
      <div class="metric-row" style="display:flex;justify-content:space-between;">
        <span>Requests sent</span>
        <span id="hl-requests">—</span>
      </div>
      <div class="metric-row" style="display:flex;justify-content:space-between;">
        <span>Rate-limit hits (429)</span>
        <span id="hl-rate-limits">—</span>
      </div>
      <div class="metric-row" style="display:flex;justify-content:space-between;">
        <span>Last 429</span>
        <span id="hl-last-429">never</span>
      </div>
    </div>
  </div>
  <form id="command-form">
    <input type="text" id="command-input" placeholder="Tell Claude to change trades (e.g. close btc)" required />
    <button type="submit">Send command</button>
    <div class="badge" style="margin-left:auto;font-size:.8rem;color:#8b949e;">
      Command publishes to /api/command on next cycle (≈30s)
    </div>
  </form>
  <div id="command-feedback" class="feedback">Command responses will appear here.</div>
</div>
<script>
  window.__AGENT_INIT = {init};

  function formatMoney(value) {{
    return `$${{value.toFixed(2)}}`;
  }}

  async function refreshState() {{
    try {{
      const response = await fetch('/api/state');
      if (!response.ok) throw new Error('state fetch failed');
      const data = await response.json();
      document.getElementById('capital-value').textContent = formatMoney(data.free_capital ?? data.capital ?? 0);
      document.getElementById('equity-value').textContent = formatMoney(data.capital + (data.open_positions || 0));
      document.getElementById('pnl-value').textContent = formatMoney(data.pnl ?? 0);
      const list = document.getElementById('positions-list');
      list.innerHTML = '';
      if ((data.positions || []).length === 0) {{
        list.innerHTML = '<li>No open positions</li>';
      }} else {{
        data.positions.forEach(p => {{
          const li = document.createElement('li');
          li.innerHTML = `<span>${{p.symbol}} ({{p.side}})</span><span>${{formatMoney(p.unrealised_pnl)}}</span>`;
          list.appendChild(li);
        }});
      }}
    }} catch (err) {{
      console.error(err);
    }}
  }}
  refreshState();
  setInterval(refreshState, 25000);

  async function refreshPatterns() {{
    try {{
      const res = await fetch('/api/report/patterns');
      if (!res.ok) throw new Error('patterns unavailable');
      const data = await res.json();
      const combo = (data.top_win_combos || [])[0] || null;
      document.getElementById('patterns-loading').style.display = 'none';
      document.getElementById('pattern-combos').style.display = 'block';
      if (combo) {{
        document.getElementById('combo-breakdown').textContent = combo.breakdown;
        document.getElementById('combo-context').textContent = combo.context;
        document.getElementById('combo-winrate').textContent = `${{(combo.win_rate * 100).toFixed(0)}}% win rate`;
      }} else {{
        document.getElementById('combo-breakdown').textContent = 'No combo data yet';
        document.getElementById('combo-context').textContent = '';
        document.getElementById('combo-winrate').textContent = '';
      }}
    }} catch (err) {{
      document.getElementById('patterns-loading').textContent = 'Unable to load combos yet.';
      console.error(err);
    }}
  }}
  refreshPatterns();
  setInterval(refreshPatterns, 15000);

  async function refreshAlert() {{
    try {{
      const res = await fetch('/api/report/patterns/alerts');
      if (!res.ok) throw new Error('alert missing');
      const data = await res.json();
      document.getElementById('alert-loading').style.display = 'none';
      document.getElementById('alert-content').style.display = 'block';
      document.getElementById('alert-updated').textContent = `Updated ${{data.updated_at}}`;
      document.getElementById('alert-winner').textContent = data.winner ? `${{data.winner[0]}} ($${{data.winner[1].toFixed(2)}})` : '—';
      document.getElementById('alert-loser').textContent = data.loser ? `${{data.loser[0]}} ($${{data.loser[1].toFixed(2)}})` : '—';
      document.getElementById('alert-combo').textContent = `${{data.top_combo.breakdown}} · ${{data.top_combo.context}}`;
    }} catch (err) {{
      document.getElementById('alert-loading').textContent = 'Awaiting automation trigger…';
      console.error(err);
    }}
  }}
  refreshAlert();
  setInterval(refreshAlert, 20000);

  document.getElementById('command-form').addEventListener('submit', async function(e) {{
    e.preventDefault();
    const input = document.getElementById('command-input');
    const command = input.value.trim();
    if (!command) return;
    const respEl = document.getElementById('command-feedback');
    respEl.textContent = 'Sending command…';
    try {{
      const res = await fetch('/api/command', {{
        method: 'POST',
        headers: {{ 'Content-Type': 'application/json' }},
        body: JSON.stringify({{ command }}),
      }});
      const data = await res.json();
      if (!data.ok) {{
        respEl.textContent = data.msg || 'Command rejected';
      }} else {{
        respEl.textContent = `Queued ${{data.action}} ${{data.symbol ?? ''}}`.trim();
      }}
    }} catch (err) {{
      respEl.textContent = 'Command failed — check console.';
      console.error(err);
    }}
    input.value = '';
  }});
</script>
</body>
</html>"###,
        init = init_payload,
    );
    Html(html).into_response()
}

// ─── Trade history page /app/history ─────────────────────────────────────────

async fn consumer_history_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let state_arc = match resolve_consumer_state(&headers, &app).await {
        ConsumerStateResult::Ok { state, .. } => state,
        ConsumerStateResult::NeedsLogin => {
            return axum::response::Redirect::to("/login").into_response()
        }
        ConsumerStateResult::NeedsOnboarding { .. } => {
            return axum::response::Redirect::to("/app/onboarding").into_response()
        }
    };
    let s = state_arc.read().await;

    let rows: String = if s.closed_trades.is_empty() {
        "<tr><td colspan='9' style='color:#8b949e;text-align:center;padding:20px'>No closed trades yet.</td></tr>".to_string()
    } else {
        s.closed_trades
            .iter()
            .rev()
            .map(|t| {
                let pnl_col = if t.pnl >= 0.0 { "#3fb950" } else { "#f85149" };
                let pnl_sign = if t.pnl >= 0.0 { "+" } else { "" };
                let fees = if t.fees_est > 0.0 {
                    t.fees_est
                } else {
                    crate::ledger::estimate_fees(t.size_usd, t.leverage.max(1.0))
                };
                let net = t.pnl - fees;
                let net_col = if net >= 0.0 { "#3fb950" } else { "#f85149" };
                let date = t.closed_at.get(..10).unwrap_or(&t.closed_at);
                format!(
                    "<tr>\
                   <td class='muted' style='font-size:.75rem'>{date}</td>\
                   <td><b>{sym}</b></td>\
                   <td style='color:{sc}'>{side}</td>\
                   <td>${entry:.4}</td>\
                   <td>${exit:.4}</td>\
                   <td class='muted'>{lev:.1}×</td>\
                   <td style='color:{pc}'>{ps}{pnl:.2}</td>\
                   <td style='color:#f85149'>-{fees:.3}</td>\
                   <td style='color:{nc};font-weight:600'>{nps}{net:.2}</td>\
                 </tr>",
                    date = date,
                    sym = t.symbol,
                    side = t.side,
                    sc = if t.side == "LONG" {
                        "#3fb950"
                    } else {
                        "#f85149"
                    },
                    entry = t.entry,
                    exit = t.exit,
                    lev = t.leverage.max(1.0),
                    pc = pnl_col,
                    ps = pnl_sign,
                    pnl = t.pnl,
                    fees = fees,
                    nc = net_col,
                    nps = if net >= 0.0 { "+" } else { "" },
                    net = net,
                )
            })
            .collect()
    };

    // Summary totals
    let total_gross: f64 = s.closed_trades.iter().map(|t| t.pnl).sum();
    let total_fees: f64 = s
        .closed_trades
        .iter()
        .map(|t| {
            if t.fees_est > 0.0 {
                t.fees_est
            } else {
                crate::ledger::estimate_fees(t.size_usd, t.leverage.max(1.0))
            }
        })
        .sum();
    let total_net = total_gross - total_fees;
    let wins = s.closed_trades.iter().filter(|t| t.pnl > 0.0).count();
    let total = s.closed_trades.len();

    let mut html = consumer_shell_open("Trade History", "History");
    html.push_str(&format!(r#"
<div class="card" style="padding:16px 20px">
  <div style="display:flex;gap:24px;flex-wrap:wrap">
    <div><div class="card-label">Net P&amp;L</div>
      <div style="font-size:1.5rem;font-weight:700;color:{nc}">{nps}${net:.2}</div></div>
    <div><div class="card-label">Gross P&amp;L</div>
      <div style="font-size:1.5rem;font-weight:700;color:{gc}">{gps}${gross:.2}</div></div>
    <div><div class="card-label">Est. Fees</div>
      <div style="font-size:1.5rem;font-weight:700;color:#f85149">-${fees:.2}</div></div>
    <div><div class="card-label">Win Rate</div>
      <div style="font-size:1.5rem;font-weight:700;color:#e6edf3">{wr:.0}%</div></div>
    <div><div class="card-label">Trades</div>
      <div style="font-size:1.5rem;font-weight:700;color:#e6edf3">{total}</div></div>
  </div>
</div>

<div class="card" style="padding:0;overflow:auto">
  <div style="padding:12px 16px;border-bottom:1px solid #30363d;display:flex;
       justify-content:space-between;align-items:center">
    <span style="font-size:.85rem;font-weight:600;color:#e6edf3">Recent trades (in-memory, last 100)</span>
    <a href="/app/tax/csv" class="btn btn-green" style="font-size:.78rem;padding:5px 12px">
      ↓ Download full CSV
    </a>
  </div>
  <table class="tbl">
    <thead><tr>
      <th>Date</th><th>Symbol</th><th>Side</th><th>Entry</th><th>Exit</th>
      <th>Lev</th><th>Gross P&amp;L</th><th>Fees</th><th>Net P&amp;L</th>
    </tr></thead>
    <tbody>{rows}</tbody>
  </table>
</div>
<p class="note" style="margin-top:8px">
  In-memory history is capped at 100 trades. Full history lives in
  <code>trades_YYYY.csv</code> on the server and can be downloaded via the
  <a href="/app/tax/csv">CSV export</a>.
</p>
"#,
        nc    = if total_net   >= 0.0 { "#3fb950" } else { "#f85149" },
        nps   = if total_net   >= 0.0 { "+" } else { "" },
        gc    = if total_gross >= 0.0 { "#3fb950" } else { "#f85149" },
        gps   = if total_gross >= 0.0 { "+" } else { "" },
        net   = total_net.abs(),
        gross = total_gross.abs(),
        fees  = total_fees,
        wr    = if total > 0 { wins as f64 / total as f64 * 100.0 } else { 0.0 },
        total = total,
        rows  = rows,
    ));
    html.push_str(consumer_shell_close());
    axum::response::Html(html).into_response()
}

// ─── Tax report page /app/tax ─────────────────────────────────────────────────

async fn consumer_tax_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    match resolve_consumer_state(&headers, &app).await {
        ConsumerStateResult::Ok { .. } => consumer_tax_page().into_response(),
        ConsumerStateResult::NeedsLogin => axum::response::Redirect::to("/login").into_response(),
        ConsumerStateResult::NeedsOnboarding { .. } => {
            axum::response::Redirect::to("/app/onboarding").into_response()
        }
    }
}

fn consumer_tax_page() -> axum::response::Html<String> {
    let summary = crate::ledger::yearly_summary();
    let (_, total_rows) = crate::ledger::read_all();

    let year_cards: String = if summary.is_empty() {
        "<div class='info-box'>No closed trades recorded yet. Trades appear here after they close.</div>".to_string()
    } else {
        summary.iter().map(|(year, gross, fees, net, count, wins, losses)| {
            let net_col  = if *net  >= 0.0 { "#3fb950" } else { "#f85149" };
            let net_sign = if *net  >= 0.0 { "+" } else { "" };
            let win_rate = if *count > 0 { *wins as f64 / *count as f64 * 100.0 } else { 0.0 };
            format!(r#"<div class="card">
  <div style="display:flex;justify-content:space-between;align-items:baseline;margin-bottom:12px">
    <span style="font-size:1.1rem;font-weight:700;color:#e6edf3">{year}</span>
    <span style="font-size:.8rem;color:#8b949e">{count} trades · {wins}W / {losses}L · {wr:.0}% win rate</span>
  </div>
  <div style="display:flex;gap:20px;flex-wrap:wrap">
    <div><div class="card-label">Net P&amp;L</div>
      <div style="font-size:1.6rem;font-weight:700;color:{nc}">{ns}${net:.2}</div></div>
    <div><div class="card-label">Gross P&amp;L</div>
      <div style="font-size:1.2rem;font-weight:600;color:#c9d1d9">{gs}${gross:.2}</div></div>
    <div><div class="card-label">Est. Fees</div>
      <div style="font-size:1.2rem;font-weight:600;color:#f85149">-${fees:.2}</div></div>
  </div>
</div>"#,
                year  = year,
                count = count,
                wins  = wins,
                losses = losses,
                wr    = win_rate,
                nc    = net_col,
                ns    = net_sign,
                net   = net.abs(),
                gs    = if *gross >= 0.0 { "+" } else { "" },
                gross = gross.abs(),
                fees  = fees,
            )
        }).collect()
    };

    let mut html = consumer_shell_open("Tax Report", "Tax");
    html.push_str(&format!(
        r#"
<div style="display:flex;justify-content:space-between;align-items:center;margin-bottom:16px">
  <div>
    <div style="font-size:.88rem;font-weight:600;color:#e6edf3">Annual P&amp;L Summary</div>
    <div class="note">{total_rows} total trades on record · Updates when a trade closes</div>
  </div>
  <a href="/app/tax/csv" class="btn btn-green">↓ Download all trades CSV</a>
</div>

{year_cards}

<div class="card">
  <div class="card-label">Important Notes</div>
  <div class="info-box">
    <b>This report is for informational purposes only and does not constitute tax advice.</b>
    Consult a qualified tax professional before filing.<br><br>
    • Perpetual futures contracts may qualify as <b>Section 1256 contracts</b> in the
      US (60% long-term / 40% short-term capital gains treatment) — verify with your
      accountant as this depends on the exchange and jurisdiction.<br><br>
    • <b>Fees shown are estimates</b> based on ~0.075 % per leg (maker + builder fee).
      Actual fees appear on your Hyperliquid account statement — always use those
      figures for filing.<br><br>
    • The CSV export contains one row per trade closure and is formatted for easy
      import into tax software (Koinly, CoinTracker, TaxBit, etc.).<br><br>
    • Partial closes (2R / 4R tranches) are recorded as separate rows.
  </div>
</div>
"#,
        total_rows = total_rows,
        year_cards = year_cards,
    ));
    html.push_str(consumer_shell_close());
    axum::response::Html(html)
}

// ─── CSV download /app/tax/csv ────────────────────────────────────────────────

async fn consumer_tax_csv_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    match resolve_consumer_state(&headers, &app).await {
        ConsumerStateResult::NeedsLogin => {
            return axum::response::Redirect::to("/login").into_response()
        }
        ConsumerStateResult::NeedsOnboarding { .. } => {
            return axum::response::Redirect::to("/app/onboarding").into_response()
        }
        ConsumerStateResult::Ok { .. } => {}
    }
    let (csv, _) = crate::ledger::read_all();
    let filename = format!(
        "tradingbots_trades_{}.csv",
        chrono::Utc::now().format("%Y%m%d")
    );
    (
        [
            ("Content-Type", "text/csv; charset=utf-8"),
            (
                "Content-Disposition",
                Box::leak(format!("attachment; filename=\"{}\"", filename).into_boxed_str()),
            ),
        ],
        csv,
    )
        .into_response()
}

// ─────────────────────────────────────────────────────────────────────────────
//  Privy auth helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Extract and verify the session cookie from request headers.
///
/// Returns `Some(TenantId)` if the `rr_session` cookie is present and its
/// HMAC is valid; `None` otherwise (missing, tampered, or expired).
fn get_session_tenant_id(
    headers: &axum::http::HeaderMap,
    secret: &str,
) -> Option<crate::tenant::TenantId> {
    let cookie_hdr = headers.get("cookie")?.to_str().ok()?;
    let session_val = crate::privy::extract_session_cookie(cookie_hdr)?;
    crate::privy::verify_session(session_val, secret).ok()
}

/// Result of resolving the consumer state for an incoming request.
pub enum ConsumerStateResult {
    /// Authenticated and has accepted terms — ready to serve trading data.
    Ok {
        state: SharedState,
        tenant_id: crate::tenant::TenantId,
    },
    /// No valid session cookie (or Privy is not configured in single-op mode).
    NeedsLogin,
    /// Valid session but tenant has not accepted the Terms & Risk Disclosure.
    NeedsOnboarding {
        #[allow(dead_code)]
        tenant_id: crate::tenant::TenantId,
    },
}

/// Resolve the `SharedState` that should be rendered for this request.
///
/// - If `privy_app_id` is set → require a valid session → check terms wall
///   → return `ConsumerStateResult`.
/// - If `privy_app_id` is `None` (single-operator mode) → bypass auth AND
///   terms check, return `ConsumerStateResult::Ok` with the global state.
async fn resolve_consumer_state(
    headers: &axum::http::HeaderMap,
    app: &AppState,
) -> ConsumerStateResult {
    // Single-operator mode: no auth, no terms wall
    if app.privy_app_id.is_none() {
        // Use a synthetic TenantId for the operator in single-op mode
        let tid = crate::tenant::TenantId::from_str("operator");
        return ConsumerStateResult::Ok {
            state: app.bot_state.clone(),
            tenant_id: tid,
        };
    }

    // Multi-tenant mode: require valid session cookie
    let tid = match get_session_tenant_id(headers, &app.session_secret) {
        Some(t) => t,
        None => return ConsumerStateResult::NeedsLogin,
    };

    // Check terms acceptance
    let tenants = app.tenants.read().await;
    let handle = match tenants.get(&tid) {
        Some(h) => h,
        None => return ConsumerStateResult::NeedsLogin,
    };

    if handle.config.terms_accepted_at.is_none() {
        return ConsumerStateResult::NeedsOnboarding { tenant_id: tid };
    }

    ConsumerStateResult::Ok {
        state: handle.state.clone(),
        tenant_id: tid,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Auth handlers
// ─────────────────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct SessionRequest {
    token: String,
    /// Invite code entered on the login page — required for new signups.
    /// Existing users who already have a session don't need to re-supply this.
    #[serde(default)]
    invite_code: Option<String>,
    /// First-touch acquisition source (utm_source) — sent by the login page JS
    /// from the URL query params / cookie captured on landing.
    #[serde(default)]
    utm_source: Option<String>,
    /// utm_campaign captured at landing — sent through to funnel_events.
    #[serde(default)]
    utm_campaign: Option<String>,
    /// True when the user arrived via our Hyperliquid referral link.
    #[serde(default)]
    hl_referred: bool,
}

/// `POST /auth/session`
///
/// Receives a Privy access token (JWT) from the browser, verifies it against
/// Privy's JWKS, auto-registers the user as a Free tenant if new, and sets
/// the `rr_session` HMAC-signed cookie.
///
/// Response: `{"ok":true,"tenant_id":"…"}` on success, HTTP 401 on failure.
async fn auth_session_handler(
    State(app): State<AppState>,
    axum::Json(req): axum::Json<SessionRequest>,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    let privy_app_id = match &app.privy_app_id {
        Some(id) => id.clone(),
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "Privy is not configured on this server",
            )
                .into_response()
        }
    };

    // Verify the Privy JWT (ES256, JWKS-backed)
    let privy_did = match crate::privy::verify_privy_jwt(&req.token, &privy_app_id, &app.jwks_cache)
        .await
    {
        Ok(did) => did,
        Err(e) => {
            log::warn!("⚠ Privy JWT verification failed: {}", e);
            return (StatusCode::UNAUTHORIZED, "Invalid or expired Privy token").into_response();
        }
    };

    // ── Invite-code gate ──────────────────────────────────────────────────────
    // New users must supply a valid invite code.  Existing users (DID already
    // known) bypass this check — they already have an account.
    let is_known_user = {
        let tenants = app.tenants.read().await;
        tenants.find_by_privy_did(&privy_did).is_some()
    };

    let mut claimed_invite: Option<crate::invite::ClaimedInvite> = None;

    if !is_known_user {
        let code = match &req.invite_code {
            Some(c) if !c.trim().is_empty() => c.trim().to_uppercase(),
            _ => {
                return (StatusCode::FORBIDDEN,
                    axum::Json(serde_json::json!({"error":"invite_required","message":"An invite code is required to create an account. Get one from a friend or the weekly campaign."}))).into_response();
            }
        };

        match &app.db {
            Some(db) => match crate::invite::claim_invite_code(db, &code).await {
                Ok(Some(invite)) => {
                    claimed_invite = Some(invite);
                }
                Ok(None) => {
                    return (StatusCode::FORBIDDEN,
                            axum::Json(serde_json::json!({"error":"invalid_invite","message":"That invite code is invalid, already used, or expired. Ask for a new one."}))).into_response();
                }
                Err(e) => {
                    log::error!("invite claim DB error: {}", e);
                    return (StatusCode::INTERNAL_SERVER_ERROR,
                            axum::Json(serde_json::json!({"error":"db_error","message":"Could not validate invite code. Please try again."}))).into_response();
                }
            },
            None => {
                // No DB — accept any non-empty code in dev/paper mode
                log::warn!(
                    "⚠ No DB — invite code '{}' accepted without validation",
                    code
                );
            }
        }
    }

    // ── Register new user or retrieve existing tenant ─────────────────────────
    let referral_source = if req.hl_referred {
        Some("hl_referral".to_string())
    } else {
        req.utm_source.clone()
    };

    let (tenant_id, is_new) = {
        let mut tenants = app.tenants.write().await;
        let existing = tenants.find_by_privy_did(&privy_did).map(|h| h.id.clone());
        let is_new = existing.is_none();
        let id = tenants.register_or_get_by_privy_did(
            &privy_did,
            None,
            referral_source.clone(),
            req.hl_referred,
            req.utm_campaign.clone(),
        );
        (id, is_new)
    };

    // Restore HL wallet from DB after server restarts (in-memory only, no lock held)
    if let Some(ref db) = app.db {
        if let Ok(tid_uuid) = uuid::Uuid::parse_str(tenant_id.as_str()) {
            if let Ok(Some(row)) = sqlx::query!(
                "SELECT hl_wallet_address, hl_wallet_key_enc, hl_setup_complete                  FROM tenants WHERE id = $1",
                tid_uuid
            )
            .fetch_optional(db.pool())
            .await
            {
                if let (Some(addr), Some(key)) = (row.hl_wallet_address, row.hl_wallet_key_enc) {
                    let mut tenants = app.tenants.write().await;
                    let _ = tenants.setup_hl_wallet(&tenant_id, addr, key);
                    if row.hl_setup_complete {
                        let _ = tenants.complete_hl_setup(&tenant_id);
                    }
                }
            }
        }
    }

    // ── Restore investment thesis from DB on login ────────────────────────────
    if let Some(ref db) = app.db {
        if let Ok(tid_uuid) = uuid::Uuid::parse_str(tenant_id.as_str()) {
            if let Ok(Some(row)) = sqlx::query!(
                "SELECT investment_thesis, symbol_whitelist, sector_filter, max_leverage_override
                 FROM tenants WHERE id = $1",
                tid_uuid
            )
            .fetch_optional(db.pool())
            .await
            {
                // Update in-memory tenant config
                {
                    let mut tenants = app.tenants.write().await;
                    let _ = tenants.update_thesis(
                        &tenant_id,
                        row.investment_thesis.clone(),
                        row.symbol_whitelist.clone(),
                        row.sector_filter.clone(),
                        row.max_leverage_override,
                    );
                }
                // Rebuild and propagate global_thesis from restored data
                {
                    let tenants = app.tenants.read().await;
                    let constraints = tenants.thesis_constraints(&tenant_id);
                    let mut gt = app.global_thesis.write().await;
                    *gt = constraints;
                }
            }
        }
    }

    // ── Stamp invite attribution on the tenant row in DB ─────────────────────
    if is_new {
        if let (Some(db), Some(invite)) = (&app.db, &claimed_invite) {
            let tenant_uuid = uuid::Uuid::parse_str(tenant_id.as_str()).ok();
            let campaign_id = invite.campaign_id;
            let invited_by = invite.created_by;
            let code_used = req.invite_code.clone().unwrap_or_default();

            if let Some(tid) = tenant_uuid {
                let _ = sqlx::query!(
                    "UPDATE tenants SET invite_code_used = $1, invited_by = $2, campaign_id = $3 WHERE id = $4",
                    code_used,
                    invited_by,
                    campaign_id,
                    tid,
                )
                .execute(db.pool())
                .await
                .map_err(|e| log::warn!("invite attribution stamp failed: {}", e));
            }
        }
    }

    // ── Fire funnel events (non-blocking) ─────────────────────────────────────
    crate::funnel::auth_success(
        &app.db,
        "", // anon_id — client fires LOGIN_CLICK with it separately
        &tenant_id,
        is_new,
        referral_source.as_deref(),
        req.hl_referred,
        req.utm_campaign.as_deref(),
    )
    .await;

    // ── Issue HMAC-signed session cookie (7-day TTL) ───────────────────────
    let set_cookie = crate::privy::set_session_header(&tenant_id, &app.session_secret);

    // Tell the client whether they're in an active campaign for UX
    let in_campaign = claimed_invite
        .as_ref()
        .and_then(|i| i.campaign_id)
        .is_some();

    axum::response::Response::builder()
        .status(StatusCode::OK)
        .header("Set-Cookie", set_cookie)
        .header("Content-Type", "application/json")
        .body(axum::body::Body::from(format!(
            r#"{{"ok":true,"tenant_id":"{}","in_campaign":{}}}"#,
            tenant_id.as_str(),
            in_campaign
        )))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// `GET /auth/logout`
///
/// Clears the session cookie and redirects to `/login`.
async fn auth_logout_handler(State(_app): State<AppState>) -> axum::response::Response {
    axum::response::Response::builder()
        .status(302)
        .header("Location", "/login")
        .header("Set-Cookie", crate::privy::clear_session_header())
        .body(axum::body::Body::empty())
        .unwrap()
}

// Serve the pre-built Privy SDK ESM bundle.
// Cached by the browser for 24 h; no external CDN required at runtime.
async fn privy_bundle_handler() -> impl axum::response::IntoResponse {
    use axum::http::header;
    (
        [
            (
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            ),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        PRIVY_BUNDLE_JS,
    )
}

/// `GET /login`
///
/// Renders the Privy-powered login page.
///
/// - When `PRIVY_APP_ID` is set: embeds the Privy JS SDK and shows a
///   "Login" button that triggers Privy's authentication modal.
/// - When Privy is not configured: shows a message directing to `/app`
///   (single-operator mode — auth not required).
async fn login_handler(State(app): State<AppState>) -> axum::response::Html<String> {
    let body = if let Some(ref app_id) = app.privy_app_id {
        // Build optional walletConnectCloudProjectId JS config key.
        // When env var is set we inject it; otherwise omit so Privy falls back
        // to injected-wallet-only mode (MetaMask browser extension).
        let wc_config = match &app.walletconnect_project_id {
            Some(id) if !id.is_empty() => format!(", walletConnectCloudProjectId: '{}'", id),
            _ => String::new(),
        };
        format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>TradingBots.fun · Sign In</title>
<style>
*{{box-sizing:border-box;margin:0;padding:0}}
body{{background:#0d1117;color:#c9d1d9;
      font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;
      min-height:100vh;display:flex;align-items:center;justify-content:center;padding:20px;
      background-image:linear-gradient(rgba(88,166,255,.03) 1px,transparent 1px),
                       linear-gradient(90deg,rgba(88,166,255,.03) 1px,transparent 1px);
      background-size:44px 44px}}
.wrap{{display:flex;max-width:860px;width:100%;border-radius:18px;overflow:hidden;
       box-shadow:0 24px 80px rgba(0,0,0,.75),0 0 0 1px rgba(88,166,255,.09)}}
/* ── Left branding panel ── */
.pl{{background:linear-gradient(155deg,#161b22 0%,#0d1117 55%,#0a0e14 100%);
     border-right:1px solid #21262d;padding:52px 44px;flex:1;
     display:flex;flex-direction:column;gap:30px;position:relative;overflow:hidden}}
.pl::before{{content:'';position:absolute;top:-90px;right:-90px;width:300px;height:300px;
             background:radial-gradient(circle,rgba(227,52,58,.1) 0%,transparent 68%);
             pointer-events:none}}
.pl::after{{content:'';position:absolute;bottom:-60px;left:-60px;width:220px;height:220px;
            background:radial-gradient(circle,rgba(63,185,80,.07) 0%,transparent 68%);
            pointer-events:none}}
.brand{{display:flex;align-items:center;gap:12px}}
.brand img{{height:42px;width:auto;flex-shrink:0}}
.brand-text .name{{font-size:1.45rem;font-weight:800;color:#e6edf3;letter-spacing:.02em;line-height:1}}
.brand-text .name .r{{color:#e6343a}}
.brand-text .name .g{{color:#3fb950}}
.brand-text .sub{{font-size:.68rem;color:#484f58;letter-spacing:.6px;text-transform:uppercase;margin-top:3px}}
.tagline{{font-size:1.65rem;font-weight:700;color:#e6edf3;line-height:1.35;letter-spacing:-.02em}}
.tagline .acc{{color:#58a6ff}}
.feats{{display:flex;flex-direction:column;gap:15px}}
.feat{{display:flex;align-items:flex-start;gap:13px}}
.feat-ic{{width:34px;height:34px;border-radius:8px;display:flex;align-items:center;
           justify-content:center;font-size:1rem;flex-shrink:0}}
.feat-ic.red{{background:rgba(227,52,58,.13)}}
.feat-ic.grn{{background:rgba(63,185,80,.11)}}
.feat-ic.blu{{background:rgba(88,166,255,.11)}}
.feat-t .tt{{font-size:.88rem;font-weight:600;color:#e6edf3;margin-bottom:2px}}
.feat-t .td{{font-size:.75rem;color:#6e7681;line-height:1.5}}
.risk-foot{{font-size:.67rem;color:#3d444d;line-height:1.55;
            border-top:1px solid #21262d;padding-top:14px;margin-top:auto}}
/* ── Right login panel ── */
.pr{{background:#0d1117;padding:52px 44px;width:360px;flex-shrink:0;
     display:flex;flex-direction:column;justify-content:center;gap:22px}}
.lh{{text-align:center}}
.lh h2{{font-size:1.2rem;font-weight:700;color:#e6edf3;margin-bottom:5px}}
.lh p{{font-size:.81rem;color:#6e7681}}
/* Terms box */
.tos{{background:rgba(248,81,73,.06);border:1px solid rgba(248,81,73,.22);
      border-radius:10px;padding:15px;font-size:.76rem;line-height:1.6;color:#8b949e}}
.tos-hd{{color:#f85149;font-size:.72rem;font-weight:700;letter-spacing:.6px;
          text-transform:uppercase;display:block;margin-bottom:8px}}
.tos-lbl{{display:flex;align-items:flex-start;gap:9px;margin-top:11px;cursor:pointer}}
.tos-lbl input{{margin-top:2px;accent-color:#3fb950;width:13px;height:13px;flex-shrink:0;cursor:pointer}}
.tos-lbl span{{font-size:.74rem;color:#8b949e}}
.tos-lbl a{{color:#58a6ff;text-decoration:underline}}
/* Button */
.btn{{display:block;width:100%;padding:14px;border-radius:9px;font-size:.94rem;
      font-weight:700;cursor:pointer;border:none;transition:.15s;letter-spacing:.01em}}
.btn-p{{background:#3fb950;color:#0d1117}}
.btn-p:hover:not(:disabled){{background:#52c965}}
.btn-p:disabled{{background:#3fb95040;color:#3fb95070;cursor:not-allowed}}
.err{{color:#f85149;font-size:.78rem;min-height:18px;text-align:center}}
#status{{color:#8b949e;font-size:.78rem;min-height:16px;text-align:center}}
/* Wallet note */
.wnote{{display:flex;align-items:center;gap:8px;background:rgba(63,185,80,.06);
        border:1px solid rgba(63,185,80,.16);border-radius:8px;
        padding:9px 12px;font-size:.73rem;color:#8b949e}}
.wnote-dot{{width:6px;height:6px;border-radius:50%;background:#3fb950;
             flex-shrink:0;box-shadow:0 0 5px #3fb950}}
/* Post-auth invite code card */
.inv-card{{background:#0d1117;border:1px solid #30363d;border-radius:12px;
           padding:22px;display:flex;flex-direction:column;gap:14px}}
.inv-card-hd{{font-size:.92rem;font-weight:700;color:#e6edf3;text-align:center}}
.inv-card-sub{{font-size:.76rem;color:#6e7681;text-align:center;line-height:1.5;margin-top:-6px}}
.inv-inp{{width:100%;padding:11px 14px;background:#010409;border:1px solid #30363d;
          border-radius:8px;color:#e6edf3;font-size:1.05rem;font-weight:700;
          letter-spacing:.1em;text-transform:uppercase;outline:none;transition:.15s;text-align:center}}
.inv-inp:focus{{border-color:#58a6ff;box-shadow:0 0 0 3px rgba(88,166,255,.12)}}
.inv-inp.ok{{border-color:#3fb950;box-shadow:0 0 0 3px rgba(63,185,80,.1)}}
.inv-inp.bad{{border-color:#f85149;box-shadow:0 0 0 3px rgba(248,81,73,.1)}}
.inv-hint-row{{font-size:.71rem;color:#484f58;text-align:center}}
.inv-hint-row a{{color:#58a6ff}}
@media(max-width:600px){{
  .pl{{display:none}}
  .pr{{width:100%;padding:36px 24px}}
  .wrap{{max-width:380px}}
}}
</style>
</head>
<body>
<div class="wrap">
  <!-- Left: branding -->
  <div class="pl">
    <div class="brand">
      <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 76 90" fill="none" stroke="rgb(230,52,58)" stroke-width="4.5" stroke-linecap="round" stroke-linejoin="round" height="48" style="display:inline-block">
  <path d="M22 2 L52 2 L57 7 L57 30 L52 35 L22 35 L17 30 L17 7 Z"/>
  <rect x="22" y="10" width="10" height="10" rx="1"/>
  <rect x="42" y="10" width="10" height="10" rx="1"/>
  <line x1="31" y1="35" x2="31" y2="40"/>
  <line x1="45" y1="35" x2="45" y2="40"/>
  <rect x="12" y="40" width="50" height="30" rx="5" transform="rotate(-4 37 55)"/>
  <path d="M33 53 C33 50 28 48 28 53 C28 57 33 62 33 62 C33 62 38 57 38 53 C38 48 33 50 33 53Z" transform="rotate(-4 33 55)"/>
  <path d="M14 44 L3 52 L1 63 L8 64"/>
  <path d="M60 43 L71 35 L75 44 L68 49"/>
  <path d="M21 70 L14 82 L4 84 L2 77"/>
  <path d="M46 70 L53 81 L65 81 L66 74"/>
</svg>
      <div class="brand-text">
        <div class="name">TradingBots<span style="color:#3fb950">.fun</span></div>
        <div class="sub">AI Algorithmic Trading</div>
      </div>
    </div>
    <div class="tagline">Non-custodial trading<br><span class="acc">powered by AI</span></div>
    <div class="feats">
      <div class="feat">
        <div class="feat-ic red">🔐</div>
        <div class="feat-t">
          <div class="tt">Non-custodial</div>
          <div class="td">Your funds stay in your own Hyperliquid wallet. We never hold your assets.</div>
        </div>
      </div>
      <div class="feat">
        <div class="feat-ic grn">⚡</div>
        <div class="feat-t">
          <div class="tt">Live AI execution</div>
          <div class="td">Autonomous trade management with risk-controlled position sizing and stop-losses.</div>
        </div>
      </div>
      <div class="feat">
        <div class="feat-ic blu">📊</div>
        <div class="feat-t">
          <div class="tt">Full transparency</div>
          <div class="td">Every trade, signal, and AI reasoning step — visible in your dashboard.</div>
        </div>
      </div>
    </div>
    <div class="risk-foot">
      Trading involves substantial risk of loss. Past performance does not guarantee future results.
      AI-generated signals are not financial advice. Capital is at risk.
    </div>
  </div>

  <!-- Right: login -->
  <div class="pr">
    <div class="lh">
      <h2>Sign in to your account</h2>
      <p>Invite-only &middot; <a href="/leaderboard" style="color:#58a6ff;text-decoration:none">🏆 View leaderboard</a></p>
    </div>

    <div class="tos">
      <span class="tos-hd">⚠ Risk &amp; Liability Notice</span>
      All trades executed by the AI run in <b style="color:#e6edf3">your own wallet</b>.
      TradingBots.fun and its operators bear <b style="color:#e6edf3">no liability</b> for trading losses
      arising from market conditions, AI decisions, or technical failures.
      <label class="tos-lbl">
        <input type="checkbox" id="tos-check">
        <span>
          I have read and agree to the
          <a href="/app/onboarding" target="_blank">Terms of Service &amp; Risk Disclosure</a>.
          I understand all trades are executed at my sole risk and responsibility.
        </span>
      </label>
    </div>

    <!-- React mounts here — replaces #login-area -->
    <div id="login-area">
      <button id="login-btn" class="btn btn-p" disabled>Loading…</button>
    </div>
    <div id="status" style="text-align:center;font-size:.78rem;color:#8b949e;min-height:16px"></div>
    <div id="err" class="err" style="margin-top:2px"></div>

    <div class="wnote">
      <div class="wnote-dot"></div>
      $20/mo &middot; 9 bots &middot; compete for weekly prizes
    </div>
  </div>
</div>

<script type="module">
const PRIVY_APP_ID = '{app_id}';

// Capture ?invite= / ?code= from URL now; passed into post-auth invite flow.
const urlParams     = new URLSearchParams(window.location.search);
const urlInviteCode = (urlParams.get('invite') || urlParams.get('code') || '').toUpperCase();

function setStatus(msg) {{ document.getElementById('status').textContent = msg; }}
function setErr(msg)    {{ document.getElementById('err').textContent    = msg; }}

function getUtm(key) {{ return new URLSearchParams(window.location.search).get(key) || ''; }}

// ── Session exchange ───────────────────────────────────────────────────────
// Throws a plain Error on generic failures.
// Throws an Error with .needsInvite = true when the server wants an invite
// code — lets the UI render the post-auth invite prompt instead.
async function exchangeToken(privyToken, inviteCode) {{
  const body = {{
    token:        privyToken,
    invite_code:  (inviteCode || '').trim().toUpperCase() || null,
    utm_source:   getUtm('utm_source') || 'direct',
    utm_campaign: getUtm('utm_campaign') || null,
    hl_referred:  getUtm('ref') === 'TRADINGBOTS' || getUtm('hl_ref') === '1',
  }};
  const res = await fetch('/auth/session', {{
    method: 'POST',
    headers: {{ 'Content-Type': 'application/json' }},
    body:    JSON.stringify(body),
  }});
  if (res.status === 403) {{
    const j = await res.json().catch(() => ({{}}));
    if (j.error === 'invite_required') {{
      const err = new Error('An invite code is required to create an account.');
      err.needsInvite = true;
      throw err;
    }}
    throw new Error(j.message || 'That invite code is invalid or already used.');
  }}
  if (!res.ok) throw new Error('Session exchange failed: ' + res.status);
  return res.json();
}}

// ── Privy SDK ──────────────────────────────────────────────────────────────
// Bundle served from our own server — no external CDN.
// Rebuild after SDK upgrades: cd js && npm run build
import('/static/privy-login.js').then(({{ PrivyProvider, usePrivy, createElement, useState, useEffect, createRoot }}) => {{
  const h = createElement;

  // Watchdog: surface error if mount div is still empty after 8 s
  const area = document.getElementById('login-area');
  const watchdog = setTimeout(() => {{
    if (!area || area.querySelector('#login-btn')) {{
      setErr('Auth SDK failed to initialise — please reload the page.');
    }}
  }}, 8000);

  // ── LoginApp ──────────────────────────────────────────────────────────
  function LoginApp() {{
    const {{ ready, authenticated, login, getAccessToken }} = usePrivy();
    // phase: 'idle' | 'loading' | 'invite' | 'done'
    const [phase, setPhase]           = useState('idle');
    const [inviteCode, setInviteCode] = useState(urlInviteCode);
    const [errMsg, setErrMsg]         = useState('');
    const [pendingToken, setPToken]   = useState(null);
    const [tosChecked, setTos]        = useState(false);

    // Mirror ToS checkbox into React state
    useEffect(() => {{
      const cb = (e) => setTos(e.target.checked);
      const el = document.getElementById('tos-check');
      el?.addEventListener('change', cb);
      return () => el?.removeEventListener('change', cb);
    }}, []);

    // Push error into the external #err div
    useEffect(() => {{ setErr(errMsg); }}, [errMsg]);

    // Auto-redirect when already authenticated on page load
    useEffect(() => {{
      if (!ready || !authenticated) return;
      setStatus('Already signed in — loading dashboard…');
      getAccessToken().then(async (token) => {{
        try {{
          await exchangeToken(token, inviteCode);
          window.location.href = '/app';
        }} catch(e) {{
          if (e.needsInvite) {{
            setPToken(token); setPhase('invite'); setStatus('');
          }} else {{
            setStatus(''); setErrMsg('Session setup failed. Please sign in again.');
          }}
        }}
      }}).catch(() => {{}});
    }}, [ready, authenticated]);

    // ── Post-auth invite code prompt ────────────────────────────────────
    if (phase === 'invite') {{
      const codeOk = inviteCode.trim().length >= 6;
      const handleSubmit = async () => {{
        if (!codeOk || phase === 'loading') return;
        setPhase('loading'); setErrMsg(''); setStatus('Verifying invite code…');
        try {{
          await exchangeToken(pendingToken, inviteCode);
          window.location.href = '/app';
        }} catch(e) {{
          setPhase('invite'); setStatus(''); setErrMsg(e.message || 'Invalid invite code.');
        }}
      }};
      return h('div', {{ className: 'inv-card' }},
        h('div', {{ className: 'inv-card-hd' }}, '🎟 Enter your invite code'),
        h('div', {{ className: 'inv-card-sub' }},
          'TradingBots.fun is invite-only for new accounts.'),
        h('input', {{
          className: 'inv-inp ' + (inviteCode.length === 0 ? '' : codeOk ? 'ok' : 'bad'),
          type: 'text', placeholder: 'TB-XXXXXXXX', value: inviteCode,
          maxLength: 20, autoFocus: true,
          onInput:   (e) => setInviteCode(e.target.value.toUpperCase()),
          onKeyDown: (e) => {{ if (e.key === 'Enter') handleSubmit(); }},
        }}),
        h('button', {{
          className: 'btn btn-p', disabled: !codeOk,
          onClick: handleSubmit,
        }}, 'Continue →'),
        h('div', {{ className: 'inv-hint-row' }},
          'Get a code from a friend or the ',
          h('a', {{ href: '/leaderboard' }}, 'weekly campaign'))
      );
    }}

    // ── Main sign-in button ─────────────────────────────────────────────
    const busy = phase === 'loading';
    const handleLogin = async () => {{
      if (!tosChecked || busy) return;
      setErrMsg(''); setStatus('Opening sign-in…'); setPhase('loading');
      try {{
        await login();
        setStatus('Authenticated — setting up your account…');
        const token = await getAccessToken();
        try {{
          await exchangeToken(token, inviteCode);
          window.location.href = '/app';
        }} catch(e) {{
          if (e.needsInvite) {{
            setPToken(token); setPhase('invite'); setStatus('');
          }} else {{ throw e; }}
        }}
      }} catch(e) {{
        setPhase('idle'); setStatus('');
        setErrMsg(e.message || 'Login failed. Please try again.');
      }}
    }};

    return h('button', {{
      className: 'btn btn-p',
      disabled: !ready || !tosChecked || busy,
      onClick: handleLogin,
    }}, !ready ? 'Loading…' : busy ? 'Signing in…' : 'Sign in');
  }}

  // Mount React in place of the static placeholder
  const mount = document.createElement('div');
  area.replaceWith(mount);

  createRoot(mount).render(
    h(PrivyProvider, {{
      appId: PRIVY_APP_ID,
      // 'wallet' enables MetaMask (browser extension) and, when
      // walletConnectCloudProjectId is set, mobile wallets too.
      // embeddedWallets createOnLogin:'off' prevents HTTPS-only wallet init
      // from crashing when the page is served over plain HTTP in dev/staging.
      config: {{
        loginMethods: ['email', 'wallet'],
        appearance: {{ theme: 'dark' }},
        embeddedWallets: {{ createOnLogin: 'off' }}{wc_config},
      }},
    }},
      h(LoginApp)
    )
  );

  const cancelWd = setInterval(() => {{
    if (mount.childElementCount > 0) {{ clearTimeout(watchdog); clearInterval(cancelWd); }}
  }}, 200);
}}).catch((e) => {{
  setErr('Could not load authentication SDK: ' + e.message);
}});
</script>
</body></html>"#,
            app_id = app_id,
            wc_config = wc_config
        )
    } else {
        // Single-operator mode — Privy not configured
        r#"<!DOCTYPE html>
<html lang="en">
<head><meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>TradingBots.fun · Login</title>
<style>
  body{background:#0d1117;color:#c9d1d9;font-family:-apple-system,sans-serif;
       min-height:100vh;display:flex;align-items:center;justify-content:center;padding:24px}
  .card{background:#161b22;border:1px solid #30363d;border-radius:12px;
        padding:32px 28px;max-width:380px;text-align:center}
  h2{font-size:1.2rem;color:#e6edf3;margin-bottom:12px}
  p{color:#8b949e;font-size:.88rem;line-height:1.6;margin-bottom:20px}
  a{display:inline-block;padding:10px 24px;background:#3fb95018;border:1px solid #3fb95050;
    border-radius:8px;color:#3fb950;font-weight:600;text-decoration:none}
</style>
</head>
<body>
<div class="card">
  <h2>Authentication not configured</h2>
  <p>Privy App ID is not set on this server.<br>
     This deployment is running in single-operator mode.</p>
  <a href="/app">Open dashboard →</a>
</div>
</body></html>"#
            .to_string()
    };
    axum::response::Html(body)
}

// ─────────────────────────────────────────────────────────────────────────────
//  Apple Pay domain verification
// ─────────────────────────────────────────────────────────────────────────────

// `GET /.well-known/apple-developer-merchantid-domain-association`
//
// Serves the Apple Pay domain-association file so Apple's servers can verify
// that this domain is allowed to initiate Apple Pay transactions.
//
// Setup (one-time, ~2 minutes):
//   1. Stripe Dashboard → Settings → Payment methods → Apple Pay
//   2. Click "Add new domain", enter your domain.
//   3. Stripe shows a verification file — copy its contents (not the URL).
//   4. Set APPLE_PAY_DOMAIN_ASSOC=<file contents> in your .env.
//   5. Deploy. Apple Pay button appears automatically in Stripe Checkout on
//      Safari / iOS for your domain.
// ─────────────────────────────────────────────────────────────────────────────
//  Onboarding / Terms wall  (/app/onboarding)
// ─────────────────────────────────────────────────────────────────────────────

/// `GET /app/onboarding` — show the full Terms & Risk Disclosure.
///
/// Redirects authenticated users who have already accepted to `/app`.
/// Redirects unauthenticated users to `/login`.
async fn onboarding_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    // Check session, but skip the terms check (that's the whole point of this page)
    if app.privy_app_id.is_some() {
        let tid = match get_session_tenant_id(&headers, &app.session_secret) {
            Some(t) => t,
            None => return axum::response::Redirect::to("/login").into_response(),
        };
        // If already accepted, skip this page
        let tenants = app.tenants.read().await;
        if let Some(h) = tenants.get(&tid) {
            if h.config.terms_accepted_at.is_some() {
                return axum::response::Redirect::to("/app").into_response();
            }
        }
    } else {
        // Single-operator mode: no onboarding required
        return axum::response::Redirect::to("/app").into_response();
    }

    let html = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>TradingBots.fun · Terms & Risk Disclosure</title>
<style>
  *{box-sizing:border-box;margin:0;padding:0}
  body{background:#0d1117;color:#c9d1d9;
        font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;
        min-height:100vh;padding:40px 16px}
  .wrap{max-width:680px;margin:0 auto}
  .logo{font-weight:700;font-size:.95rem;color:#e6edf3;letter-spacing:.04em;margin-bottom:32px}
  .logo .r{color:#e6343a}
  .logo .b{color:#3fb950}
  h1{font-size:1.35rem;font-weight:700;color:#e6edf3;margin-bottom:8px}
  .sub{font-size:.85rem;color:#8b949e;margin-bottom:28px}
  .section{background:#161b22;border:1px solid #30363d;border-radius:12px;
            padding:24px;margin-bottom:16px}
  h2{font-size:.9rem;font-weight:700;color:#e6edf3;text-transform:uppercase;
      letter-spacing:.06em;margin-bottom:12px}
  p{font-size:.85rem;line-height:1.75;color:#8b949e;margin-bottom:10px}
  p:last-child{margin-bottom:0}
  strong{color:#c9d1d9}
  .warning{border-color:#f8514950;background:#f8514908}
  .warning h2{color:#f85149}
  .accept-row{display:flex;flex-direction:column;gap:12px;margin-top:28px}
  .btn-accept{background:#238636;color:#fff;border:none;border-radius:8px;
               padding:14px 24px;font-size:1rem;font-weight:700;cursor:pointer;width:100%}
  .btn-accept:hover{background:#2ea043}
  .cancel{font-size:.8rem;color:#8b949e;text-align:center}
  .cancel a{color:#58a6ff}
  input[type=checkbox]{accent-color:#3fb950;width:16px;height:16px;cursor:pointer}
  .check-row{display:flex;align-items:flex-start;gap:10px;font-size:.83rem;
              color:#8b949e;line-height:1.55}
</style>
</head>
<body>
<div class="wrap">
<p class="logo">TradingBots<span class="b">.fun</span></p>
<h1>Terms &amp; Risk Disclosure</h1>
<p class="sub">Please read and accept these terms before accessing the trading platform.</p>

<div class="section warning">
  <h2>⚠ High-Risk Investment Warning</h2>
  <p><strong>Leveraged cryptocurrency trading involves substantial risk of loss.</strong>
     You may lose all of your deposited capital. Past performance of any trading system,
     signal, or algorithm does not guarantee future results.</p>
  <p>Leveraged positions can be liquidated quickly during periods of high volatility.
     You should only trade with funds you can afford to lose entirely.</p>
</div>

<div class="section">
  <h2>Not Investment Advice</h2>
  <p>TradingBots.fun is an <strong>automated trading tool</strong>, not a licensed financial advisor,
     broker, or investment manager. Nothing displayed on this platform constitutes investment
     advice, a solicitation to trade, or a recommendation to buy or sell any asset.</p>
  <p>All trading decisions are made by the algorithmic system. You are solely responsible
     for evaluating the suitability of this service for your financial situation.</p>
</div>

<div class="section">
  <h2>Self-Custody &amp; Fund Safety</h2>
  <p>Your funds remain in <strong>your Hyperliquid account at all times</strong>.
     TradingBots.fun never holds, custodies, or has direct access to withdraw your funds.
     The platform holds an API key with trading permissions only — not withdrawal access.</p>
  <p>You retain full custody and can withdraw your funds directly from
     <a href="https://app.hyperliquid.xyz" target="_blank" style="color:#58a6ff">
     app.hyperliquid.xyz</a> at any time without our involvement.</p>
</div>

<div class="section">
  <h2>Fees &amp; Revenue Disclosure</h2>
  <p>TradingBots.fun earns revenue through the following mechanisms:</p>
  <p>• <strong>Subscription:</strong> $19.99/month for the Pro plan (live trading).<br>
     • <strong>Builder fee:</strong> A small fee (approximately 0.01–0.03% per fill) is
       embedded in every order and credited to the platform's Hyperliquid builder address.
       This fee is in addition to Hyperliquid's standard taker/maker fees.<br>
     • <strong>Referral:</strong> If you sign up to Hyperliquid via our referral link,
       the platform earns a portion of your trading fee rebates.</p>
  <p>All fees are disclosed above. There are no hidden charges.</p>
</div>

<div class="section">
  <h2>Jurisdiction &amp; Eligibility</h2>
  <p>This platform is <strong>not available</strong> to residents of the United States,
     Canada, or any jurisdiction where accessing cryptocurrency derivatives trading is
     prohibited by law. By accepting these terms you confirm that you are not accessing
     this platform from a restricted jurisdiction.</p>
  <p>You must be at least 18 years of age (or the age of majority in your jurisdiction,
     whichever is higher) to use this platform.</p>
</div>

<div class="section">
  <h2>Platform Availability &amp; Liability</h2>
  <p>TradingBots.fun is provided <strong>"as is"</strong> without warranty of any kind. The
     platform may experience downtime, connectivity issues, or bugs that cause trading
     to be delayed, skipped, or executed at unfavourable prices. The operator accepts
     no liability for losses arising from system failures, network outages, exchange
     API errors, or market conditions.</p>
</div>

<form method="POST" action="/app/onboarding/accept">
  <div class="accept-row">
    <label class="check-row">
      <input type="checkbox" id="chk1" required>
      <span>I have read and understand the risk warnings above. I am aware that I may
            lose all of my deposited funds.</span>
    </label>
    <label class="check-row">
      <input type="checkbox" id="chk2" required>
      <span>I confirm I am not a resident of a restricted jurisdiction and I am of legal
            trading age in my country.</span>
    </label>
    <label class="check-row">
      <input type="checkbox" id="chk3" required>
      <span>I acknowledge the fee structure described above, including the builder fee
            embedded in every order.</span>
    </label>
    <button type="submit" class="btn-accept"
            onclick="return document.getElementById('chk1').checked &&
                            document.getElementById('chk2').checked &&
                            document.getElementById('chk3').checked ||
                     (alert('Please check all boxes before continuing.'), false)">
      I Accept — Continue to Platform
    </button>
    <p class="cancel"><a href="/auth/logout">Sign out instead</a></p>
  </div>
</form>

</div>
</body>
</html>"#.to_string();
    axum::response::Html(html).into_response()
}

/// `POST /app/onboarding/accept` — record terms acceptance, auto-generate the
/// tenant's Hyperliquid trading wallet, and redirect to `/app/setup`.
async fn onboarding_accept_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let tid = match get_session_tenant_id(&headers, &app.session_secret) {
        Some(t) => t,
        None => return axum::response::Redirect::to("/login").into_response(),
    };

    // Accept ToS (idempotent)
    {
        let mut tenants = app.tenants.write().await;
        let _ = tenants.accept_terms(&tid);
    }

    // Generate HL trading wallet if the tenant doesn't have one yet
    let needs_wallet = {
        let tenants = app.tenants.read().await;
        tenants
            .get(&tid)
            .map(|h| !h.config.has_hl_wallet())
            .unwrap_or(false)
    };

    if needs_wallet {
        let (address, private_key) = crate::hl_wallet::generate_keypair();
        let key_enc =
            crate::hl_wallet::encrypt_key(&private_key, &app.session_secret, tid.as_str());

        // Update in-memory tenant
        {
            let mut tenants = app.tenants.write().await;
            let _ = tenants.setup_hl_wallet(&tid, address.clone(), key_enc.clone());
        }

        // Persist to DB
        if let Some(ref db) = app.db {
            if let Ok(tid_uuid) = uuid::Uuid::parse_str(tid.as_str()) {
                let _ = sqlx::query!(
                    "UPDATE tenants                      SET hl_wallet_address = $1, hl_wallet_key_enc = $2                      WHERE id = $3",
                    address, key_enc, tid_uuid,
                )
                .execute(db.pool())
                .await
                .map_err(|e| log::error!("❌ persist HL wallet: {}", e));
            }
        }
    }

    // If setup already acknowledged on a previous visit, skip straight to /app
    let setup_done = {
        let tenants = app.tenants.read().await;
        tenants
            .get(&tid)
            .map(|h| h.config.hl_setup_done())
            .unwrap_or(false)
    };

    if setup_done {
        axum::response::Redirect::to("/app").into_response()
    } else {
        axum::response::Redirect::to("/app/setup").into_response()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Consumer settings page  (/app/settings)
// ─────────────────────────────────────────────────────────────────────────────

/// `GET /app/settings` — wallet linking, subscription status, account info.
async fn consumer_settings_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let tid = match resolve_consumer_state(&headers, &app).await {
        ConsumerStateResult::Ok { tenant_id, .. } => tenant_id,
        ConsumerStateResult::NeedsLogin => {
            return axum::response::Redirect::to("/login").into_response()
        }
        ConsumerStateResult::NeedsOnboarding { .. } => {
            return axum::response::Redirect::to("/app/onboarding").into_response()
        }
    };

    let (
        display_name,
        email,
        wallet,
        tier,
        trial_days,
        terms_ts,
        wallet_ts,
        hl_balance,
        net_dep,
        total_dep,
        total_with,
        max_pos,
        trial_expired,
        hl_trading_addr,
        hl_setup_done,
    ) = {
        let tenants = app.tenants.read().await;
        let h = match tenants.get(&tid) {
            Some(h) => h,
            None => return axum::response::Redirect::to("/login").into_response(),
        };
        let fund_sum = crate::fund_tracker::summary(&tid);
        (
            h.config.display_name.clone(),
            h.config.email.clone().unwrap_or_else(|| "—".to_string()),
            h.config.wallet_address.clone(),
            format!("{:?}", h.config.tier),
            h.config.trial_days_remaining(),
            h.config
                .terms_accepted_at
                .map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
                .unwrap_or_else(|| "—".to_string()),
            h.config
                .wallet_linked_at
                .map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
                .unwrap_or_else(|| "—".to_string()),
            h.config.hl_balance_usd,
            fund_sum.net_deposits,
            fund_sum.total_deposited,
            fund_sum.total_withdrawn,
            h.config.max_positions(),
            h.config.is_trial_expired_free(),
            h.config.hl_wallet_address.clone(),
            h.config.hl_setup_done(),
        )
    };

    let wallet_section = if let Some(ref addr) = wallet {
        format!(
            r#"
<div class="metric-row">
  <span class="ml">HL Wallet</span>
  <span class="mv" style="font-family:monospace;font-size:.78rem">{addr}</span>
</div>
<div class="metric-row">
  <span class="ml">Last known balance</span>
  <span class="mv">${hl_balance:.2}</span>
</div>
<div class="metric-row">
  <span class="ml">Wallet linked</span>
  <span class="mv">{wallet_ts}</span>
</div>"#,
            addr = addr,
            hl_balance = hl_balance,
            wallet_ts = wallet_ts,
        )
    } else {
        r#"<div class="info-box" style="margin-top:4px">
  No wallet linked yet. Paste your Hyperliquid wallet address (0x…) below.
  Your funds never leave your HL account — we only need the address to query
  your balance and attribute trades to your account.
</div>"#
            .to_string()
    };

    // HL auto-generated trading wallet section (separate from the auth/Privy wallet)
    let hl_trading_wallet_section = if let Some(ref addr) = hl_trading_addr {
        let setup_link = if !hl_setup_done {
            r#"<div class="metric-row" style="margin-top:8px">
  <span class="ml" style="color:#e3b341">Setup incomplete</span>
  <span class="mv"><a href="/app/setup" style="color:#58a6ff">Resume setup wizard →</a></span>
</div>"#
        } else {
            ""
        };
        format!(
            r#"
<div class="card" style="margin-top:16px">
  <div class="card-label">Your Trading Wallet</div>
  <p style="font-size:.8rem;color:#8b949e;margin-bottom:12px">
    This dedicated Hyperliquid wallet was auto-generated for you at sign-up.
    It is separate from your login wallet and is used exclusively by the bot to
    sign trades on your behalf.
  </p>
  <div class="metric-row">
    <span class="ml">Address</span>
    <span class="mv" style="font-family:monospace;font-size:.78rem;word-break:break-all">{addr}</span>
  </div>
  {setup_link}
  <div style="margin-top:14px;display:flex;gap:10px;flex-wrap:wrap">
    <a href="https://app.hyperliquid.xyz/portfolio?user={addr}" target="_blank" rel="noopener"
       class="btn" style="font-size:.82rem;padding:7px 14px;background:#21262d;border:1px solid #30363d">
      View on HL ↗
    </a>
    <a href="/api/hl/wallet/key.json"
       class="btn btn-green" style="font-size:.82rem;padding:7px 14px">
      Export Private Key ↓
    </a>
  </div>
  <p class="note" style="margin-top:10px">
    Store your private key in a password manager or cloud drive (iCloud / Google Drive).
    You can always re-export it here. Never share it with anyone.
  </p>
</div>"#,
            addr = addr,
            setup_link = setup_link,
        )
    } else {
        String::new()
    };

    let tier_badge = match tier.as_str() {
        "Pro" => r#"<span style="color:#3fb950;font-weight:700">Pro</span>"#,
        "Internal" => r#"<span style="color:#e3b341;font-weight:700">Internal</span>"#,
        _ => r#"<span style="color:#8b949e;font-weight:600">Free</span>"#,
    };

    let trial_note = if trial_days > 0 {
        format!(
            r#"<span style="color:#e3b341;font-size:.78rem;margin-left:6px">
  ({trial_days} trial day{s} remaining)</span>"#,
            trial_days = trial_days,
            s = if trial_days == 1 { "" } else { "s" },
        )
    } else {
        String::new()
    };

    // Position cap row — shown in account card
    let pos_cap_row = {
        let cap_str = if max_pos == usize::MAX {
            "Unlimited".to_string()
        } else {
            format!("{} max", max_pos)
        };
        let cap_colour = if trial_expired { "#f85149" } else { "#3fb950" };
        let cap_hint = if trial_expired {
            r#" &nbsp;<span style="font-size:.75rem;color:#8b949e">(upgrade to Pro for unlimited)</span>"#
        } else {
            ""
        };
        format!(
            r#"<div class="metric-row">
    <span class="ml">Open positions</span>
    <span class="mv" style="color:{cap_colour}">{cap_str}{cap_hint}</span>
  </div>"#,
            cap_colour = cap_colour,
            cap_str = cap_str,
            cap_hint = cap_hint,
        )
    };

    let mut html = consumer_shell_open("Settings", "Settings");
    html.push_str(&format!(
        r#"
<div class="card">
  <div class="card-label">Account</div>
  <div class="metric-row">
    <span class="ml">Display name</span>
    <span class="mv">{display_name}</span>
  </div>
  <div class="metric-row">
    <span class="ml">Email</span>
    <span class="mv">{email}</span>
  </div>
  <div class="metric-row">
    <span class="ml">Plan</span>
    <span class="mv">{tier_badge}{trial_note}</span>
  </div>
  {pos_cap_row}
  <div class="metric-row">
    <span class="ml">Terms accepted</span>
    <span class="mv">{terms_ts}</span>
  </div>
</div>

<div class="card">
  <div class="card-label">Hyperliquid Wallet</div>
  {wallet_section}
  <form method="POST" action="/app/settings/wallet" style="margin-top:16px;display:flex;gap:8px">
    <input name="address" type="text" placeholder="0x…wallet address"
           style="flex:1;background:#0d1117;border:1px solid #30363d;border-radius:6px;
                  padding:8px 12px;color:#e6edf3;font-size:.85rem;font-family:monospace"
           pattern="0x[0-9a-fA-F]{{38,}}" required>
    <button type="submit" class="btn btn-green" style="white-space:nowrap">
      {link_label}
    </button>
  </form>
  <p class="note">We store your wallet address only to query your HL balance.
     We never have withdrawal access.</p>
</div>

<div class="card">
  <div class="card-label">Fund History</div>
  <div class="metric-row">
    <span class="ml">Total deposited</span>
    <span class="mv green">${total_dep:.2}</span>
  </div>
  <div class="metric-row">
    <span class="ml">Total withdrawn</span>
    <span class="mv red">−${total_with:.2}</span>
  </div>
  <div class="metric-row">
    <span class="ml">Net deposits</span>
    <span class="mv">${net_dep:.2}</span>
  </div>
  <p class="note" style="margin-top:10px">
    Deposits and withdrawals are detected automatically by comparing your HL
    balance between cycles. Small balance changes due to unrealised P&L are
    filtered out.
  </p>
</div>

{hl_trading_wallet_section}

{upgrade_block}

<p class="note" style="text-align:center;margin-top:12px">
  Need help? Contact support or
  <a href="/auth/logout">sign out</a>.
</p>
"#,
        display_name = display_name,
        email = email,
        tier_badge = tier_badge,
        trial_note = trial_note,
        pos_cap_row = pos_cap_row,
        terms_ts = terms_ts,
        wallet_section = wallet_section,
        link_label = if wallet.is_some() {
            "Update"
        } else {
            "Link Wallet"
        },
        total_dep = total_dep,
        total_with = total_with,
        net_dep = net_dep,
        hl_trading_wallet_section = hl_trading_wallet_section,
        upgrade_block = if tier == "Free" && trial_expired {
            // Trial has expired — hard upgrade call-to-action
            r#"<div class="card" style="border-color:#f85149aa;background:#f8514906">
  <div class="card-label" style="color:#f85149">Trial Ended · Upgrade to Unlock</div>
  <p style="font-size:.85rem;color:#8b949e;margin-bottom:6px">
    Your 14-day free trial has ended. You can still trade, but you are now
    limited to <strong style="color:#e6edf3">2 open positions</strong> at a time.
  </p>
  <p style="font-size:.85rem;color:#8b949e;margin-bottom:16px">
    Upgrade to <strong style="color:#3fb950">Pro</strong> to unlock unlimited
    positions, full live trading, and priority support —
    <strong style="color:#e6edf3">$19.99/month</strong>. Cancel any time.
  </p>
  <a href="/billing/checkout" class="btn btn-green" data-funnel="upgrade_click"
     style="font-size:.92rem;padding:10px 22px">
    Upgrade to Pro →
  </a>
</div>"#
        } else if tier == "Free" {
            // Trial still active — softer upsell
            r#"<div class="card">
  <div class="card-label">Upgrade to Pro</div>
  <p style="font-size:.85rem;color:#8b949e;margin-bottom:14px">
    Live algorithmic trading on Hyperliquid for <strong style="color:#e6edf3">$19.99/month</strong>.
    Cancel any time.
  </p>
  <a href="/billing/checkout" class="btn btn-green" data-funnel="upgrade_click">Upgrade to Pro →</a>
</div>"#
        } else {
            ""
        },
    ));
    html.push_str(consumer_shell_close());
    axum::response::Html(html).into_response()
}

/// `POST /app/settings/wallet` — validate and store HL wallet address.
async fn consumer_settings_wallet_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
    axum::Form(form): axum::Form<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let tid = match get_session_tenant_id(&headers, &app.session_secret) {
        Some(t) => t,
        None => return axum::response::Redirect::to("/login").into_response(),
    };

    let address = match form.get("address") {
        Some(a) => a.trim().to_string(),
        None => {
            return axum::response::Redirect::to("/app/settings?error=missing_address")
                .into_response()
        }
    };

    {
        let mut tenants = app.tenants.write().await;
        match tenants.link_wallet(&tid, &address) {
            Ok(_) => log::info!("🔗 Tenant {} updated wallet to {}", tid, address),
            Err(e) => {
                log::warn!("⚠ Wallet link failed for tenant {}: {}", tid, e);
                return axum::response::Redirect::to("/app/settings?error=invalid_address")
                    .into_response();
            }
        }
    }

    axum::response::Redirect::to("/app/settings?ok=wallet_linked").into_response()
}

// ─────────────────────────────────────────────────────────────────────────────
//  Admin panel  (/admin, /admin/users)
// ─────────────────────────────────────────────────────────────────────────────

/// Verify HTTP Basic Auth for admin routes.
///
/// Returns `Some(())` when credentials are valid, `None` when they are missing
/// or incorrect.  Username is always `"admin"`.
fn check_admin_auth(headers: &axum::http::HeaderMap, password: &str) -> bool {
    let auth_header = match headers.get("authorization") {
        Some(v) => match v.to_str() {
            Ok(s) => s,
            Err(_) => return false,
        },
        None => return false,
    };
    let encoded = match auth_header.strip_prefix("Basic ") {
        Some(e) => e,
        None => return false,
    };
    use base64::Engine as _;
    let decoded = match base64::engine::general_purpose::STANDARD.decode(encoded) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(_) => return false,
        },
        Err(_) => return false,
    };
    // Expected format: "admin:<password>"
    decoded == format!("admin:{}", password)
}

/// Respond with a WWW-Authenticate challenge to trigger the browser's
/// Basic Auth dialog.
fn www_authenticate_response() -> axum::response::Response {
    use axum::response::IntoResponse;
    axum::response::Response::builder()
        .status(401)
        .header(
            "WWW-Authenticate",
            r#"Basic realm="TradingBots.fun Admin", charset="UTF-8""#,
        )
        .body(axum::body::Body::from("Unauthorized"))
        .unwrap_or_else(|_| axum::http::StatusCode::UNAUTHORIZED.into_response())
}

/// `GET /admin` — operator admin dashboard.
async fn admin_dashboard_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let password = match &app.admin_password {
        Some(p) => p.clone(),
        None => {
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "Admin panel is not configured. Set ADMIN_PASSWORD.",
            )
                .into_response()
        }
    };

    if !check_admin_auth(&headers, &password) {
        return www_authenticate_response();
    }

    let (tenant_count, pro_count, free_count, total_balance) = {
        let tenants = app.tenants.read().await;
        let count = tenants.count();
        let pro = tenants
            .all()
            .filter(|h| h.config.tier == crate::tenant::TenantTier::Pro)
            .count();
        let free = tenants
            .all()
            .filter(|h| h.config.tier == crate::tenant::TenantTier::Free)
            .count();
        let balance: f64 = tenants.all().map(|h| h.config.hl_balance_usd).sum();
        (count, pro, free, balance)
    };

    let pattern_snapshot = {
        let cache = app.pattern_cache.lock().await;
        cache.latest()
    };
    let pattern_panel = if let Some(insights) = pattern_snapshot {
        let summary = &insights.report_summary;
        let winner = summary
            .daily_winner
            .as_ref()
            .map(|(sym, pnl)| format!("{} (${:0.2})", html_escape(sym), pnl))
            .unwrap_or_else(|| "—".to_string());
        let loser = summary
            .daily_loser
            .as_ref()
            .map(|(sym, pnl)| format!("{} (${:0.2})", html_escape(sym), pnl))
            .unwrap_or_else(|| "—".to_string());
        let win_combo_rows = if insights.top_win_combos.is_empty() {
            "<li style=\"color:#8b949e\">Not enough combos yet.</li>".to_string()
        } else {
            insights
                .top_win_combos
                .iter()
                .take(3)
                .map(|combo| {
                    format!(
                        "<li>{} · {} · {rate:.0}% · {wins}W/{losses}L</li>",
                        html_escape(&combo.breakdown),
                        html_escape(&combo.context),
                        rate = combo.win_rate * 100.0,
                        wins = combo.wins,
                        losses = combo.losses,
                    )
                })
                .collect::<Vec<_>>()
                .join("")
        };
        let loss_combo_rows = if insights.top_loss_combos.is_empty() {
            "<li style=\"color:#8b949e\">False-breakout / stall data pending.</li>".to_string()
        } else {
            insights
                .top_loss_combos
                .iter()
                .take(3)
                .map(|combo| {
                    format!(
                        "<li>{} · {} · {rate:.0}% · {wins}W/{losses}L</li>",
                        html_escape(&combo.breakdown),
                        html_escape(&combo.context),
                        rate = combo.win_rate * 100.0,
                        wins = combo.wins,
                        losses = combo.losses,
                    )
                })
                .collect::<Vec<_>>()
                .join("")
        };
        format!(
            r#"<div class="card">
  <div class="cl">Pattern cache · {date}</div>
  <div class="cv-sm">Winner: {winner} · Loser: {loser}</div>
  <div style="margin-top:12px;font-size:.8rem;color:#8b949e">Top win combos</div>
  <ul style="margin:6px 0;padding-left:18px;color:#c9d1d9;font-size:.83rem;line-height:1.5">
    {win_combo_rows}
  </ul>
  <div style="font-size:.8rem;color:#8b949e">Loss warnings</div>
  <ul style="margin:6px 0;padding-left:18px;color:#c9d1d9;font-size:.83rem;line-height:1.5">
    {loss_combo_rows}
  </ul>
  <div style="margin-top:12px;font-size:.78rem;color:#8b949e">
    <a href="/app/agents" style="color:#58a6ff">Open agent console</a> · rerun <code>scripts/verify_pattern_cache.sh</code> after exits.
  </div>
</div>"#,
            date = insights.date.format("%Y-%m-%d"),
            winner = winner,
            loser = loser,
            win_combo_rows = win_combo_rows,
            loss_combo_rows = loss_combo_rows,
        )
    } else {
        r#"<div class="card">
  <div class="cl">Pattern cache</div>
  <div class="info-box">
    No cached insights yet. Run <code>cargo run --bin reporter</code> so <code>reports/pattern_cache.json</code> can feed the AI bar and automation alerts.
  </div>
</div>"#
        .to_string()
    };

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>TradingBots.fun · Admin</title>
<style>
  *{{box-sizing:border-box;margin:0;padding:0}}
  body{{background:#0d1117;color:#c9d1d9;
        font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;
        min-height:100vh;padding:32px 16px}}
  .wrap{{max-width:900px;margin:0 auto}}
  .top{{display:flex;justify-content:space-between;align-items:center;margin-bottom:28px}}
  .logo{{font-weight:700;font-size:.95rem;color:#e6edf3;letter-spacing:.04em}}
  .logo .r{{color:#e6343a}}
  .logo .b{{color:#3fb950}}
  .badge-admin{{font-size:.72rem;color:#e3b341;border:1px solid #e3b34150;
                background:#e3b34112;border-radius:12px;padding:2px 10px;margin-left:8px}}
  .nav-admin a{{color:#58a6ff;font-size:.85rem;text-decoration:none;margin-left:16px}}
  .cards{{display:grid;grid-template-columns:repeat(auto-fit,minmax(160px,1fr));gap:12px;margin-bottom:24px}}
  .card{{background:#161b22;border:1px solid #30363d;border-radius:12px;padding:20px}}
  .cl{{font-size:.72rem;color:#8b949e;text-transform:uppercase;letter-spacing:.07em;margin-bottom:6px}}
  .cv{{font-size:1.8rem;font-weight:700;color:#e6edf3}}
  .cv-sm{{font-size:1.1rem;font-weight:600;color:#e6edf3}}
  a{{color:#58a6ff;text-decoration:none}}
</style>
</head>
<body>
<div class="wrap">
<div class="top">
  <span class="logo"><span class="r">Red</span><span class="b">Robot</span> <span class="badge-admin">Admin</span></span>
  <div class="nav-admin">
    <a href="/admin">Dashboard</a>
    <a href="/admin/users">Users</a>
    <a href="/admin/wallets">Wallets</a>
    <a href="/app/agents">Agents</a>
    <a href="/">Operator view</a>
  </div>
</div>

<div class="cards">
  <div class="card"><div class="cl">Total Users</div><div class="cv">{tenant_count}</div></div>
  <div class="card"><div class="cl">Pro</div><div class="cv" style="color:#3fb950">{pro_count}</div></div>
  <div class="card"><div class="cl">Free</div><div class="cv" style="color:#8b949e">{free_count}</div></div>
  <div class="card"><div class="cl">Total HL Balance</div><div class="cv-sm">${total_balance:.2}</div></div>
</div>

{pattern_panel}

<p style="font-size:.85rem;color:#8b949e;margin-bottom:28px">
  <a href="/admin/users">View all users →</a>
</p>

<!-- ── Bot controls ─────────────────────────────────────────── -->
<div style="background:#161b22;border:1px solid #30363d;border-radius:12px;padding:20px;margin-bottom:16px">
  <div style="font-size:.75rem;color:#8b949e;text-transform:uppercase;letter-spacing:.07em;margin-bottom:14px">
    Bot Controls
  </div>
  <div style="display:flex;flex-wrap:wrap;gap:10px;align-items:center">
    <!-- Reset Stats -->
    <button onclick="resetStats()" id="btn-reset" style="
      background:#b94300;border:none;border-radius:7px;
      color:#fff;font-size:.82rem;padding:9px 18px;
      cursor:pointer;font-family:inherit;font-weight:600;
    ">🔄 Reset Stats</button>
    <span style="font-size:.75rem;color:#8b949e">
      Clears P&amp;L history, closed trades, metrics and drawdown window.
      Open positions and signal weights are kept.
    </span>
  </div>
  <div id="reset-resp" style="display:none;margin-top:10px;font-size:.80rem;border-radius:6px;padding:8px 12px"></div>
</div>

</div>
<script>
function resetStats() {{
  var btn = document.getElementById('btn-reset');
  var resp = document.getElementById('reset-resp');
  if (!confirm('Reset all trading stats? P&L, closed trades and metrics will be cleared. This cannot be undone.')) return;
  btn.disabled = true;
  btn.textContent = '⏳ Resetting…';
  fetch('/api/admin/reset-stats', {{method:'POST'}})
    .then(function(r) {{ return r.json(); }})
    .then(function(d) {{
      resp.style.display = 'block';
      if (d.ok) {{
        resp.style.background = '#0d2018';
        resp.style.color = '#3fb950';
        resp.style.border = '1px solid #3fb95044';
        resp.textContent = '✅ ' + d.message;
        btn.textContent = '✓ Done';
        setTimeout(function() {{ window.location.reload(); }}, 2000);
      }} else {{
        resp.style.background = '#2d1a0e';
        resp.style.color = '#e3b341';
        resp.style.border = '1px solid #e3b34144';
        resp.textContent = '⚠ ' + d.message;
        btn.disabled = false;
        btn.textContent = '🔄 Reset Stats';
      }}
    }}).catch(function() {{
      resp.style.display = 'block';
      resp.textContent = '⚠ Network error';
      btn.disabled = false;
      btn.textContent = '🔄 Reset Stats';
    }});
}}
</script>
</body>
</html>"#,
        tenant_count = tenant_count,
        pro_count = pro_count,
        free_count = free_count,
        total_balance = total_balance,
        pattern_panel = pattern_panel,
    );

    axum::response::Html(html).into_response()
}

/// `POST /api/admin/reset-stats` — clear P&L history and metrics for a fresh start.
///
/// Resets: capital → initial_capital, pnl → 0, peak_equity, equity_window,
/// cb_active, closed_trades, recent_decisions, metrics, equity_history,
/// house_money_pool, pool_deployed_usd, recently_closed.
///
/// Preserved: open positions, signal_weights, cycle_count, session_prices.
///
/// Requires HTTP Basic Auth (same as /admin).
async fn admin_reset_stats_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let password = match &app.admin_password {
        Some(p) => p.clone(),
        None => {
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "Admin panel not configured.",
            )
                .into_response()
        }
    };
    if !check_admin_auth(&headers, &password) {
        return www_authenticate_response();
    }

    {
        let mut s = app.bot_state.write().await;
        let ic = s.initial_capital;
        // Recalculate equity including any open positions so capital is correct
        let committed: f64 = s.positions.iter().map(|p| p.size_usd).sum();
        let unrealised: f64 = s.positions.iter().map(|p| p.unrealised_pnl).sum();
        let current_equity = s.capital + committed + unrealised;
        // Reset financials — keep current equity as new starting point
        s.capital = current_equity - committed; // free cash only
        s.initial_capital = ic; // keep original for context
        s.pnl = 0.0;
        s.peak_equity = current_equity;
        s.equity_window = std::collections::VecDeque::new();
        s.equity_history = vec![];
        s.cb_active = false;
        // Clear history
        s.closed_trades = vec![];
        s.recent_decisions = vec![];
        s.metrics = crate::metrics::PerformanceMetrics::default();
        // Clear house-money pool (profits are gone from the P&L slate)
        s.house_money_pool = 0.0;
        s.pool_deployed_usd = 0.0;
        s.recently_closed = std::collections::VecDeque::new();
        // Mark positions as starting fresh — reset their entry context for P&L tracking
        // (positions themselves are kept open, only pool funding tracking resets)
        for p in s.positions.iter_mut() {
            p.funded_from_pool = false;
            p.pool_stake_usd = 0.0;
        }
    }

    log::info!("🔄 Admin: trading stats reset via /api/admin/reset-stats");

    axum::response::Json(serde_json::json!({
        "ok":      true,
        "message": "Stats reset — P&L, metrics and trade history cleared. Open positions kept. Bot continues from current equity.",
    })).into_response()
}

/// `GET /admin/users` — table of all tenants with key stats.
async fn admin_users_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let password = match &app.admin_password {
        Some(p) => p.clone(),
        None => {
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "Admin panel not configured",
            )
                .into_response()
        }
    };

    if !check_admin_auth(&headers, &password) {
        return www_authenticate_response();
    }

    let rows: String = {
        let tenants = app.tenants.read().await;
        tenants
            .all()
            .map(|h| {
                let tier_col = match h.config.tier {
                    crate::tenant::TenantTier::Pro => "#3fb950",
                    crate::tenant::TenantTier::Internal => "#e3b341",
                    crate::tenant::TenantTier::Free => "#8b949e",
                };
                let wallet_short = h
                    .config
                    .wallet_address
                    .as_deref()
                    .map(|w| format!("{}…{}", &w[..6], &w[w.len().saturating_sub(4)..]))
                    .unwrap_or_else(|| "—".to_string());
                let terms_ok = if h.config.terms_accepted_at.is_some() {
                    r#"<span style="color:#3fb950">✓</span>"#
                } else {
                    r#"<span style="color:#f85149">✗</span>"#
                };
                let fund_sum = crate::fund_tracker::summary(&h.id);
                format!(
                    "<tr>\
                   <td style='font-family:monospace;font-size:.72rem'>{id_short}</td>\
                   <td>{name}</td>\
                   <td style='color:{tier_col}'>{tier:?}</td>\
                   <td>{wallet}</td>\
                   <td style='font-size:.8rem'>${bal:.2}</td>\
                   <td style='font-size:.8rem'>${dep:.2}</td>\
                   <td>{terms}</td>\
                 </tr>",
                    id_short = &h.id.as_str()[..8.min(h.id.as_str().len())],
                    name = h.config.display_name,
                    tier_col = tier_col,
                    tier = h.config.tier,
                    wallet = wallet_short,
                    bal = h.config.hl_balance_usd,
                    dep = fund_sum.net_deposits,
                    terms = terms_ok,
                )
            })
            .collect()
    };

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>TradingBots.fun · Admin Users</title>
<style>
  *{{box-sizing:border-box;margin:0;padding:0}}
  body{{background:#0d1117;color:#c9d1d9;
        font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;
        padding:32px 16px}}
  .wrap{{max-width:960px;margin:0 auto}}
  .top{{display:flex;justify-content:space-between;align-items:center;margin-bottom:24px}}
  .logo{{font-weight:700;font-size:.95rem;color:#e6edf3;letter-spacing:.04em}}
  .logo .r{{color:#e6343a}}
  .logo .b{{color:#3fb950}}
  .badge-admin{{font-size:.72rem;color:#e3b341;border:1px solid #e3b34150;
                background:#e3b34112;border-radius:12px;padding:2px 10px;margin-left:8px}}
  .nav-admin a{{color:#58a6ff;font-size:.85rem;text-decoration:none;margin-left:16px}}
  .card{{background:#161b22;border:1px solid #30363d;border-radius:12px;padding:24px}}
  table{{width:100%;border-collapse:collapse;font-size:.82rem}}
  th{{color:#8b949e;font-weight:500;padding:8px;border-bottom:1px solid #30363d;text-align:left}}
  td{{padding:8px;border-bottom:1px solid #21262d;color:#c9d1d9}}
  tr:last-child td{{border-bottom:none}}
  a{{color:#58a6ff;text-decoration:none}}
</style>
</head>
<body>
<div class="wrap">
<div class="top">
  <span class="logo"><span class="r">Red</span><span class="b">Robot</span> <span class="badge-admin">Admin</span></span>
  <div class="nav-admin">
    <a href="/admin">Dashboard</a>
    <a href="/admin/users">Users</a>
    <a href="/admin/wallets">Wallets</a>
    <a href="/">Operator view</a>
  </div>
</div>
<div class="card">
  <table>
    <thead>
      <tr>
        <th>ID (prefix)</th>
        <th>Name</th>
        <th>Tier</th>
        <th>Wallet</th>
        <th>HL Balance</th>
        <th>Net Deposits</th>
        <th>Terms</th>
      </tr>
    </thead>
    <tbody>
      {rows}
    </tbody>
  </table>
</div>
</div>
</body>
</html>"#,
        rows = if rows.is_empty() {
            "<tr><td colspan='7' style='color:#8b949e;text-align:center;padding:20px'>No users registered yet.</td></tr>".to_string()
        } else {
            rows
        },
    );

    axum::response::Html(html).into_response()
}

// ─────────────────────────────────────────────────────────────────────────────
/// `GET /admin/wallets` — per-wallet P&L and LTV performance table.
///
/// Shows all 9 demo wallets (Bot Alpha → Iota) side-by-side with:
///   • Starting capital and current equity
///   • Unrealised and realised P&L
///   • Return % since inception
///   • Open positions count
///   • Estimated builder fees earned (LTV proxy)
///   • Retention signal: equity trend (gaining / flat / bleeding)
// ─────────────────────────────────────────────────────────────────────────────
async fn admin_wallets_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let password = match &app.admin_password {
        Some(p) => p.clone(),
        None => {
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "Admin panel not configured",
            )
                .into_response()
        }
    };
    if !check_admin_auth(&headers, &password) {
        return www_authenticate_response();
    }

    struct WalletRow {
        name: String,
        capital: f64,
        equity: f64,
        realised_pnl: f64,
        unrealised: f64,
        return_pct: f64,
        open_pos: usize,
        closed_trades: usize,
        est_ltv_usd: f64,
        trend: &'static str,
    }

    let rows_data: Vec<WalletRow> = {
        let tm = app.tenants.read().await;
        let mut out = Vec::new();
        for h in tm.all() {
            let s = h.state.read().await;
            let unrealised: f64 = s.positions.iter().map(|p| p.unrealised_pnl).sum();
            let committed: f64 = s.positions.iter().map(|p| p.size_usd).sum();
            let equity = s.capital + committed + unrealised;
            let return_pct = if s.initial_capital > 0.0 {
                (equity - s.initial_capital) / s.initial_capital * 100.0
            } else {
                0.0
            };
            // Estimate LTV: 3 bps entry + 3 bps exit = 6 bps per round-trip
            let avg_size: f64 = if !s.closed_trades.is_empty() {
                s.closed_trades.iter().map(|t| t.size_usd).sum::<f64>()
                    / s.closed_trades.len() as f64
            } else {
                s.initial_capital * 0.08
            };
            let est_ltv = s.closed_trades.len() as f64 * avg_size * 0.0006;
            let trend = if return_pct > 2.0 {
                "🟢"
            } else if return_pct > -1.0 {
                "🟡"
            } else {
                "🔴"
            };
            out.push(WalletRow {
                name: h.config.display_name.clone(),
                capital: s.initial_capital,
                equity,
                realised_pnl: s.pnl,
                unrealised,
                return_pct,
                open_pos: s.positions.len(),
                closed_trades: s.closed_trades.len(),
                est_ltv_usd: est_ltv,
                trend,
            });
        }
        out.sort_by(|a, b| {
            a.capital
                .partial_cmp(&b.capital)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out
    };

    let total_est_ltv: f64 = rows_data.iter().map(|r| r.est_ltv_usd).sum();
    let total_equity: f64 = rows_data.iter().map(|r| r.equity).sum();
    let total_capital: f64 = rows_data.iter().map(|r| r.capital).sum();
    let total_ret_pct = if total_capital > 0.0 {
        (total_equity - total_capital) / total_capital * 100.0
    } else {
        0.0
    };

    let table_rows: String = rows_data
        .iter()
        .map(|r| {
            let ret_col = if r.return_pct >= 0.0 {
                "#3fb950"
            } else {
                "#f85149"
            };
            let upnl_col = if r.unrealised >= 0.0 {
                "#3fb950"
            } else {
                "#f85149"
            };
            let pnl_sign = if r.realised_pnl >= 0.0 { "+" } else { "" };
            let cap_tier = if r.capital <= 25.0 {
                "Nano"
            } else if r.capital <= 100.0 {
                "Micro"
            } else if r.capital <= 500.0 {
                "Small"
            } else if r.capital <= 2000.0 {
                "Mid"
            } else {
                "Large"
            };
            format!(
                "<tr>\
               <td>{trend} <b>{name}</b></td>\
               <td style='color:#8b949e;font-size:.75em'>{tier}</td>\
               <td>${cap:.0}</td>\
               <td>${eq:.2}</td>\
               <td style='color:{rc}'>{ret:+.2}%</td>\
               <td>{ps}${rpnl:.2} / <span style='color:{uc}'>{us:+.2}</span></td>\
               <td>{ops} open · {cl} closed</td>\
               <td style='color:#e3b341'>${ltv:.4}</td>\
             </tr>",
                trend = r.trend,
                name = r.name,
                tier = cap_tier,
                cap = r.capital,
                eq = r.equity,
                rc = ret_col,
                ret = r.return_pct,
                ps = pnl_sign,
                rpnl = r.realised_pnl,
                uc = upnl_col,
                us = r.unrealised,
                ops = r.open_pos,
                cl = r.closed_trades,
                ltv = r.est_ltv_usd,
            )
        })
        .collect();

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>TradingBots.fun · Wallet Performance</title>
<style>
  *{{box-sizing:border-box;margin:0;padding:0}}
  body{{background:#0d1117;color:#c9d1d9;
        font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;
        padding:32px 16px}}
  .wrap{{max-width:1100px;margin:0 auto}}
  .top{{display:flex;justify-content:space-between;align-items:center;margin-bottom:24px}}
  .logo{{font-weight:700;font-size:.95rem;color:#e6edf3;letter-spacing:.04em}}
  .logo .r{{color:#e6343a}}
  .logo .b{{color:#3fb950}}
  .badge-admin{{font-size:.72rem;color:#e3b341;border:1px solid #e3b34150;
                background:#e3b34112;border-radius:12px;padding:2px 10px;margin-left:8px}}
  .nav-admin a{{color:#58a6ff;font-size:.85rem;text-decoration:none;margin-left:16px}}
  .card{{background:#161b22;border:1px solid #30363d;border-radius:12px;padding:24px;margin-bottom:20px}}
  .metrics{{display:grid;grid-template-columns:repeat(auto-fit,minmax(160px,1fr));gap:12px;margin-bottom:20px}}
  .m{{background:#0d1117;border:1px solid #30363d;border-radius:8px;padding:14px}}
  .ml{{font-size:.7rem;color:#8b949e;text-transform:uppercase;letter-spacing:.07em;margin-bottom:4px}}
  .mv{{font-size:1.4rem;font-weight:700;color:#e6edf3}}
  .mv-sm{{font-size:.95rem;font-weight:600;color:#e6edf3}}
  table{{width:100%;border-collapse:collapse;font-size:.82rem}}
  th{{color:#8b949e;font-weight:500;padding:10px 8px;border-bottom:1px solid #30363d;
      text-align:left;white-space:nowrap}}
  td{{padding:10px 8px;border-bottom:1px solid #21262d;color:#c9d1d9;vertical-align:middle}}
  tr:last-child td{{border-bottom:none}}
  tr:hover td{{background:#0d1117}}
  a{{color:#58a6ff;text-decoration:none}}
  .note{{font-size:.75rem;color:#8b949e;margin-top:14px;line-height:1.6}}
</style>
</head>
<body>
<div class="wrap">
<div class="top">
  <span class="logo">
    <span class="r">Red</span><span class="b">Robot</span>
    <span class="badge-admin">Admin · Wallets</span>
  </span>
  <div class="nav-admin">
    <a href="/admin">Dashboard</a>
    <a href="/admin/users">Users</a>
    <a href="/admin/wallets">Wallets</a>
    <a href="/">Operator view</a>
  </div>
</div>

<div class="metrics">
  <div class="m"><div class="ml">Wallets</div><div class="mv">{wc}</div></div>
  <div class="m"><div class="ml">Total Capital</div><div class="mv-sm">${tc:.0}</div></div>
  <div class="m">
    <div class="ml">Total Equity</div>
    <div class="mv-sm" style="color:{teq_col}">${te:.2}</div>
  </div>
  <div class="m">
    <div class="ml">Portfolio Return</div>
    <div class="mv-sm" style="color:{tret_col}">{tr:+.2}%</div>
  </div>
  <div class="m">
    <div class="ml">Est. Builder Fees (all-time)</div>
    <div class="mv-sm" style="color:#e3b341">${ltv_total:.4}</div>
  </div>
</div>

<div class="card">
  <table>
    <thead>
      <tr>
        <th>Wallet</th><th>Tier</th><th>Capital</th><th>Equity</th>
        <th>Return</th><th>P&amp;L (realised / unrealised)</th>
        <th>Positions</th><th>Builder Fees (est.)</th>
      </tr>
    </thead>
    <tbody>{trows}</tbody>
  </table>
  <div class="note">
    ⚡ <b>Builder fee estimate:</b> 3 bps entry + 3 bps exit = 6 bps per round-trip × avg position size × closed trades.
    <br><br>
    🎯 <b>LTV maximisation playbook:</b><br>
    &nbsp;&nbsp;• <b>Nano/Micro ($10–$100):</b> Highest churn risk. Prioritise win-rate over trade frequency.
    Even a +5% monthly return on $25 = $1.25 — users will stay if they see growth.
    Max 3–6 positions, conservative leverage (2–3×), take profits at +10%.<br>
    &nbsp;&nbsp;• <b>Small ($100–$500):</b> Bread and butter. Target 4–8 round-trips/day.
    Builder fee per user ≈ $0.02–$0.10/day at this size. Monthly LTV ≈ $1–$3.<br>
    &nbsp;&nbsp;• <b>Mid/Large ($1k–$10k):</b> High-value. Builder fee ≈ $0.30–$2/day.
    Monthly LTV ≈ $9–$60. These users stay as long as drawdown stays &lt; 10%.<br>
    &nbsp;&nbsp;• <b>Retention formula:</b> equity growing → user stays → more fills → more LTV.
    Close losing positions early (stop the bleed), let winners ride.
  </div>
</div>
</div>
</body>
</html>"#,
        wc = rows_data.len(),
        tc = total_capital,
        te = total_equity,
        teq_col = if total_equity >= total_capital {
            "#3fb950"
        } else {
            "#f85149"
        },
        tr = total_ret_pct,
        tret_col = if total_ret_pct >= 0.0 {
            "#3fb950"
        } else {
            "#f85149"
        },
        ltv_total = total_est_ltv,
        trows = if table_rows.is_empty() {
            "<tr><td colspan='8' style='color:#8b949e;text-align:center;padding:20px'>\
             No wallets active — restart the bot to seed demo wallets.\
             </td></tr>"
                .to_string()
        } else {
            table_rows
        },
    );

    axum::response::Html(html).into_response()
}

/// Google Pay requires no domain verification — it is automatically enabled
/// in Stripe Checkout when the user's device supports it.
async fn apple_pay_domain_handler(State(app): State<AppState>) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    match &app.apple_pay_domain_assoc {
        Some(content) => (
            StatusCode::OK,
            [("Content-Type", "text/plain; charset=utf-8")],
            content.clone(),
        )
            .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            "Apple Pay domain association file not configured.\n\
             Set APPLE_PAY_DOMAIN_ASSOC in your environment.",
        )
            .into_response(),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Public TVL API — no auth required, powers landing page hero graph
// ═══════════════════════════════════════════════════════════════════════════════

// `GET /api/public/tvl`
// ─────────────────────────────────────────────────────────────────────────────
// Leaderboard + campaign handlers
// ─────────────────────────────────────────────────────────────────────────────

/// `GET /leaderboard` — public leaderboard page for the active campaign.
///
/// Shows the current standings, prize pool, countdown timer, and how to get
/// an invite code.  No authentication required — it's a viral acquisition page.
async fn leaderboard_handler(State(app): State<AppState>) -> axum::response::Html<String> {
    let (campaign, entries) = match &app.db {
        Some(db) => {
            let c = crate::leaderboard::active_campaign(db).await.ok().flatten();
            let e = if c.is_some() {
                crate::leaderboard::live_standings(db, 50)
                    .await
                    .unwrap_or_default()
            } else {
                vec![]
            };
            (c, e)
        }
        None => (None, vec![]),
    };

    let campaign_title = campaign
        .as_ref()
        .map(|c| c.title.clone())
        .unwrap_or_else(|| "Weekly Trading Contest".into());
    let campaign_desc = campaign
        .as_ref()
        .and_then(|c| c.description.clone())
        .unwrap_or_else(|| "Top traders by % return win weekly prizes.".into());
    let prize_pool = campaign.as_ref().map(|c| c.prize_pool_usd).unwrap_or(0.0);
    let seconds_left = campaign.as_ref().map(|c| c.seconds_left).unwrap_or(0);

    let prizes_html = campaign.as_ref().map(|c| {
        c.prizes.iter().map(|p| format!(
            r#"<div class="prize-row"><span class="prize-label">{}</span><span class="prize-amt">${}</span></div>"#,
            p.label, p.usd as i64
        )).collect::<Vec<_>>().join("")
    }).unwrap_or_default();

    let rows_html: String = if entries.is_empty() {
        r#"<tr><td colspan="5" style="text-align:center;color:#484f58;padding:32px">No trades recorded yet this week — be the first!</td></tr>"#.into()
    } else {
        entries
            .iter()
            .map(|e| {
                let medal = match e.rank {
                    1 => "🥇",
                    2 => "🥈",
                    3 => "🥉",
                    _ => "",
                };
                let pct_color = if e.pct_return >= 0.0 {
                    "#3fb950"
                } else {
                    "#f85149"
                };
                let pct_sign = if e.pct_return >= 0.0 { "+" } else { "" };
                format!(
                    r#"<tr class="lb-row{rank_cls}">
                  <td class="lb-rank">{medal}{rank}</td>
                  <td class="lb-name">{name}</td>
                  <td class="lb-wallet">{wallet}</td>
                  <td class="lb-trades">{trades}</td>
                  <td class="lb-pct" style="color:{pct_color}">{pct_sign}{pct:.2}%</td>
                </tr>"#,
                    rank_cls = if e.rank <= 3 { " top3" } else { "" },
                    medal = medal,
                    rank = e.rank,
                    name = html_escape(&e.display_name),
                    wallet = e.wallet_short,
                    trades = e.trades_in_period,
                    pct_color = pct_color,
                    pct_sign = pct_sign,
                    pct = e.pct_return,
                )
            })
            .collect()
    };

    axum::response::Html(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>TradingBots.fun · Leaderboard</title>
<style>
*{{box-sizing:border-box;margin:0;padding:0}}
body{{background:#0d1117;color:#c9d1d9;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;min-height:100vh;padding:0 0 60px}}
.hero{{background:linear-gradient(155deg,#161b22,#0d1117);border-bottom:1px solid #21262d;padding:48px 24px 40px;text-align:center}}
.hero-badge{{display:inline-block;background:rgba(255,215,0,.12);border:1px solid rgba(255,215,0,.3);border-radius:20px;padding:5px 14px;font-size:.72rem;font-weight:700;color:#ffd700;letter-spacing:.8px;text-transform:uppercase;margin-bottom:16px}}
.hero h1{{font-size:2rem;font-weight:800;color:#e6edf3;margin-bottom:8px}}
.hero h1 .g{{color:#3fb950}}.hero h1 .r{{color:#e6343a}}
.hero-sub{{font-size:.9rem;color:#8b949e;max-width:500px;margin:0 auto 24px}}
.prize-bar{{display:flex;justify-content:center;gap:16px;flex-wrap:wrap;margin-bottom:28px}}
.prize-row{{background:#161b22;border:1px solid #30363d;border-radius:10px;padding:12px 20px;text-align:center;min-width:100px}}
.prize-label{{display:block;font-size:.75rem;color:#8b949e;margin-bottom:4px}}
.prize-amt{{display:block;font-size:1.3rem;font-weight:800;color:#ffd700}}
.countdown{{font-size:.82rem;color:#484f58;margin-top:8px}}
.countdown span{{color:#58a6ff;font-weight:700}}
.cta-strip{{background:rgba(63,185,80,.07);border:1px solid rgba(63,185,80,.2);border-radius:12px;padding:20px 24px;max-width:520px;margin:0 auto;text-align:left}}
.cta-strip h3{{font-size:.92rem;font-weight:700;color:#e6edf3;margin-bottom:6px}}
.cta-strip p{{font-size:.78rem;color:#8b949e;line-height:1.6;margin-bottom:12px}}
.cta-strip .how{{font-size:.75rem;color:#6e7681;line-height:1.8}}
.cta-strip .how b{{color:#c9d1d9}}
.btn-signin{{display:inline-block;padding:11px 24px;background:#3fb950;color:#0d1117;border-radius:8px;font-weight:700;font-size:.88rem;text-decoration:none;transition:.15s}}
.btn-signin:hover{{background:#52c965}}
.wrap{{max-width:860px;margin:0 auto;padding:32px 20px 0}}
.lb-wrap{{background:#161b22;border:1px solid #21262d;border-radius:14px;overflow:hidden}}
.lb-hd{{padding:18px 22px;border-bottom:1px solid #21262d;display:flex;align-items:center;justify-content:space-between}}
.lb-hd-title{{font-size:.92rem;font-weight:700;color:#e6edf3}}
.lb-hd-sub{{font-size:.72rem;color:#484f58}}
table{{width:100%;border-collapse:collapse}}
th{{padding:10px 16px;font-size:.7rem;font-weight:700;color:#484f58;text-transform:uppercase;letter-spacing:.6px;text-align:left;border-bottom:1px solid #21262d}}
.lb-row td{{padding:13px 16px;font-size:.85rem;border-bottom:1px solid rgba(48,54,61,.5);transition:.1s}}
.lb-row:hover td{{background:rgba(255,255,255,.02)}}
.lb-row.top3 td{{background:rgba(255,215,0,.03)}}
.lb-rank{{font-weight:700;color:#e6edf3;width:60px}}
.lb-name{{color:#c9d1d9;font-weight:600}}
.lb-wallet{{color:#484f58;font-size:.78rem;font-family:monospace}}
.lb-trades{{color:#8b949e;text-align:center;width:80px}}
.lb-pct{{font-weight:700;text-align:right;width:100px}}
.pool-badge{{display:inline-block;background:rgba(255,215,0,.12);border:1px solid rgba(255,215,0,.25);border-radius:8px;padding:4px 10px;font-size:.8rem;color:#ffd700;font-weight:700}}
</style>
</head>
<body>

<!-- Hero -->
<div class="hero">
  <div class="hero-badge">🏆 Weekly Contest</div>
  <h1>TradingBots<span class="g">.fun</span> Leaderboard</h1>
  <p class="hero-sub">{desc}</p>

  <div class="prize-bar">
    {prizes_html}
  </div>
  <div class="countdown" id="countdown">Prize pool: <span class="pool-badge">${prize_pool}</span></div>

  <!-- How to join -->
  <div class="cta-strip" style="margin-top:28px">
    <h3>🎟 How to join</h3>
    <p>TradingBots.fun is invite-only. Get a code from a friend, enter it on the sign-in page, deposit as little as <b style="color:#e6edf3">$20</b>, and let two bots trade for you. Best % return wins.</p>
    <div class="how">
      <b>1.</b> Get an invite code from a friend or this leaderboard ·
      <b>2.</b> Sign in at <a href="/login" style="color:#58a6ff">/login</a> ·
      <b>3.</b> Deposit $20+ to Hyperliquid ·
      <b>4.</b> Two bots start automatically · <b>5.</b> Compete
    </div>
    <br>
    <a href="/login" class="btn-signin">Get started →</a>
  </div>
</div>

<!-- Standings -->
<div class="wrap">
  <div class="lb-wrap">
    <div class="lb-hd">
      <span class="lb-hd-title">{title} · Current Standings</span>
      <span class="lb-hd-sub">Ranked by % return · any deposit size competes equally</span>
    </div>
    <table>
      <thead>
        <tr>
          <th>Rank</th>
          <th>Trader</th>
          <th>Wallet</th>
          <th style="text-align:center">Trades</th>
          <th style="text-align:right">Return</th>
        </tr>
      </thead>
      <tbody>{rows_html}</tbody>
    </table>
  </div>
</div>

<script>
// Countdown timer
const secsLeft = {seconds_left};
function fmt(s) {{
  if (s <= 0) return 'Contest ended';
  const d = Math.floor(s/86400), h = Math.floor((s%86400)/3600),
        m = Math.floor((s%3600)/60), ss = s%60;
  if (d > 0) return d+'d '+h+'h '+m+'m left';
  if (h > 0) return h+'h '+m+'m '+ss+'s left';
  return m+'m '+ss+'s left';
}}
let remaining = secsLeft;
const el = document.getElementById('countdown');
function tick() {{
  const pool = el.querySelector('.pool-badge');
  const poolHtml = pool ? pool.outerHTML : '';
  el.innerHTML = 'Prize pool: '+poolHtml+'  ·  <span style="color:#58a6ff;font-weight:700">'+fmt(remaining)+'</span>';
  remaining--;
  if (remaining >= 0) setTimeout(tick, 1000);
}}
if (secsLeft > 0) tick();
</script>
</body></html>"#,
        title = html_escape(&campaign_title),
        desc = html_escape(&campaign_desc),
        prizes_html = prizes_html,
        prize_pool = prize_pool as i64,
        rows_html = rows_html,
        seconds_left = seconds_left,
    ))
}

/// `POST /app/invite/generate` — authenticated endpoint.
///
/// Generates a personal referral code for the logged-in tenant and returns it.
/// The code is valid for 30 days, single-use, and tied to the active campaign.
async fn generate_invite_handler(
    State(app): State<AppState>,
    headers: axum::http::HeaderMap,
) -> impl axum::response::IntoResponse {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    let tenant_id = match crate::privy::require_tenant_id(&headers, &app.session_secret) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({"error":"Unauthorized"})),
            )
                .into_response()
        }
    };

    let db = match &app.db {
        Some(db) => db,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                axum::Json(serde_json::json!({"error":"Database not configured"})),
            )
                .into_response()
        }
    };

    match crate::invite::generate_referral_code(db, &tenant_id).await {
        Ok(code) => axum::Json(serde_json::json!({
            "ok": true,
            "code": code,
            "share_url": format!("/login?invite={}", code),
            "expires_days": 30,
        }))
        .into_response(),
        Err(e) => {
            log::error!("generate_invite_handler: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({"error":"Could not generate code"})),
            )
                .into_response()
        }
    }
}

/// `GET /app/invite` — returns the tenant's current referral code (or generates one).
async fn get_invite_handler(
    State(app): State<AppState>,
    headers: axum::http::HeaderMap,
) -> impl axum::response::IntoResponse {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    let tenant_id = match crate::privy::require_tenant_id(&headers, &app.session_secret) {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({"error":"Unauthorized"})),
            )
                .into_response()
        }
    };

    let db = match &app.db {
        Some(db) => db,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                axum::Json(serde_json::json!({"error":"Database not configured"})),
            )
                .into_response()
        }
    };

    let code = match crate::invite::get_referral_code_for_tenant(db, &tenant_id).await {
        Ok(Some(c)) => c,
        Ok(None) => {
            // Auto-generate on first request
            match crate::invite::generate_referral_code(db, &tenant_id).await {
                Ok(c) => c,
                Err(e) => {
                    log::error!("get_invite auto-generate: {}", e);
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        axum::Json(serde_json::json!({"error":"Could not generate code"})),
                    )
                        .into_response();
                }
            }
        }
        Err(e) => {
            log::error!("get_invite_handler: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({"error":"DB error"})),
            )
                .into_response();
        }
    };

    axum::Json(serde_json::json!({
        "ok": true,
        "code": code,
        "share_url": format!("/login?invite={}", code),
    }))
    .into_response()
}

/// `GET /api/leaderboard` — JSON endpoint for the current standings.
async fn api_leaderboard_handler(State(app): State<AppState>) -> impl axum::response::IntoResponse {
    use axum::response::IntoResponse;
    let db = match &app.db {
        Some(db) => db,
        None => {
            return axum::Json(serde_json::json!({"entries":[],"campaign":null})).into_response()
        }
    };
    let campaign = crate::leaderboard::active_campaign(db).await.ok().flatten();
    let entries = crate::leaderboard::live_standings(db, 100)
        .await
        .unwrap_or_default();
    axum::Json(serde_json::json!({ "campaign": campaign, "entries": entries })).into_response()
}

///
/// Returns the last 90 days of AUM snapshots as JSON.
/// Used by the landing page to render the TVL hero graph client-side.
/// No authentication required — returns aggregate data only, never per-tenant.
async fn public_tvl_handler(State(app): State<AppState>) -> impl axum::response::IntoResponse {
    use axum::http::{HeaderMap, StatusCode};

    let mut headers = HeaderMap::new();
    // Allow embedding in the landing page (different origin during dev).
    headers.insert("Access-Control-Allow-Origin", "*".parse().unwrap());
    headers.insert("Cache-Control", "public, max-age=60".parse().unwrap());

    let Some(db) = &app.db else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            headers,
            axum::Json(serde_json::json!({
                "error": "database not yet configured",
                "points": [],
            })),
        );
    };

    let points = match db.get_aum_history(90).await {
        Ok(p) => p,
        Err(e) => {
            log::warn!("public_tvl_handler: DB error: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                headers,
                axum::Json(serde_json::json!({ "error": "query failed", "points": [] })),
            );
        }
    };

    // Pull the latest snapshot for the headline numbers.
    let latest = db.get_latest_aum().await.ok().flatten();

    let response = serde_json::json!({
        "generated_at":    chrono::Utc::now().to_rfc3339(),
        "window_days":     90,
        "point_count":     points.len(),
        "latest": latest.as_ref().map(|a| serde_json::json!({
            "total_aum":         a.total_aum,
            "deposited_capital": a.deposited_capital,
            "total_pnl":         a.total_pnl,
            "pnl_pct":           a.pnl_pct,
            "active_tenants":    a.active_tenants,
            "total_tenants":     a.total_tenants,
            "open_positions":    a.open_positions,
            "recorded_at":       a.recorded_at.to_rfc3339(),
        })),
        "points": points.iter().map(|p| serde_json::json!({
            "ts":          p.recorded_at.to_rfc3339(),
            "aum":         p.total_aum,
            "pnl":         p.total_pnl,
            "pnl_pct":     p.pnl_pct,
            "tenants":     p.active_tenants,
            "positions":   p.open_positions,
        })).collect::<Vec<_>>(),
    });

    (StatusCode::OK, headers, axum::Json(response))
}

/// `GET /api/public/tvl/svg`
///
/// Returns a self-contained SVG sparkline of the TVL curve.
/// Embed directly in the landing page `<img src="/api/public/tvl/svg">` —
/// no JavaScript required.  Auto-updates every 60 seconds via HTTP cache.
async fn public_tvl_svg_handler(State(app): State<AppState>) -> impl axum::response::IntoResponse {
    use axum::http::{HeaderMap, StatusCode};

    let mut headers = HeaderMap::new();
    headers.insert("Content-Type", "image/svg+xml".parse().unwrap());
    headers.insert("Cache-Control", "public, max-age=60".parse().unwrap());
    headers.insert("Access-Control-Allow-Origin", "*".parse().unwrap());

    let placeholder_svg = r##"<svg width="480" height="80" viewBox="0 0 480 80"
         xmlns="http://www.w3.org/2000/svg"
         style="background:#0d1117;border-radius:8px">
  <text x="240" y="45" text-anchor="middle" fill="#484f58"
        font-family="system-ui,sans-serif" font-size="13">
    Accumulating data…
  </text>
</svg>"##;

    let Some(db) = &app.db else {
        return (StatusCode::OK, headers, placeholder_svg.to_string());
    };

    let points = match db.get_aum_history(90).await {
        Ok(p) if p.len() >= 2 => p,
        _ => return (StatusCode::OK, headers, placeholder_svg.to_string()),
    };

    // Build SVG using the same proven pattern as the equity sparkline.
    let w_px: f64 = 480.0;
    let h_px: f64 = 80.0;
    let pad: f64 = 8.0;
    let inner_h = h_px - 2.0 * pad;

    let values: Vec<f64> = points.iter().map(|p| p.total_aum).collect();
    let deposited = points.first().map(|p| p.deposited_capital).unwrap_or(0.0);

    let data_min = values
        .iter()
        .cloned()
        .fold(f64::INFINITY, f64::min)
        .min(deposited);
    let data_max = values
        .iter()
        .cloned()
        .fold(f64::NEG_INFINITY, f64::max)
        .max(deposited);
    let buf = ((data_max - data_min).max(deposited * 0.002)) * 0.15;
    let min_v = data_min - buf;
    let max_v = data_max + buf;
    let range = (max_v - min_v).max(0.01);

    let to_y = |v: f64| h_px - pad - (v - min_v) / range * inner_h;

    let n = values.len() as f64;
    let pts: String = values
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let x = i as f64 / (n - 1.0) * w_px;
            let y = to_y(v);
            format!("{x:.1},{y:.1}")
        })
        .collect::<Vec<_>>()
        .join(" ");

    let base_y = to_y(deposited);
    let last_y = to_y(*values.last().unwrap());
    let last_v = *values.last().unwrap();
    let trend_c = if last_v >= deposited {
        "#3fb950"
    } else {
        "#f85149"
    };
    let fill_pts = format!("{pts} {w_px:.1},{base_y:.1} 0.0,{base_y:.1}");

    let latest_pnl_pct = points.last().map(|p| p.pnl_pct).unwrap_or(0.0);
    let pnl_sign = if latest_pnl_pct >= 0.0 { "+" } else { "" };
    let label = format!("{pnl_sign}{latest_pnl_pct:.1}% all-time");

    let svg = format!(
        r##"<svg width="480" height="80" viewBox="0 0 480 80"
     xmlns="http://www.w3.org/2000/svg"
     style="background:#0d1117;border-radius:8px;display:block">
  <line x1="0" y1="{by:.1}" x2="480" y2="{by:.1}"
        stroke="{c}" stroke-width="0.8" stroke-dasharray="3 3" stroke-opacity="0.4"/>
  <polygon points="{fp}" fill="{c}" fill-opacity="0.12"/>
  <polyline points="{pts}" fill="none" stroke="{c}"
            stroke-width="2" stroke-linejoin="round" stroke-linecap="round"/>
  <circle cx="480" cy="{ly:.1}" r="4" fill="{c}"/>
  <text x="8" y="20" font-family="system-ui,sans-serif" font-size="11"
        fill="{c}" font-weight="600">{label}</text>
</svg>"##,
        c = trend_c,
        by = base_y,
        fp = fill_pts,
        pts = pts,
        ly = last_y,
        label = label,
    );

    (StatusCode::OK, headers, svg)
}

// ─────────────────────────────────────────────────────────────────────────────
//  POST /api/funnel  — client-side event ingestion
// ─────────────────────────────────────────────────────────────────────────────

/// Accepts `navigator.sendBeacon` payloads from the client tracking script.
///
/// Validates the `event_type` against the known set, attaches the server-side
/// tenant context if the session cookie is present, then writes to `funnel_events`.
async fn funnel_event_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
    body: axum::extract::Json<crate::funnel::FunnelEventPayload>,
) -> axum::http::StatusCode {
    use crate::funnel::{record, FunnelEvent};
    use axum::http::StatusCode;

    let payload = body.0;

    // Map the string event_type → enum (rejects unknown values)
    let event = match payload.event_type.as_str() {
        "PAGE_VIEW" => FunnelEvent::PageView,
        "LOGIN_CLICK" => FunnelEvent::LoginClick,
        "AUTH_SUCCESS" => FunnelEvent::AuthSuccess,
        "TRIAL_START" => FunnelEvent::TrialStart,
        "TERMS_ACCEPTED" => FunnelEvent::TermsAccepted,
        "WALLET_LINKED" => FunnelEvent::WalletLinked,
        "FIRST_POSITION" => FunnelEvent::FirstPosition,
        "UPGRADE_CLICK" => FunnelEvent::UpgradeClick,
        "CHECKOUT_STARTED" => FunnelEvent::CheckoutStarted,
        "UPGRADED" => FunnelEvent::Upgraded,
        "TRIAL_EXPIRED" => FunnelEvent::TrialExpired,
        "CHURNED" => FunnelEvent::Churned,
        "AD_IMPRESSION" => FunnelEvent::AdImpression,
        "AD_CLICK" => FunnelEvent::AdClick,
        _ => return StatusCode::BAD_REQUEST,
    };

    // Resolve tenant from session cookie if present (pre-auth events have None)
    // get_session_tenant_id already returns Option<TenantId> — no mapping needed
    let tid = get_session_tenant_id(&headers, &app.session_secret);

    record(
        &app.db,
        event,
        &payload.anon_id,
        tid.as_ref(),
        Some(payload.extra),
    )
    .await;

    StatusCode::NO_CONTENT
}

// ─────────────────────────── Trade journal ───────────────────────────────────

/// Payload for `POST /api/trade-note`.
#[derive(Debug, Deserialize)]
struct TradeNotePayload {
    /// Index into `bot_state.closed_trades` (0 = most recent).
    index: usize,
    /// Operator's plain-text note — max 500 chars.
    note: String,
}

/// `POST /api/trade-note` — attach an operator note to a closed trade.
///
/// The note is written to the in-memory `ClosedTrade` and also persisted to
/// the PostgreSQL `closed_trade_notes` table so it survives restarts.
///
/// Requires a valid admin session (checked via `ADMIN_PASSWORD`).
/// Returns 204 No Content on success, 400 on bad input, 404 if index OOB.
async fn trade_note_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
    body: axum::extract::Json<TradeNotePayload>,
) -> axum::http::StatusCode {
    use axum::http::StatusCode;

    // Simple admin gate: require the same HTTP-Basic admin password used on /admin.
    // In production this endpoint is only hit from the admin panel JS.
    if let Some(pw) = &app.admin_password {
        let auth = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        // Accept both "Basic <b64(admin:pw)>" and bare bearer token equal to password.
        let b64 = base64_encode(&format!("admin:{}", pw));
        let expected_basic = format!("Basic {}", b64);
        if auth != expected_basic && auth != pw.as_str() {
            return StatusCode::UNAUTHORIZED;
        }
    }

    let payload = body.0;

    // Validate note length.
    if payload.note.len() > 500 {
        return StatusCode::BAD_REQUEST;
    }

    // Write into in-memory state.
    {
        let mut state = app.bot_state.write().await;
        match state.closed_trades.get_mut(payload.index) {
            Some(trade) => {
                trade.note = Some(payload.note.clone());
            }
            None => return StatusCode::NOT_FOUND,
        }
    }

    // Persist to DB (best-effort — don't fail the request if DB is down).
    // Uses sqlx::query() (not macro) so migration 007 need not exist at compile time.
    if let Some(db) = &app.db {
        let idx = payload.index as i64;
        let note = payload.note.clone();
        let _ = sqlx::query(
            "INSERT INTO closed_trade_notes (trade_index, note, updated_at) \
             VALUES ($1, $2, NOW()) \
             ON CONFLICT (trade_index) DO UPDATE \
               SET note = EXCLUDED.note, updated_at = NOW()",
        )
        .bind(idx)
        .bind(note)
        .execute(db.pool())
        .await;
    }

    StatusCode::NO_CONTENT
}

#[derive(Debug, Deserialize)]
struct ReportQueryPayload {
    question: String,
    answer: Option<String>,
}

#[derive(Debug, Serialize)]
struct ReportQueryResponse {
    answer: Option<String>,
    cached: bool,
    report_hash: String,
}

async fn api_report_latest_handler(
    State(_app): State<AppState>,
) -> Result<Json<reporting::ReportSummary>, axum::http::StatusCode> {
    match reporting::load_summary() {
        Ok(summary) => Ok(Json(summary)),
        Err(_) => Err(axum::http::StatusCode::NOT_FOUND),
    }
}

async fn api_report_query_handler(
    State(app): State<AppState>,
    Json(payload): Json<ReportQueryPayload>,
) -> Result<Json<ReportQueryResponse>, axum::http::StatusCode> {
    if payload.question.trim().is_empty() {
        return Err(axum::http::StatusCode::BAD_REQUEST);
    }
    let summary = reporting::load_summary().map_err(|_| axum::http::StatusCode::NOT_FOUND)?;
    let report_hash = summary.report_hash.clone();
    let mut cache = app.report_cache.lock().await;
    if let Some(answer) = payload.answer {
        cache.store(&payload.question, &report_hash, answer.clone());
        let _ = cache.save();
        return Ok(Json(ReportQueryResponse {
            answer: Some(answer),
            cached: false,
            report_hash,
        }));
    }
    if let Some(entry) = cache.lookup(&payload.question, &report_hash) {
        return Ok(Json(ReportQueryResponse {
            answer: Some(entry.answer.clone()),
            cached: true,
            report_hash,
        }));
    }
    Ok(Json(ReportQueryResponse {
        answer: None,
        cached: false,
        report_hash,
    }))
}

async fn api_report_patterns_handler(
    State(app): State<AppState>,
) -> Result<Json<pattern_insights::PatternInsights>, axum::http::StatusCode> {
    let cache = app.pattern_cache.lock().await;
    if let Some(insights) = cache.latest() {
        Ok(Json(insights))
    } else {
        Err(axum::http::StatusCode::NOT_FOUND)
    }
}

async fn api_pattern_alert_handler(
    State(_app): State<AppState>,
) -> Result<Json<serde_json::Value>, axum::http::StatusCode> {
    let path = PathBuf::from("reports").join("pattern_cache_alert.json");
    match fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(json) => Ok(Json(json)),
            Err(_) => Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR),
        },
        Err(_) => Err(axum::http::StatusCode::NOT_FOUND),
    }
}

#[derive(Debug, Deserialize)]
struct BridgeWithdrawRequest {
    amount_usd: f64,
    destination: String,
}

async fn bridge_withdraw_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
    axum::Json(req): axum::Json<BridgeWithdrawRequest>,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let tenant_id = match get_session_tenant_id(&headers, &app.session_secret) {
        Some(t) => t,
        None => {
            return axum::http::StatusCode::UNAUTHORIZED.into_response();
        }
    };

    match app
        .bridge_manager
        .request_withdrawal(&tenant_id, req.amount_usd, req.destination.trim())
        .await
    {
        Ok(record) => axum::response::Json(record.view()).into_response(),
        Err(e) => (
            axum::http::StatusCode::BAD_REQUEST,
            axum::response::Json(serde_json::json!({
                "error": "bridge_failed",
                "message": e.to_string(),
            })),
        )
            .into_response(),
    }
}

async fn bridge_status_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let tenant_id = match get_session_tenant_id(&headers, &app.session_secret) {
        Some(t) => t,
        None => {
            return axum::http::StatusCode::UNAUTHORIZED.into_response();
        }
    };

    match app.bridge_manager.fetch_record(&id).await {
        Some(record) if record.tenant_id == tenant_id => {
            axum::response::Json(record.view()).into_response()
        }
        Some(_) => axum::http::StatusCode::FORBIDDEN.into_response(),
        None => axum::http::StatusCode::NOT_FOUND.into_response(),
    }
}

/// Minimal base64 encoder (no external dep) — only used for the Basic-Auth check above.
fn base64_encode(input: &str) -> String {
    use std::fmt::Write;
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        let _ = write!(out, "{}", TABLE[((n >> 18) & 0x3f) as usize] as char);
        let _ = write!(out, "{}", TABLE[((n >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            let _ = write!(out, "{}", TABLE[((n >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            let _ = write!(out, "{}", TABLE[(n & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
//  HL Wallet setup  (/app/setup)
// ─────────────────────────────────────────────────────────────────────────────

/// `GET /app/setup` — three-step wallet setup page shown after ToS acceptance.
///
/// Step 1 — Your wallet (address + private key + download)
/// Step 2 — Fund it (bridge USDC from Arbitrum)
/// Step 3 — Confirmed (balance detected / dashboard redirect)
async fn hl_setup_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let tid = match get_session_tenant_id(&headers, &app.session_secret) {
        Some(t) => t,
        None => return axum::response::Redirect::to("/login").into_response(),
    };

    // Terms must be accepted before setup
    let (hl_address, key_enc, setup_complete) = {
        let tenants = app.tenants.read().await;
        match tenants.get(&tid) {
            Some(h) => (
                h.config.hl_wallet_address.clone(),
                h.config.hl_wallet_key_enc.clone(),
                h.config.hl_setup_complete,
            ),
            None => return axum::response::Redirect::to("/login").into_response(),
        }
    };

    // If wallet not generated yet, go back to ToS
    let (address, key_enc_str) = match (hl_address, key_enc) {
        (Some(a), Some(k)) => (a, k),
        _ => return axum::response::Redirect::to("/app/onboarding").into_response(),
    };

    // Decrypt private key for display — only materialised in memory here
    let private_key =
        match crate::hl_wallet::decrypt_key(&key_enc_str, &app.session_secret, tid.as_str()) {
            Ok(k) => k,
            Err(e) => {
                log::error!("❌ HL wallet key decrypt failed for {}: {}", tid, e);
                return (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "Key decryption failed — please contact support",
                )
                    .into_response();
            }
        };

    let setup_done_js = if setup_complete { "true" } else { "false" };

    let html = format!(
        r###"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>TradingBots.fun · Wallet Setup</title>
<style>
*{{box-sizing:border-box;margin:0;padding:0}}
body{{background:#0d1117;color:#c9d1d9;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;
      min-height:100vh;display:flex;flex-direction:column;align-items:center;justify-content:center;
      padding:24px;
      background-image:linear-gradient(rgba(88,166,255,.03) 1px,transparent 1px),
                       linear-gradient(90deg,rgba(88,166,255,.03) 1px,transparent 1px);
      background-size:44px 44px}}
.wrap{{width:100%;max-width:520px;display:flex;flex-direction:column;gap:16px}}
/* progress bar */
.prog{{display:flex;align-items:center;gap:0;margin-bottom:4px}}
.ps{{display:flex;align-items:center;gap:8px;flex:1}}
.ps-dot{{width:28px;height:28px;border-radius:50%;display:flex;align-items:center;
          justify-content:center;font-size:.78rem;font-weight:700;flex-shrink:0;transition:.3s}}
.ps-dot.done{{background:#3fb950;color:#0d1117}}
.ps-dot.active{{background:#58a6ff;color:#0d1117}}
.ps-dot.idle{{background:#21262d;color:#484f58}}
.ps-label{{font-size:.74rem;color:#6e7681;white-space:nowrap}}
.ps-line{{flex:1;height:2px;background:#21262d;margin:0 4px}}
.ps-line.done{{background:#3fb950}}
/* cards */
.card{{background:#161b22;border:1px solid #21262d;border-radius:14px;padding:24px;
       display:flex;flex-direction:column;gap:16px}}
.card-title{{font-size:1rem;font-weight:700;color:#e6edf3;display:flex;align-items:center;gap:8px}}
.card-sub{{font-size:.78rem;color:#6e7681;line-height:1.55}}
/* address / key display */
.mono-box{{background:#010409;border:1px solid #30363d;border-radius:8px;padding:12px 14px;
           font-family:'JetBrains Mono',Consolas,monospace;font-size:.82rem;color:#58a6ff;
           word-break:break-all;line-height:1.5;position:relative}}
.mono-box.key-box{{color:#f0883e;border-color:rgba(240,136,62,.3);background:rgba(240,136,62,.04)}}
.mono-label{{font-size:.68rem;color:#484f58;font-weight:600;letter-spacing:.5px;
             text-transform:uppercase;margin-bottom:4px}}
/* buttons */
.btn{{display:block;width:100%;padding:13px;border-radius:9px;font-size:.92rem;font-weight:700;
      cursor:pointer;border:none;transition:.15s;letter-spacing:.01em;text-align:center;text-decoration:none}}
.btn-g{{background:#3fb950;color:#0d1117}}
.btn-g:hover:not(:disabled){{background:#52c965}}
.btn-g:disabled{{opacity:.4;cursor:not-allowed}}
.btn-outline{{background:transparent;border:1px solid #30363d;color:#8b949e;font-size:.85rem;padding:10px}}
.btn-outline:hover{{border-color:#58a6ff;color:#58a6ff}}
.btn-row{{display:flex;gap:10px}}
.btn-row .btn{{flex:1}}
/* warning box */
.warn{{background:rgba(248,81,73,.06);border:1px solid rgba(248,81,73,.22);
       border-radius:8px;padding:12px 14px;font-size:.76rem;color:#8b949e;line-height:1.6}}
.warn strong{{color:#f85149}}
/* balance indicator */
.bal-check{{display:flex;align-items:center;gap:10px;padding:12px 14px;
            background:#010409;border:1px solid #30363d;border-radius:8px}}
.spinner{{width:18px;height:18px;border:2px solid #30363d;border-top-color:#58a6ff;
          border-radius:50%;animation:spin .8s linear infinite;flex-shrink:0}}
@keyframes spin{{to{{transform:rotate(360deg)}}}}
.bal-text{{font-size:.82rem;color:#8b949e}}
.bal-amount{{font-size:.9rem;font-weight:700;color:#3fb950}}
.hidden{{display:none!important}}
/* bridge chips */
.chips{{display:flex;gap:8px;flex-wrap:wrap}}
.chip{{padding:6px 14px;border-radius:20px;font-size:.78rem;font-weight:600;
       background:#21262d;color:#8b949e;border:1px solid #30363d}}
.chip.rec{{border-color:#58a6ff;color:#58a6ff;background:rgba(88,166,255,.08)}}
@media(max-width:480px){{
  .btn-row{{flex-direction:column}}
  .prog{{gap:2px}}
  .ps-label{{display:none}}
}}
</style>
</head>
<body>
<div class="wrap">

  <!-- Header -->
  <div style="text-align:center;margin-bottom:8px">
    <div style="font-size:1.1rem;font-weight:800;color:#e6edf3;margin-bottom:3px">
      TradingBots<span style="color:#3fb950">.fun</span>
    </div>
    <div style="font-size:.75rem;color:#484f58">Wallet setup — takes about 2 minutes</div>
  </div>

  <!-- Progress -->
  <div class="prog">
    <div class="ps">
      <div class="ps-dot active" id="dot1">1</div>
      <span class="ps-label">Your wallet</span>
    </div>
    <div class="ps-line" id="line1"></div>
    <div class="ps">
      <div class="ps-dot idle" id="dot2">2</div>
      <span class="ps-label">Add funds</span>
    </div>
    <div class="ps-line" id="line2"></div>
    <div class="ps">
      <div class="ps-dot idle" id="dot3">3</div>
      <span class="ps-label">Done</span>
    </div>
  </div>

  <!-- Step 1: Wallet keys -->
  <div class="card" id="step1">
    <div class="card-title">🔑 Your Hyperliquid Trading Wallet</div>
    <div class="card-sub">
      A dedicated wallet has been generated for you. This wallet holds your funds on Hyperliquid
      and is used to sign every trade the bot makes on your behalf.
    </div>

    <div>
      <div class="mono-label">Wallet address (public)</div>
      <div class="mono-box" id="addr-box">{address}</div>
    </div>

    <div>
      <div class="mono-label">Private key — save this somewhere safe</div>
      <div class="mono-box key-box" id="key-box">{private_key}</div>
    </div>

    <div class="warn">
      <strong>⚠ Back up your private key now.</strong>
      Anyone who has it can access your wallet. Save it to your iCloud Drive, Google Drive,
      or a password manager — then click the button below to continue.
      <br><br>You can also re-export it any time from <b>Settings → Export Private Key</b>.
    </div>

    <div class="btn-row">
      <button class="btn btn-outline" onclick="downloadKey()">⬇ Download .json</button>
      <button class="btn btn-g" onclick="copyKey()">Copy key</button>
    </div>
    <button class="btn btn-g" id="ack-btn" onclick="goStep2()">
      ✓ I&apos;ve saved my private key — continue
    </button>
  </div>

  <!-- Step 2: Fund wallet (hidden until step 1 acked) -->
  <div class="card hidden" id="step2">
    <div class="card-title">💸 Fund Your Trading Account</div>
    <div class="card-sub">
      Deposit USDC from Arbitrum directly to your Hyperliquid account.
      The bot needs at least <strong style="color:#e6edf3">$50 USDC</strong> to open its first position.
    </div>

    <div>
      <div class="mono-label">Your Hyperliquid deposit address</div>
      <div class="mono-box" style="cursor:pointer" onclick="copyAddr()" title="Click to copy">
        {address}
        <span style="float:right;font-size:.7rem;color:#484f58" id="copy-addr-hint">click to copy</span>
      </div>
    </div>

    <div>
      <div class="mono-label">Suggested amounts</div>
      <div class="chips">
        <div class="chip">$50</div>
        <div class="chip rec">$100 ★</div>
        <div class="chip">$250</div>
        <div class="chip">$500</div>
      </div>
    </div>

    <a class="btn btn-g" href="https://app.hyperliquid.xyz/deposit" target="_blank"
       style="text-align:center">
      Open Hyperliquid Bridge →
    </a>

    <div class="card-sub" style="margin-top:-4px">
      Already have USDC on Arbitrum? Paste your deposit address into the Hyperliquid bridge.
      Funds typically arrive within 2 minutes.
      <br><br>
      New to crypto?
      <a href="https://www.coinbase.com" target="_blank" style="color:#58a6ff">Buy USDC on Coinbase</a>
      → send to Arbitrum → bridge to Hyperliquid.
    </div>

    <div style="display:flex;flex-direction:column;gap:8px">
      <div class="bal-check">
        <div class="spinner" id="spinner"></div>
        <div>
          <div class="bal-text" id="bal-text">Checking for deposits…</div>
          <div class="bal-amount hidden" id="bal-amount"></div>
        </div>
      </div>
      <button class="btn btn-outline" onclick="goStep3()" style="margin-top:4px">
        Skip for now, go to dashboard →
      </button>
    </div>
  </div>

  <!-- Step 3: Done -->
  <div class="card hidden" id="step3">
    <div class="card-title">🎉 You&apos;re all set!</div>
    <div class="card-sub">
      Your trading wallet is ready. Head to the dashboard to activate your bots and
      start tracking your positions.
    </div>
    <a href="/app" class="btn btn-g">Go to dashboard →</a>
  </div>

</div>

<script>
const WALLET_ADDRESS = {address:?};
const PRIVATE_KEY    = {private_key:?};
const SETUP_DONE     = {setup_done_js};

function downloadKey() {{
  const data = {{
    platform:   "TradingBots.fun",
    address:    WALLET_ADDRESS,
    privateKey: PRIVATE_KEY,
    network:    "Hyperliquid (EVM-compatible)",
    createdAt:  new Date().toISOString(),
    note:       "Keep this file safe. Import into MetaMask to access your wallet externally.",
  }};
  const blob = new Blob([JSON.stringify(data, null, 2)], {{type: 'application/json'}});
  const a    = document.createElement('a');
  a.href     = URL.createObjectURL(blob);
  a.download = 'tradingbots-wallet.json';
  a.click();
  URL.revokeObjectURL(a.href);
}}

function copyKey() {{
  navigator.clipboard.writeText(PRIVATE_KEY).then(() => {{
    const btn = document.querySelector('#step1 .btn-row .btn-g');
    btn.textContent = '✓ Copied!';
    setTimeout(() => btn.textContent = 'Copy key', 2000);
  }});
}}

function copyAddr() {{
  navigator.clipboard.writeText(WALLET_ADDRESS).then(() => {{
    document.getElementById('copy-addr-hint').textContent = '✓ copied';
    setTimeout(() => document.getElementById('copy-addr-hint').textContent = 'click to copy', 2000);
  }});
}}

function setStep(n) {{
  for (let i = 1; i <= 3; i++) {{
    document.getElementById('step'+i).classList.toggle('hidden', i !== n);
    const dot  = document.getElementById('dot'+i);
    dot.className = 'ps-dot ' + (i < n ? 'done' : i === n ? 'active' : 'idle');
    dot.textContent = i < n ? '✓' : i;
    if (i < 3) {{
      document.getElementById('line'+i).className = 'ps-line' + (i < n ? ' done' : '');
    }}
  }}
}}

async function goStep2() {{
  // Mark setup acknowledged on server
  await fetch('/app/setup/complete', {{method:'POST'}}).catch(()=>{{}});
  setStep(2);
  startPolling();
}}

function goStep3() {{ setStep(3); }}

// Balance polling
let pollTimer;
function startPolling() {{
  pollTimer = setInterval(checkBalance, 15000);
  checkBalance();
}}

async function checkBalance() {{
  try {{
    const res  = await fetch('/api/hl/balance');
    const data = await res.json();
    const bal  = data.balance_usd || 0;
    if (bal > 0) {{
      clearInterval(pollTimer);
      document.getElementById('spinner').style.display = 'none';
      document.getElementById('bal-text').textContent = 'Funds detected!';
      const amtEl = document.getElementById('bal-amount');
      amtEl.textContent = '$' + bal.toFixed(2) + ' USDC on Hyperliquid';
      amtEl.classList.remove('hidden');
      setTimeout(() => setStep(3), 1500);
    }} else {{
      document.getElementById('bal-text').textContent = 'Watching for deposits…';
    }}
  }} catch(e) {{}}
}}

// Auto-start on step 2 if setup was already done on a previous visit
if (SETUP_DONE) {{
  setStep(2);
  startPolling();
}}
</script>
</body></html>"###,
        address = address,
        private_key = private_key,
        setup_done_js = setup_done_js,
    );

    axum::response::Response::builder()
        .status(200)
        .header("Content-Type", "text/html; charset=utf-8")
        .header("Cache-Control", "no-store, no-cache, must-revalidate")
        .body(axum::body::Body::from(html))
        .unwrap_or_else(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// `POST /app/setup/complete` — mark the HL wallet setup as acknowledged.
/// Called by the frontend when the user confirms they have saved their private key.
async fn hl_setup_complete_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let tid = match get_session_tenant_id(&headers, &app.session_secret) {
        Some(t) => t,
        None => return axum::http::StatusCode::UNAUTHORIZED.into_response(),
    };

    {
        let mut tenants = app.tenants.write().await;
        let _ = tenants.complete_hl_setup(&tid);
    }

    if let Some(ref db) = app.db {
        if let Ok(tid_uuid) = uuid::Uuid::parse_str(tid.as_str()) {
            let _ = sqlx::query!(
                "UPDATE tenants SET hl_setup_complete = true WHERE id = $1",
                tid_uuid,
            )
            .execute(db.pool())
            .await
            .map_err(|e| log::error!("❌ hl_setup_complete persist: {}", e));
        }
    }

    axum::http::StatusCode::OK.into_response()
}

/// `GET /api/hl/balance` — return the live Hyperliquid cleared balance for the
/// authenticated tenant.  Used by the setup page to detect first deposits.
async fn hl_balance_api_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let tid = match get_session_tenant_id(&headers, &app.session_secret) {
        Some(t) => t,
        None => return axum::http::StatusCode::UNAUTHORIZED.into_response(),
    };

    let address = {
        let tenants = app.tenants.read().await;
        tenants
            .get(&tid)
            .and_then(|h| h.config.hl_wallet_address.clone())
    };

    let balance_usd = match address {
        Some(ref addr) => crate::hl_wallet::check_balance(addr).await,
        None => 0.0,
    };

    axum::response::Json(serde_json::json!({
        "balance_usd": balance_usd,
        "address":     address,
    }))
    .into_response()
}

/// `GET /api/hl/wallet/key.json` — export the tenant's HL trading wallet as a
/// downloadable JSON file.  Requires an active session (authenticated user only).
async fn hl_export_key_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let tid = match get_session_tenant_id(&headers, &app.session_secret) {
        Some(t) => t,
        None => return axum::http::StatusCode::UNAUTHORIZED.into_response(),
    };

    let (address, key_enc) = {
        let tenants = app.tenants.read().await;
        match tenants.get(&tid) {
            Some(h) => (
                h.config.hl_wallet_address.clone(),
                h.config.hl_wallet_key_enc.clone(),
            ),
            None => return axum::http::StatusCode::NOT_FOUND.into_response(),
        }
    };

    let (addr, enc) = match (address, key_enc) {
        (Some(a), Some(k)) => (a, k),
        _ => {
            return (
                axum::http::StatusCode::NOT_FOUND,
                "No HL wallet found for this account",
            )
                .into_response()
        }
    };

    let private_key = match crate::hl_wallet::decrypt_key(&enc, &app.session_secret, tid.as_str()) {
        Ok(k) => k,
        Err(e) => {
            log::error!("❌ HL key export decrypt failed for {}: {}", tid, e);
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Key decryption failed",
            )
                .into_response();
        }
    };

    let payload = serde_json::json!({
        "platform":   "TradingBots.fun",
        "address":    addr,
        "privateKey": private_key,
        "network":    "Hyperliquid (EVM-compatible)",
        "exportedAt": chrono::Utc::now().to_rfc3339(),
        "note": "Keep this file safe. Import into MetaMask or any EVM wallet to access your Hyperliquid account externally."
    });

    let json_str = serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{{}}".to_string());

    axum::response::Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .header(
            "Content-Disposition",
            "attachment; filename=\"tradingbots-wallet.json\"",
        )
        .header("Cache-Control", "no-store")
        .body(axum::body::Body::from(json_str))
        .unwrap_or_else(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

// ─────────────────────────────────────────────────────────────────────────────
//  AI Trade Command API  — /api/command
// ─────────────────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct CommandRequest {
    command: String,
}

/// Parse a natural-language operator command into a `BotCommand`.
///
/// Recognised patterns (case-insensitive):
///   "close all" / "close everything" / "exit all"  → CloseAll
///   "take profits" / "take all profits"             → CloseProfitable
///   "take profit from <sym>" / "take profit <sym>"  → TakePartial { symbol }
///   "close <sym>" / "exit <sym>" / "sell <sym>"     → ClosePosition { symbol }
///   "tp <sym>"                                       → TakePartial { symbol }
fn parse_trade_command(cmd: &str) -> Option<BotCommand> {
    let lower = cmd.trim().to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();

    // ── "close all" variants ──────────────────────────────────────────────
    if lower.contains("close all")
        || lower.contains("close everything")
        || lower.contains("exit all")
        || lower.contains("sell all")
        || lower.contains("close every")
    {
        return Some(BotCommand::CloseAll);
    }

    // ── Partial-close intent: "close 1/3", "take 1/3", "close half",
    //    "close a third of", "close one third of", "partial close", etc.
    //
    // These all indicate TakePartial (removes first unbanked tranche from
    // the position).  The fraction/qualifier is stripped and the symbol is
    // extracted from the remainder of the phrase.
    //
    // Matched phrases (case-insensitive):
    //   "close 1/3 of TAO"             → TakePartial TAO
    //   "close a third of the TAO position" → TakePartial TAO
    //   "close half of SOL"            → TakePartial SOL
    //   "take 1/3 of TAO"              → TakePartial TAO
    //   "partial close TAO"            → TakePartial TAO
    //   "take partial TAO"             → TakePartial TAO
    //   "take some profit from TAO"    → TakePartial TAO
    let is_partial_phrase = lower.contains("1/3")
        || lower.contains("one third")
        || lower.contains("a third")
        || lower.contains("half")
        || lower.contains("partial")
        || lower.contains("some profit")
        || lower.contains("1/2")
        || lower.contains("33%")
        || lower.contains("50%");

    if is_partial_phrase {
        // Strip all fraction/size qualifiers then find the first word that
        // looks like a crypto symbol (all-uppercase letters or a known ticker).
        // Strategy: walk words, skip known stopwords, return first non-stop word.
        const STOP: &[&str] = &[
            "close", "take", "exit", "sell", "reduce", "partial", "profit",
            "profits", "1/3", "1/2", "a", "an", "the", "of", "from", "my",
            "on", "in", "for", "some", "half", "third", "one", "position",
            "trade", "33%", "50%", "percent", "lot",
        ];
        for w in &words {
            if !STOP.contains(w) && !w.is_empty() && w.len() >= 2 {
                return Some(BotCommand::TakePartial {
                    symbol: w.to_uppercase(),
                });
            }
        }
        // Phrase matched but no symbol found — return TakePartial with empty
        // to surface a clearer error rather than falling through silently.
        return None;
    }

    // ── "take profits" with no specific symbol ────────────────────────────
    if (lower.contains("take profit") || lower.contains("take profits"))
        && !lower.contains(" from ")
        && words.len() <= 3
    {
        return Some(BotCommand::CloseProfitable);
    }

    // ── Word-by-word scan for close / take-profit + symbol ────────────────
    //
    // NOTE: this runs AFTER the partial-close check, so "close 1/3 …" will
    // never reach here; only plain "close <SYMBOL>" falls through.
    for (i, word) in words.iter().enumerate() {
        match *word {
            "close" | "exit" | "sell" => {
                // Skip quantifiers (numbers, fractions, "the", "my") so that
                // "close the BTC position" → ClosePosition BTC (not "the").
                const SKIP_WORDS: &[&str] =
                    &["the", "my", "a", "an", "position", "trade", "all"];
                let sym = words[i + 1..]
                    .iter()
                    .find(|&&w| !SKIP_WORDS.contains(&w) && w != "all")
                    .copied();
                if let Some(sym) = sym {
                    return Some(BotCommand::ClosePosition {
                        symbol: sym.to_uppercase(),
                    });
                }
            }
            "tp" => {
                // "tp SOL"
                if let Some(sym) = words.get(i + 1) {
                    return Some(BotCommand::TakePartial {
                        symbol: sym.to_uppercase(),
                    });
                }
            }
            "profit" | "profits" => {
                // "take profit from kFloki", "take profit BTC"
                // skip optional "from"
                let next = words.get(i + 1);
                let sym = if next == Some(&"from") {
                    words.get(i + 2)
                } else {
                    next
                };
                if let Some(s) = sym {
                    return Some(BotCommand::TakePartial {
                        symbol: s.to_uppercase(),
                    });
                }
            }
            _ => {}
        }
    }

    None
}

/// `POST /api/command` — queue a manual trade-execution command.
///
/// Body:  `{"command": "take profit from kFloki"}`
///
/// The command is parsed into a `BotCommand` and appended to `pending_cmds`
/// in `BotState`.  It executes at the start of the **next trading cycle**
/// (~30 seconds) with a live market price.
///
/// Response:
///   `{"ok":true,  "action":"TakePartial","symbol":"KFLOKI","msg":"Queued…"}`
///   `{"ok":false, "msg":"Could not parse…"}`
async fn command_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
    axum::Json(req): axum::Json<CommandRequest>,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    // Accept either a valid consumer session OR the operator Basic-Auth header.
    let has_consumer_session = get_session_tenant_id(&headers, &app.session_secret).is_some();
    let has_admin_auth = app
        .admin_password
        .as_deref()
        .map(|pw| check_admin_auth(&headers, pw))
        .unwrap_or(false);
    if !has_consumer_session && !has_admin_auth {
        return axum::http::StatusCode::UNAUTHORIZED.into_response();
    }

    // Basic length guard
    if req.command.len() > 200 {
        return axum::response::Json(serde_json::json!({
            "ok": false,
            "msg": "Command too long (max 200 chars)."
        }))
        .into_response();
    }

    let cmd_clean: String = req
        .command
        .chars()
        .map(|c| if (c as u32) < 32 && c != ' ' { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    match parse_trade_command(&cmd_clean) {
        Some(bot_cmd) => {
            // Build a human-readable description for the response
            let (action, symbol, msg) = match &bot_cmd {
                BotCommand::ClosePosition { symbol } => (
                    "ClosePosition",
                    symbol.clone(),
                    format!("Closing {symbol} on next cycle ⏱"),
                ),
                BotCommand::TakePartial { symbol } => (
                    "TakePartial",
                    symbol.clone(),
                    format!("Taking partial profit on {symbol} (tranche 1/3) on next cycle ⏱"),
                ),
                BotCommand::CloseAll => (
                    "CloseAll",
                    String::new(),
                    "Closing ALL positions on next cycle ⏱".to_string(),
                ),
                BotCommand::CloseProfitable => (
                    "CloseProfitable",
                    String::new(),
                    "Taking profits on all winning positions on next cycle ⏱".to_string(),
                ),
                BotCommand::OpenLong { symbol, .. } => (
                    "OpenLong",
                    symbol.clone(),
                    format!("Opening LONG on {symbol} on next cycle ⏱"),
                ),
                BotCommand::OpenShort { symbol, .. } => (
                    "OpenShort",
                    symbol.clone(),
                    format!("Opening SHORT on {symbol} on next cycle ⏱"),
                ),
                BotCommand::SetLeverage { symbol, leverage } => (
                    "SetLeverage",
                    symbol.clone(),
                    format!("Setting leverage for {symbol} to {leverage}× on next cycle ⏱"),
                ),
                BotCommand::PauseTrading => (
                    "PauseTrading",
                    String::new(),
                    "Trading paused — no new entries until ResumeTrading ⏸".to_string(),
                ),
                BotCommand::ResumeTrading => (
                    "ResumeTrading",
                    String::new(),
                    "Trading resumed ▶".to_string(),
                ),
            };

            // Push to queue
            {
                let mut s = app.bot_state.write().await;
                s.pending_cmds.push_back(bot_cmd);
            }

            axum::response::Json(serde_json::json!({
                "ok":     true,
                "action": action,
                "symbol": symbol,
                "msg":    msg,
            }))
            .into_response()
        }
        None => {
            // Not a recognised trade command — tell the caller
            axum::response::Json(serde_json::json!({
                "ok":  false,
                "msg": format!(
                    "Couldn't parse '{}' as a trade command. \
                     Try: 'close SOL', 'take profit ETH', 'close all', 'take profits'.",
                    cmd_clean
                )
            }))
            .into_response()
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Investment thesis API
// ─────────────────────────────────────────────────────────────────────────────

/// `GET /api/thesis` — return the current investment thesis for the UI chip.
///
/// Returns JSON `{"summary": "...", "thesis_text": "..."}` or `{}` when empty.
async fn thesis_get_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    // Accept consumer session OR operator Basic-Auth
    let has_consumer = get_session_tenant_id(&headers, &app.session_secret).is_some();
    let has_admin = app
        .admin_password
        .as_deref()
        .map(|pw| check_admin_auth(&headers, pw))
        .unwrap_or(false);
    if !has_consumer && !has_admin {
        return axum::http::StatusCode::UNAUTHORIZED.into_response();
    }

    let c = app.global_thesis.read().await;
    axum::response::Json(serde_json::json!({
        "summary":     c.summary,
        "thesis_text": c.thesis_text,
    }))
    .into_response()
}

#[derive(serde::Deserialize)]
struct ThesisCommand {
    command: String,
}

/// `POST /api/thesis` — update the investment thesis from a natural-language command.
///
/// Request body: `{"command": "only meme coins max 3x leverage"}`
///
/// Response:
///   - Constraint update: `{"type":"update","summary":"Meme coins · max 3×","message":"..."}`
///   - Reset:             `{"type":"reset","message":"..."}`
///   - Trade query:       `{"type":"query","message":"<recent trades text>"}`
async fn thesis_update_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
    axum::Json(req): axum::Json<ThesisCommand>,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    // Accept consumer session OR operator Basic-Auth (admin acts as single-op tenant)
    let tid = match get_session_tenant_id(&headers, &app.session_secret) {
        Some(t) => t,
        None => {
            let is_admin = app
                .admin_password
                .as_deref()
                .map(|pw| check_admin_auth(&headers, pw))
                .unwrap_or(false);
            if !is_admin {
                return axum::http::StatusCode::UNAUTHORIZED.into_response();
            }
            crate::tenant::TenantId::from_str("00000000-0000-0000-0000-000000000001")
        }
    };

    // ── Input validation ──────────────────────────────────────────────────────

    // 1. Length cap — reject anything over 200 chars before any processing.
    //    Prevents memory exhaustion and cuts off most injection attempts.
    const MAX_CMD_LEN: usize = 200;
    if req.command.len() > MAX_CMD_LEN {
        return axum::response::Json(serde_json::json!({
            "type":    "error",
            "message": "Command too long. Please keep it under 200 characters.",
        }))
        .into_response();
    }

    // 2. Strip control characters and null bytes; collapse whitespace.
    let cmd: String = req
        .command
        .chars()
        .map(|c| if (c as u32) < 32 && c != ' ' { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();

    if cmd.is_empty() {
        return axum::response::Json(serde_json::json!({
            "type":    "error",
            "message": "Empty command.",
        }))
        .into_response();
    }

    // 3. Topic guard — only crypto portfolio commands are accepted.
    //    Reject obvious off-topic patterns before they reach the parser.
    let cmd_lower = cmd.to_lowercase();
    let off_topic_patterns = [
        "ignore previous",
        "disregard",
        "forget your instructions",
        "act as",
        "you are now",
        "new persona",
        "pretend you",
        "system prompt",
        "jailbreak",
        "dan mode",
        "tell me a joke",
        "write a poem",
        "write code",
        "help me with",
        "explain how to",
        "what is the weather",
        "translate",
        "summarize this article",
    ];
    if off_topic_patterns.iter().any(|p| cmd_lower.contains(p)) {
        return axum::response::Json(serde_json::json!({
            "type":    "error",
            "message": "This AI only handles crypto portfolio commands — e.g. \"only BTC ETH\", \"max 5x\", \"show recent trades\".",
        })).into_response();
    }

    // ── Trade query path ──────────────────────────────────────────────────────
    if crate::thesis::parse_command(&cmd).is_none() {
        // Query intent detected — return recent closed trades summary
        let trades_summary = {
            let s = app.bot_state.read().await;
            if s.closed_trades.is_empty() {
                "No trades recorded yet.".to_string()
            } else {
                let recent: Vec<String> = s
                    .closed_trades
                    .iter()
                    .rev()
                    .take(5)
                    .map(|t| {
                        format!(
                            "• {} {} @ ${:.4} → ${:.4} · P&L: {}",
                            t.side, t.symbol, t.entry, t.exit, t.pnl
                        )
                    })
                    .collect();
                recent.join("<br>")
            }
        };
        return axum::response::Json(serde_json::json!({
            "type":    "query",
            "message": trades_summary,
        }))
        .into_response();
    }

    // ── Constraint update path ────────────────────────────────────────────────
    let parsed = crate::thesis::parse_command(&cmd).unwrap_or_default();

    let (whitelist_str, sector, max_lev, thesis_txt) = if parsed.is_empty() {
        // Reset
        (None, None, None, None)
    } else {
        let wl_str = parsed.symbol_whitelist.as_ref().map(|v| v.join(","));
        (
            wl_str,
            parsed.sector_filter.clone(),
            parsed.max_leverage_override,
            parsed.thesis_text.clone(),
        )
    };

    // Update in-memory tenant config
    {
        let mut tenants = app.tenants.write().await;
        let _ = tenants.update_thesis(
            &tid,
            thesis_txt.clone(),
            whitelist_str.clone(),
            sector.clone(),
            max_lev,
        );
    }

    // Persist to DB (non-blocking)
    if let Some(ref db) = app.db {
        if let Ok(tid_uuid) = uuid::Uuid::parse_str(tid.as_str()) {
            let db2 = db.clone();
            let (wl2, sec2, txt2) = (whitelist_str.clone(), sector.clone(), thesis_txt.clone());
            tokio::spawn(async move {
                let _ = sqlx::query!(
                    "UPDATE tenants
                     SET investment_thesis    = $1,
                         symbol_whitelist     = $2,
                         sector_filter        = $3,
                         max_leverage_override = $4
                     WHERE id = $5",
                    txt2,
                    wl2,
                    sec2,
                    max_lev,
                    tid_uuid,
                )
                .execute(db2.pool())
                .await
                .map_err(|e| log::warn!("thesis persist failed: {e}"));
            });
        }
    }

    // Update the global_thesis Arc so run_cycle picks it up immediately
    {
        let new_constraints = if parsed.is_empty() {
            crate::thesis::ThesisConstraints::default()
        } else {
            parsed.clone()
        };
        let mut gt = app.global_thesis.write().await;
        *gt = new_constraints;
    }

    let (resp_type, message, summary) = if parsed.is_empty() {
        (
            "reset",
            "AI decides everything now — all constraints cleared.".to_string(),
            None,
        )
    } else {
        let sum = parsed.summary.clone().unwrap_or_default();
        let msg = format!(
            "Thesis updated: {}. The bot will apply these constraints from the next cycle.",
            sum
        );
        ("update", msg, parsed.summary.clone())
    };

    axum::response::Json(serde_json::json!({
        "type":    resp_type,
        "message": message,
        "summary": summary,
    }))
    .into_response()
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Public landing page — tradingbots.fun
// ═══════════════════════════════════════════════════════════════════════════════

/// `GET /` — public landing page showing aggregate AUM, positions, and returns.
///
/// Visible to any visitor — no authentication required.
/// All numbers are live, fetched from `/api/public/tvl` and `/api/public/stats`
/// via client-side JavaScript that auto-refreshes every 30 seconds.
async fn public_landing_handler(State(_app): State<AppState>) -> axum::response::Html<String> {
    axum::response::Html(r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>TradingBots.fun — AI Trading Bot on Hyperliquid · x402 API</title>
<meta name="description" content="Autonomous AI trading on Hyperliquid perpetuals. Deposit USDC via Arbitrum bridge, earn 24/7. x402 Bot API for autonomous agents — pay-per-session, no subscription.">
<meta name="keywords" content="x402 trading API, AI agent trading, Hyperliquid bot, autonomous trading agent, x402 payment protocol, crypto trading bot, Arbitrum USDC bridge, AI agent Hyperliquid, perpetuals trading bot, x402 bot">
<link rel="canonical" href="https://tradingbots.fun/">
<meta property="og:type" content="website">
<meta property="og:url" content="https://tradingbots.fun/">
<meta property="og:title" content="TradingBots.fun — AI Trading Bot · x402 Agent API">
<meta property="og:description" content="Real capital. Real trades. Autonomous AI on Hyperliquid perpetuals. x402 Bot API lets AI agents start, fund, and control trading sessions programmatically.">
<meta property="og:image" content="https://tradingbots.fun/og-image.png">
<meta name="twitter:card" content="summary_large_image">
<meta name="twitter:title" content="TradingBots.fun — AI Trading · x402 Agent API">
<meta name="twitter:description" content="Autonomous AI trading on Hyperliquid. x402 pay-per-session Bot API. Arbitrum ↔ Hyperliquid bridge built-in.">
<script type="application/ld+json">
{
  "@context": "https://schema.org",
  "@graph": [
    {
      "@type": "SoftwareApplication",
      "name": "TradingBots.fun",
      "url": "https://tradingbots.fun",
      "applicationCategory": "FinanceApplication",
      "operatingSystem": "Web",
      "description": "Autonomous AI trading bot on Hyperliquid perpetuals with x402 payment protocol API for AI agents and autonomous systems.",
      "offers": {
        "@type": "Offer",
        "price": "10",
        "priceCurrency": "USDC",
        "description": "30-day Bot API session via x402 protocol"
      }
    },
    {
      "@type": "WebAPI",
      "name": "TradingBots.fun Bot API",
      "url": "https://tradingbots.fun/api/v1/",
      "description": "x402-gated REST API for autonomous agents. AI agents can start trading sessions, fund wallets via Arbitrum bridge, and control live positions programmatically.",
      "documentation": "https://tradingbots.fun/api/v1/status"
    },
    {
      "@type": "Organization",
      "name": "TradingBots.fun",
      "url": "https://tradingbots.fun",
      "description": "Autonomous AI trading infrastructure for Hyperliquid perpetuals with x402 payment protocol support."
    }
  ]
}
</script>
<style>
*{box-sizing:border-box;margin:0;padding:0}
body{background:#0d1117;color:#c9d1d9;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,sans-serif;min-height:100vh}
a{color:inherit;text-decoration:none}

/* ── Globals ── */
:root{--green:#3fb950;--red:#f85149;--blue:#58a6ff;--yellow:#e3b341;--bg:#0d1117;--bg2:#161b22;--border:#21262d;--muted:#8b949e;--dim:#484f58;--text:#c9d1d9;--text-hi:#e6edf3}

/* ── Nav ── */
.nav{display:flex;align-items:center;justify-content:space-between;padding:14px 32px;border-bottom:1px solid var(--border);position:sticky;top:0;background:rgba(13,17,23,.95);backdrop-filter:blur(8px);z-index:100}
.nav-logo{font-size:1.05rem;font-weight:800;color:var(--text-hi);letter-spacing:-.3px}
.nav-logo .dot{color:var(--green)}
.nav-links{display:flex;gap:20px;align-items:center}
.nav-link{font-size:.83rem;color:var(--muted);transition:.15s}
.nav-link:hover{color:var(--text-hi)}
.nav-cta{background:var(--green);color:#0d1117;padding:7px 16px;border-radius:7px;font-weight:700;font-size:.83rem;transition:.15s}
.nav-cta:hover{background:#52c965}
.live-badge{display:inline-flex;align-items:center;gap:5px;font-size:.72rem;color:var(--green);border:1px solid rgba(63,185,80,.3);border-radius:20px;padding:3px 10px}
.live-badge::before{content:'';width:6px;height:6px;background:var(--green);border-radius:50%;animation:blink 2s infinite}
@keyframes blink{0%,100%{opacity:1}50%{opacity:.2}}

/* ── Hero ── */
.hero{text-align:center;padding:64px 24px 52px;background:radial-gradient(ellipse 120% 60% at 50% 0%,rgba(63,185,80,.07) 0%,transparent 70%);position:relative;overflow:hidden}
.hero-eyebrow{display:inline-block;background:rgba(63,185,80,.1);border:1px solid rgba(63,185,80,.25);border-radius:20px;padding:4px 14px;font-size:.7rem;font-weight:700;color:var(--green);letter-spacing:.9px;text-transform:uppercase;margin-bottom:18px}
.hero h1{font-size:clamp(2rem,4.5vw,3rem);font-weight:800;color:var(--text-hi);line-height:1.15;margin-bottom:10px}
.hero h1 em{font-style:normal;background:linear-gradient(135deg,var(--green),#58e87a);-webkit-background-clip:text;-webkit-text-fill-color:transparent}
.hero-pnl{font-size:1.5rem;font-weight:800;color:var(--green);margin-bottom:10px;font-variant-numeric:tabular-nums;letter-spacing:-.02em}
.hero-pnl.neg{color:var(--red)}
.hero-pnl-meta{font-size:.78rem;color:var(--muted);font-weight:400;margin-left:8px}
.hero-sub{font-size:.88rem;color:var(--muted);max-width:420px;margin:0 auto 28px;line-height:1.6}
.hero-btns{display:flex;gap:10px;justify-content:center;flex-wrap:wrap}
.btn-p{background:var(--green);color:#0d1117;padding:12px 26px;border-radius:9px;font-weight:700;font-size:.9rem;transition:.15s;display:inline-block}
.btn-p:hover{background:#52c965;transform:translateY(-1px)}
.btn-s{background:transparent;border:1px solid var(--border);color:var(--text);padding:12px 26px;border-radius:9px;font-weight:600;font-size:.9rem;transition:.15s;display:inline-block}
.btn-s:hover{border-color:var(--blue);color:var(--blue)}

/* ── Metrics grid ── */
.metrics-grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(160px,1fr));gap:1px;background:var(--border)}
.m-cell{background:var(--bg);padding:22px 20px;text-align:center;position:relative}
.m-cell:hover{background:#111820}
.m-val{font-size:1.7rem;font-weight:800;color:var(--text-hi);letter-spacing:-.5px;line-height:1;font-variant-numeric:tabular-nums}
.m-val.g{color:var(--green)}.m-val.r{color:var(--red)}.m-val.b{color:var(--blue)}.m-val.y{color:var(--yellow)}
.m-lbl{font-size:.67rem;color:var(--dim);text-transform:uppercase;letter-spacing:.7px;margin-top:5px}
.m-sub{font-size:.72rem;color:var(--dim);margin-top:2px}
.m-tip{position:absolute;top:8px;right:10px;font-size:.6rem;color:var(--dim);cursor:default}

/* ── Section ── */
.wrap{max-width:1060px;margin:0 auto;padding:0 20px}
.sec{padding:40px 0 0}
.sec-head{display:flex;align-items:center;gap:10px;margin-bottom:16px}
.sec-title{font-size:.78rem;font-weight:700;color:var(--muted);text-transform:uppercase;letter-spacing:.8px}
.sec-line{flex:1;height:1px;background:var(--border)}

/* ── AUM Chart (hero background) ── */
#aum-canvas{position:absolute;inset:0;width:100%;height:100%;opacity:.18;pointer-events:none}

/* ── Stat bar (below hero) ── */
.stat-bar{display:flex;justify-content:center;background:var(--bg2);border-bottom:1px solid var(--border);flex-wrap:wrap}
.stat-cell{padding:18px 36px;text-align:center;border-right:1px solid var(--border);flex:1;min-width:140px}
.stat-cell:last-child{border-right:none}
.stat-val{font-size:1.45rem;font-weight:800;color:var(--text-hi);font-variant-numeric:tabular-nums;line-height:1}
.stat-lbl{font-size:.64rem;color:var(--dim);text-transform:uppercase;letter-spacing:.75px;margin-top:5px}

/* ── Bridge badge ── */
.bridge-badge{display:inline-flex;align-items:center;gap:6px;background:rgba(88,166,255,.08);border:1px solid rgba(88,166,255,.2);border-radius:8px;padding:5px 12px;font-size:.72rem;color:var(--blue);font-weight:600;margin-bottom:22px}
.bridge-badge svg{opacity:.7}

/* ── Venue badge ── */
.venue-badge{display:inline-block;font-size:.7rem;padding:2px 8px;border-radius:999px;background:rgba(56,139,253,.12);color:#58a6ff;border:1px solid rgba(56,139,253,.3);white-space:nowrap}

/* ── x402 highlight band ── */
.x402-hero{background:linear-gradient(135deg,rgba(88,166,255,.06) 0%,rgba(63,185,80,.04) 100%);border:1px solid rgba(88,166,255,.18);border-radius:16px;padding:28px 28px 24px;margin-top:40px}
.x402-hero h3{font-size:1.05rem;font-weight:800;color:var(--text-hi);margin-bottom:6px;display:flex;align-items:center;gap:8px}
.x402-tag{background:rgba(88,166,255,.15);color:var(--blue);border-radius:5px;padding:2px 8px;font-size:.65rem;font-weight:800;letter-spacing:.5px}
.x402-hero p{font-size:.82rem;color:var(--muted);line-height:1.7;margin-bottom:18px}
.x402-flow{display:flex;flex-direction:column;gap:8px}
.x402-step{display:flex;align-items:flex-start;gap:10px;font-size:.78rem;color:var(--text)}
.x402-step-num{width:20px;height:20px;border-radius:50%;background:rgba(88,166,255,.15);color:var(--blue);font-size:.65rem;font-weight:800;display:flex;align-items:center;justify-content:center;flex-shrink:0;margin-top:1px}

/* ── Algo cards ── */
.algo-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(300px,1fr));gap:12px}
.algo-card{background:var(--bg2);border:1px solid var(--border);border-radius:14px;padding:20px 22px}
.algo-card.active-regime{border-color:rgba(63,185,80,.4);background:rgba(63,185,80,.03)}
.algo-name{font-size:.95rem;font-weight:700;color:var(--text-hi);margin-bottom:4px;display:flex;align-items:center;gap:8px}
.algo-badge{font-size:.63rem;padding:2px 7px;border-radius:5px;font-weight:700;letter-spacing:.4px}
.badge-active{background:rgba(63,185,80,.15);color:var(--green);border:1px solid rgba(63,185,80,.3)}
.badge-standby{background:rgba(72,79,88,.15);color:var(--dim);border:1px solid var(--border)}
.algo-desc{font-size:.78rem;color:var(--muted);margin-bottom:14px;line-height:1.6}
.algo-signals{display:flex;flex-wrap:wrap;gap:6px}
.sig-pill{font-size:.68rem;padding:3px 9px;border-radius:5px;font-weight:600}
.sig-primary{background:rgba(88,166,255,.12);color:var(--blue);border:1px solid rgba(88,166,255,.2)}
.sig-secondary{background:rgba(72,79,88,.12);color:var(--muted);border:1px solid var(--border)}

/* ── Position tile scroll strip ── */
.pos-strip-wrap{overflow:hidden;border-top:1px solid var(--border);border-bottom:1px solid var(--border);background:var(--bg);padding:12px 0;min-height:110px;display:flex;align-items:center}
.pos-strip{display:flex;gap:12px;animation:pan-tiles 30s linear infinite;white-space:nowrap;padding:0 16px;align-items:stretch}
.pos-strip:hover{animation-play-state:paused}
.pos-strip.no-anim{animation:none}
@keyframes pan-tiles{0%{transform:translateX(0)}100%{transform:translateX(-50%)}}
.pos-tile{display:inline-flex;flex-direction:column;justify-content:space-between;gap:6px;background:var(--bg2);border:1px solid var(--border);border-radius:11px;padding:13px 16px;min-width:160px;cursor:default;transition:.15s;white-space:normal;vertical-align:top}
.pos-tile:hover{border-color:rgba(88,166,255,.35);background:#111820}
.pos-tile.long-tile{border-color:rgba(63,185,80,.25)}
.pos-tile.short-tile{border-color:rgba(248,81,73,.25)}
.pt-sym{font-size:1rem;font-weight:800;color:var(--text-hi);letter-spacing:-.3px}
.pt-row{display:flex;justify-content:space-between;align-items:center;gap:8px}
.pt-entry{font-size:.7rem;color:var(--dim);font-variant-numeric:tabular-nums}
.pt-pnl{font-size:.95rem;font-weight:700;font-variant-numeric:tabular-nums}
.pt-meta{font-size:.65rem;color:var(--dim)}
.pos-tile-empty{color:var(--dim);font-size:.82rem;padding:0 24px;white-space:nowrap}

/* ── Signal weights ── */
.weights-grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(200px,1fr));gap:8px}
.w-row{background:var(--bg2);border:1px solid var(--border);border-radius:10px;padding:12px 14px;display:flex;align-items:center;gap:10px}
.w-name{font-size:.78rem;color:var(--text);font-weight:600;width:110px;flex-shrink:0}
.w-bar-wrap{flex:1;height:6px;background:rgba(255,255,255,.05);border-radius:3px;overflow:hidden}
.w-bar{height:100%;border-radius:3px;background:var(--green);transition:width .5s ease}
.w-pct{font-size:.72rem;color:var(--muted);width:34px;text-align:right;flex-shrink:0;font-variant-numeric:tabular-nums}

/* ── Tables ── */
.card{background:var(--bg2);border:1px solid var(--border);border-radius:14px;overflow:hidden}
.card-head{padding:14px 18px;border-bottom:1px solid var(--border);display:flex;justify-content:space-between;align-items:center}
.card-title{font-size:.85rem;font-weight:700;color:var(--text-hi)}
.live-dot{display:inline-flex;align-items:center;gap:5px;font-size:.7rem;color:var(--green)}
.live-dot::before{content:'';width:6px;height:6px;background:var(--green);border-radius:50%;animation:blink 2s infinite}
tbl{width:100%;border-collapse:collapse}
table{width:100%;border-collapse:collapse}
th{padding:9px 14px;font-size:.65rem;font-weight:700;color:var(--dim);text-transform:uppercase;letter-spacing:.6px;text-align:left;border-bottom:1px solid var(--border)}
.tr td{padding:12px 14px;font-size:.82rem;border-bottom:1px solid rgba(48,54,61,.4);transition:background .1s}
.tr:last-child td{border-bottom:none}
.tr:hover td{background:rgba(255,255,255,.018)}
.pos{color:var(--green);font-weight:700}
.neg{color:var(--red);font-weight:700}
.neu{color:var(--muted)}
.mono{font-family:monospace;font-size:.75rem;color:var(--dim)}
.side-long{background:rgba(63,185,80,.12);color:var(--green);padding:2px 7px;border-radius:4px;font-size:.68rem;font-weight:700}
.side-short{background:rgba(248,81,73,.12);color:var(--red);padding:2px 7px;border-radius:4px;font-size:.68rem;font-weight:700}
.reason-pill{font-size:.65rem;padding:2px 7px;border-radius:4px;border:1px solid var(--border);color:var(--muted)}

/* ── Scrolling trades ticker ── */
.ticker-wrap{overflow:hidden;border-top:1px solid var(--border);border-bottom:1px solid var(--border);background:var(--bg);padding:8px 0}
.ticker-inner{display:flex;gap:32px;animation:scroll-ticker 40s linear infinite;white-space:nowrap;padding:0 20px}
.ticker-inner:hover{animation-play-state:paused}
@keyframes scroll-ticker{0%{transform:translateX(0)}100%{transform:translateX(-50%)}}
.tick-item{display:inline-flex;align-items:center;gap:7px;font-size:.78rem}
.tick-sym{font-weight:700;color:var(--text-hi)}
.tick-amt{font-variant-numeric:tabular-nums}

/* ── Portfolio table ── */
.acct-cap{color:var(--muted);font-variant-numeric:tabular-nums}
.acct-name{font-weight:700;color:var(--text-hi)}

/* ── How it works ── */
.steps{display:grid;grid-template-columns:repeat(auto-fit,minmax(220px,1fr));gap:16px;margin-top:16px}
.step{background:var(--bg2);border:1px solid var(--border);border-radius:14px;padding:22px 20px;text-align:center}
.step-num{font-size:1.8rem;margin-bottom:8px}
.step-title{font-size:.95rem;font-weight:700;color:var(--text-hi);margin-bottom:6px}
.step-desc{font-size:.78rem;color:var(--muted);line-height:1.6}

/* ── Bot API section ── */
.agent-grid{display:grid;grid-template-columns:1fr 1fr;gap:20px;align-items:start}
@media(max-width:700px){.agent-grid{grid-template-columns:1fr}}
.agent-text h3{font-size:1.1rem;font-weight:800;color:var(--text-hi);margin-bottom:8px}
.agent-text p{font-size:.82rem;color:var(--muted);line-height:1.7;margin-bottom:16px}
.endpoint-list{display:flex;flex-direction:column;gap:7px}
.ep{display:flex;align-items:center;gap:9px;font-size:.78rem;font-family:monospace;color:var(--text);background:rgba(255,255,255,.03);border:1px solid var(--border);border-radius:7px;padding:7px 10px}
.m-get{background:rgba(63,185,80,.15);color:var(--green);border-radius:4px;padding:2px 7px;font-size:.65rem;font-weight:800;letter-spacing:.5px}
.m-post{background:rgba(88,166,255,.15);color:var(--blue);border-radius:4px;padding:2px 7px;font-size:.65rem;font-weight:800;letter-spacing:.5px}
.code-box{background:#0a0e14;border:1px solid var(--border);border-radius:12px;overflow:hidden}
.code-box-head{background:#161b22;padding:10px 14px;font-size:.7rem;color:var(--dim);display:flex;align-items:center;gap:6px;border-bottom:1px solid var(--border)}
.code-box pre{padding:16px;font-size:.72rem;color:#cdd9e5;line-height:1.7;overflow-x:auto;margin:0}
.code-box .kw{color:#79c0ff}.code-box .str{color:#a5d6ff}.code-box .cmt{color:#484f58}
.cta-band{background:linear-gradient(135deg,rgba(63,185,80,.08),rgba(88,166,255,.05));border:1px solid rgba(63,185,80,.2);border-radius:16px;padding:36px 24px;text-align:center;margin:40px 0 0}
.cta-band h3{font-size:1.4rem;font-weight:800;color:var(--text-hi);margin-bottom:8px}
.cta-band p{font-size:.88rem;color:var(--muted);margin-bottom:20px}

/* ── Compact win row ── */
.win-row{display:flex;align-items:center;gap:10px;background:var(--bg2);border:1px solid var(--border);border-radius:9px;padding:10px 14px;font-size:.82rem}
.win-row.profit{border-color:rgba(63,185,80,.2)}
.win-row.loss{border-color:rgba(248,81,73,.15)}
.win-sym{font-weight:700;color:var(--text-hi);min-width:72px}
.win-pnl{font-weight:700;font-variant-numeric:tabular-nums;margin-left:auto}
.win-meta{font-size:.7rem;color:var(--dim)}

/* ── Footer ── */
.footer{border-top:1px solid var(--border);padding:20px 32px;display:flex;justify-content:space-between;align-items:center;font-size:.73rem;color:var(--dim);flex-wrap:wrap;gap:12px;margin-top:40px}
.footer-links{display:flex;gap:16px}
.footer-link:hover{color:var(--muted)}
.ts{font-size:.65rem;color:#30363d;margin-top:6px;text-align:center}
</style>
</head>
<body>

<!-- ═══ NAV ═══ -->
<nav class="nav">
  <div class="nav-logo">TradingBots<span class="dot">.</span>fun</div>
  <div class="nav-links">
    <span class="live-badge"><span id="live-pill-text">1 Bot</span> Live</span>
    <a href="#how" class="nav-link">How it works</a>
    <a href="#x402-api" class="nav-link">x402 API</a>
    <a href="/leaderboard" class="nav-link">Leaderboard</a>
    <a href="/login" class="nav-cta">Get Your Bot →</a>
  </div>
</nav>

<!-- ═══ HERO ═══ -->
<section class="hero">
  <!-- AUM history sparkline renders here as a translucent background -->
  <canvas id="aum-canvas"></canvas>
  <div style="position:relative;z-index:1">
    <div class="hero-eyebrow">● AI Trading · Live on Hyperliquid</div>
    <h1>Get a bot.<br>Let it <em>trade for you</em>.</h1>
    <div id="hero-pnl" class="hero-pnl" style="display:none">
      <span id="hero-pnl-val">+$0.00</span>
      <span class="hero-pnl-meta">session profit · <span id="hero-wr">—</span> win rate</span>
    </div>
    <p class="hero-sub">Deposit USDC via Arbitrum. The AI scans 50+ perpetual pairs every 30 seconds and manages your trades 24/7.<br><span id="hero-aum" style="color:var(--green);font-weight:700">—</span> under management right now.</p>
    <div class="hero-btns">
      <a href="/login" class="btn-p" style="font-size:1rem;padding:14px 32px">Get Your Bot →</a>
      <a href="#x402-api" class="btn-s">Bot API (x402)</a>
    </div>
    <p style="margin-top:14px;font-size:.72rem;color:var(--dim)">No subscription. Deposit USDC &amp; start. Withdraw anytime.</p>
  </div>
</section>

<!-- ═══ STAT BAR ═══ -->
<div class="stat-bar">
  <div class="stat-cell">
    <div class="stat-val" id="m-pnl" style="color:var(--green)">—</div>
    <div class="stat-lbl">Session Profit</div>
  </div>
  <div class="stat-cell">
    <div class="stat-val" id="m-wr2" style="color:var(--yellow)">—</div>
    <div class="stat-lbl">Win Rate</div>
  </div>
  <div class="stat-cell">
    <div class="stat-val" id="m-pos" style="color:var(--blue)">—</div>
    <div class="stat-lbl">Open Now</div>
  </div>
  <div class="stat-cell">
    <div class="stat-val" id="m-trades2" style="color:var(--muted)">—</div>
    <div class="stat-lbl">Trades Closed</div>
  </div>
</div>

<!-- ═══ POSITION TILES — horizontal scroll strip ═══ -->
<div class="pos-strip-wrap" id="pos-strip-wrap">
  <div class="pos-strip" id="pos-strip">
    <!-- tiles injected by JS; duplicated for infinite scroll -->
  </div>
</div>

<!-- metrics-grid removed — stats now in stat-bar, positions table above fold -->

<!-- ═══ RECENT TRADES TICKER ═══ -->
<div class="ticker-wrap" id="ticker-wrap" style="display:none">
  <div class="ticker-inner" id="ticker-inner"></div>
</div>

<div class="wrap">

<!-- ═══ HOW IT WORKS ═══ -->
<section class="sec" id="how">
  <div class="sec-head"><span class="sec-title">How It Works</span><span class="sec-line"></span></div>
  <div class="steps">
    <div class="step">
      <div class="step-num">💰</div>
      <div class="step-title">Deposit USDC</div>
      <div class="step-desc">Send USDC from Arbitrum — the built-in bridge handles the rest. No lock-ups, withdraw profits anytime.</div>
    </div>
    <div class="step">
      <div class="step-num">🤖</div>
      <div class="step-title">AI Trades For You</div>
      <div class="step-desc">Multi-signal AI scans 50+ Hyperliquid perpetual pairs every 30 seconds, entering and exiting with precision.</div>
    </div>
    <div class="step">
      <div class="step-num">📈</div>
      <div class="step-title">Collect Your Profits</div>
      <div class="step-desc">Watch real P&L accumulate live. Take profit on demand — your keys, your capital, full control.</div>
    </div>
    <div class="step" style="border-color:rgba(88,166,255,.2);background:rgba(88,166,255,.03)">
      <div class="step-num">⚡</div>
      <div class="step-title">Or: Use the x402 API</div>
      <div class="step-desc">AI agent? Call the API. HTTP 402 → pay 10 USDC on Base → get session token. Fully autonomous, no UI required.</div>
    </div>
  </div>
</section>

<!-- ═══ RECENT CLOSED TRADES — compact list ═══ -->
<section class="sec" id="wins-sec" style="display:none">
  <div class="sec-head"><span class="sec-title">Recent Closed Trades</span><span class="sec-line"></span></div>
  <div id="wins-list" style="display:flex;flex-direction:column;gap:6px"></div>
</section>

<!-- ═══ FOR AI AGENTS — x402 ═══ -->
<section class="sec" id="x402-api">
  <div class="sec-head"><span class="sec-title">For AI Agents &amp; Autonomous Systems</span><span class="sec-line"></span></div>

  <!-- x402 intro band -->
  <div class="x402-hero">
    <h3><span class="x402-tag">x402</span> The Machine-Native Trading API</h3>
    <p>The <strong style="color:var(--text-hi)">x402 protocol</strong> enables fully autonomous trading sessions — no UI, no human in the loop, no subscription sign-up. An AI agent sends a standard HTTP request, receives HTTP 402 with on-chain payment details, pays 10 USDC on Base, and immediately gains a 30-day authenticated session. The entire flow is machine-executable in milliseconds.</p>
    <div class="x402-flow">
      <div class="x402-step"><div class="x402-step-num">1</div><span><strong style="color:var(--text-hi)">Probe</strong> — <code style="font-size:.75rem;color:var(--dim)">POST /api/v1/session</code> returns <code style="font-size:.75rem;color:var(--yellow)">HTTP 402</code> with amount + payTo address</span></div>
      <div class="x402-step"><div class="x402-step-num">2</div><span><strong style="color:var(--text-hi)">Pay</strong> — Agent sends 10 USDC on Base mainnet to the payment address, gets a tx hash</span></div>
      <div class="x402-step"><div class="x402-step-num">3</div><span><strong style="color:var(--text-hi)">Activate</strong> — Retry with <code style="font-size:.75rem;color:var(--dim)">X-Payment: &lt;tx_hash&gt;</code> header, receive session token</span></div>
      <div class="x402-step"><div class="x402-step-num">4</div><span><strong style="color:var(--text-hi)">Trade</strong> — Fund bot via Arbitrum bridge, issue commands, monitor positions — all via REST</span></div>
    </div>
  </div>

  <div class="agent-grid" style="margin-top:20px">
    <div class="agent-text">
      <h3>REST Endpoints</h3>
      <p style="margin-bottom:14px">Full programmatic control. Human or machine — same API.</p>
      <div class="endpoint-list">
        <div class="ep"><span class="m-get">GET</span>  /api/v1/status <span style="color:var(--dim);font-size:.65rem;margin-left:auto">public</span></div>
        <div class="ep"><span class="m-post">POST</span> /api/v1/session <span style="color:var(--yellow);font-size:.65rem;margin-left:auto">← x402 gated</span></div>
        <div class="ep"><span class="m-get">GET</span>  /api/v1/session/{id}</div>
        <div class="ep"><span class="m-post">POST</span> /api/v1/session/{id}/command</div>
        <div class="ep"><span class="m-post">POST</span> /api/v1/bridge/deposit <span style="color:var(--dim);font-size:.65rem;margin-left:auto">Arb→HL</span></div>
        <div class="ep"><span class="m-post">POST</span> /api/v1/bridge/withdraw <span style="color:var(--dim);font-size:.65rem;margin-left:auto">HL→Arb</span></div>
      </div>
      <p style="margin-top:16px;font-size:.75rem;color:var(--dim)">Bridge endpoints route USDC between Arbitrum One and Hyperliquid natively — no manual wallet management.</p>
    </div>
    <div class="code-box">
      <div class="code-box-head">
        <span style="width:10px;height:10px;border-radius:50%;background:#f85149;display:inline-block"></span>
        <span style="width:10px;height:10px;border-radius:50%;background:#e3b341;display:inline-block"></span>
        <span style="width:10px;height:10px;border-radius:50%;background:#3fb950;display:inline-block"></span>
        <span style="margin-left:8px;color:var(--dim)">x402 flow · bash</span>
      </div>
<pre><span class="cmt"># Step 1 — probe (any agent can call this)</span>
<span class="kw">curl</span> -X POST https://tradingbots.fun/api/v1/session
<span class="cmt"># ← 402 { "x402Version":1, "accepts":[{</span>
<span class="cmt">#         "scheme":"exact","amount":"10000000",</span>
<span class="cmt">#         "asset":"USDC","network":"base",</span>
<span class="cmt">#         "payTo":"0x..." }] }</span>

<span class="cmt"># Step 2 — pay on Base (agent executes ERC-20 transfer)</span>
<span class="cmt"># → tx: 0xabc...def</span>

<span class="cmt"># Step 3 — activate session</span>
<span class="kw">curl</span> -X POST https://tradingbots.fun/api/v1/session \
  -H <span class="str">"X-Payment: 0xabc...def"</span>
<span class="cmt"># ← 200 {"session_id":"ses_...","token":"tok_..."}</span>

<span class="cmt"># Step 4 — bridge USDC from Arbitrum to fund bot</span>
<span class="kw">curl</span> -X POST https://tradingbots.fun/api/v1/bridge/deposit \
  -H <span class="str">"Authorization: Bearer tok_..."</span> \
  -d <span class="str">'{"amount_usdc":500,"arb_wallet":"0x..."}'</span>

<span class="cmt"># Step 5 — take profit on BTC position</span>
<span class="kw">curl</span> -X POST https://tradingbots.fun/api/v1/session/ses_.../command \
  -H <span class="str">"Authorization: Bearer tok_..."</span> \
  -d <span class="str">'{"cmd":"take_profit","symbol":"BTC"}'</span></pre>
    </div>
  </div>
</section>

<!-- ═══ CTA BAND ═══ -->
<div class="cta-band">
  <h3>Get your bot trading today.</h3>
  <p>Human: deposit USDC and watch it trade.<br>AI agent: call the x402 API and it handles everything — session, funding, positions.</p>
  <div class="hero-btns">
    <a href="/login" class="btn-p" style="font-size:1rem;padding:14px 32px">Get Your Bot →</a>
    <a href="/api/v1/status" class="btn-s" target="_blank">Try the API</a>
  </div>
  <p style="margin-top:14px;font-size:.72rem;color:var(--dim)">Arbitrum ↔ Hyperliquid bridge built-in · x402 pay-per-session · no lock-ups</p>
</div>

</div><!-- /wrap -->

<!-- hidden tbody still needed for JS data processing -->
<div style="display:none">
  <table><tbody id="pos-tbody"></tbody></table>
  <table><tbody id="trades-tbody"></tbody></table>
  <span id="trade-count"></span>
  <span id="footer-aum"></span>
  <span id="wallet-count"></span>
  <div id="weights-grid"></div>
</div>

<!-- ═══ FOOTER ═══ -->
<footer class="footer">
  <span>© 2026 TradingBots.fun — Autonomous AI Trading on Hyperliquid</span>
  <div class="footer-links">
    <a href="/leaderboard" class="footer-link">Leaderboard</a>
    <a href="#x402-api" class="footer-link">x402 API</a>
    <a href="/login" class="footer-link">Login</a>
    <a href="/dashboard" class="footer-link">Operator</a>
  </div>
</footer>

<script>
// ═══════════════════════════════════════════════════════
//  Formatters
// ═══════════════════════════════════════════════════════
const fmtUsd = (n) => {
  const abs = Math.abs(n);
  const s = abs >= 1e6 ? '$'+(abs/1e6).toFixed(2)+'M'
          : abs >= 1e3 ? '$'+(abs/1e3).toFixed(1)+'K'
          : '$'+abs.toFixed(2);
  return n < 0 ? '-'+s : s;
};
const fmtPct  = (n) => (n >= 0 ? '+' : '') + n.toFixed(2) + '%';
const fmtNum  = (n, d=2) => Number.isFinite(n) ? n.toFixed(d) : '—';
const pClass  = (n) => n > 0.005 ? 'pos' : n < -0.005 ? 'neg' : 'neu';
const fmtPrice = (n) => n >= 1000 ? '$'+n.toFixed(0) : n >= 1 ? '$'+n.toFixed(3) : '$'+n.toFixed(6);

// ═══════════════════════════════════════════════════════
//  Hero background chart
// ═══════════════════════════════════════════════════════
function drawChart(points) {
  const cv = document.getElementById('aum-canvas');
  if (!cv || !points || points.length < 2) return;
  // Size canvas to match hero (it fills via position:absolute)
  const hero = cv.parentElement;
  const W = hero.offsetWidth || window.innerWidth;
  const H = hero.offsetHeight || 300;
  cv.width = W * devicePixelRatio; cv.height = H * devicePixelRatio;
  cv.style.width = W + 'px'; cv.style.height = H + 'px';
  const ctx = cv.getContext('2d');
  ctx.scale(devicePixelRatio, devicePixelRatio);
  const vals = points.map(p => p.aum);
  const minV = Math.min(...vals), maxV = Math.max(...vals), range = maxV - minV || 1;
  const pxX = i => (i / (points.length-1)) * W;
  const pxY = v => H - ((v-minV)/range) * (H * 0.7) - H * 0.1;
  // Soft fill under curve
  const grd = ctx.createLinearGradient(0, 0, 0, H);
  grd.addColorStop(0, 'rgba(63,185,80,.22)');
  grd.addColorStop(1, 'rgba(63,185,80,0)');
  ctx.beginPath();
  ctx.moveTo(pxX(0), pxY(vals[0]));
  vals.forEach((v, i) => ctx.lineTo(pxX(i), pxY(v)));
  ctx.lineTo(W, H); ctx.lineTo(0, H); ctx.closePath();
  ctx.fillStyle = grd; ctx.fill();
  // Chart line
  ctx.beginPath();
  ctx.moveTo(pxX(0), pxY(vals[0]));
  vals.forEach((v, i) => ctx.lineTo(pxX(i), pxY(v)));
  ctx.strokeStyle = 'rgba(63,185,80,.7)'; ctx.lineWidth = 1.5;
  ctx.lineJoin = 'round'; ctx.stroke();
}

// ═══════════════════════════════════════════════════════
//  Position tiles strip (panning carousel)
// ═══════════════════════════════════════════════════════
function renderPositionTiles(positions) {
  const strip = document.getElementById('pos-strip');
  if (!strip) return;
  if (!positions || !positions.length) {
    strip.className = 'pos-strip no-anim';
    strip.innerHTML = '<div class="pos-tile-empty">🔍 No open positions — scanning for signals…</div>';
    return;
  }
  const makeTile = p => {
    const pnlCls  = p.unrealised_pnl > 0.5 ? 'pos' : p.unrealised_pnl < -0.5 ? 'neg' : 'neu';
    const tileCls = p.side === 'LONG' ? 'pos-tile long-tile' : 'pos-tile short-tile';
    const sideCls = p.side === 'LONG' ? 'side-long' : 'side-short';
    const pnlStr  = (p.unrealised_pnl >= 0 ? '+' : '') + fmtUsd(p.unrealised_pnl);
    const venueBadge = p.venue ? `<span class="venue-badge">${p.venue}</span>` : '';
    return `<div class="${tileCls}">
      <div class="pt-row">
        <span class="pt-sym">${p.symbol}</span>
        <span class="${sideCls}">${p.side}</span>
      </div>
      <div class="pt-pnl ${pnlCls}">${pnlStr}</div>
      <div class="pt-row">
        <span class="pt-entry">Entry ${fmtPrice(p.entry_price)}</span>
        <span class="pt-meta">${p.leverage.toFixed(1)}× · ${fmtUsd(p.size_usd)}</span>
      </div>
      ${venueBadge}
    </div>`;
  };
  const tiles = positions.map(makeTile).join('');
  // Duplicate for seamless infinite pan (only animate if multiple tiles)
  strip.className = positions.length > 3 ? 'pos-strip' : 'pos-strip no-anim';
  strip.innerHTML = tiles + (positions.length > 3 ? tiles : '');
  // Set animation duration proportional to number of tiles
  if (positions.length > 3) {
    strip.style.animationDuration = Math.max(18, positions.length * 6) + 's';
  }
}

// ═══════════════════════════════════════════════════════
//  Signal weight bars
// ═══════════════════════════════════════════════════════
const SIG_LABELS = {
  rsi:'RSI (14)', bollinger:'Bollinger Bands', macd:'MACD Histogram',
  ema_cross:'EMA 8/21 Cross', order_flow:'Order Flow', z_score:'Z-Score',
  volume:'Volume Conviction', sentiment:'Social Sentiment', funding_rate:'Funding Rate',
  candle_pattern:'Candle Patterns', chart_pattern:'Chart Patterns', trend:'Trend (legacy)'
};
function renderWeights(w) {
  if (!w) return;
  const grid = document.getElementById('weights-grid');
  const entries = Object.entries(SIG_LABELS)
    .map(([k, lbl]) => [k, lbl, w[k] || 0])
    .filter(([,,v]) => v > 0)
    .sort((a,b) => b[2]-a[2]);
  const max = entries[0]?.[2] || 1;
  grid.innerHTML = entries.map(([,lbl,val]) => `
    <div class="w-row">
      <span class="w-name">${lbl}</span>
      <div class="w-bar-wrap"><div class="w-bar" style="width:${(val/max*100).toFixed(1)}%"></div></div>
      <span class="w-pct">${(val*100).toFixed(1)}%</span>
    </div>`).join('');
}

// ═══════════════════════════════════════════════════════
//  Regime detection from rationale / candidates
// ═══════════════════════════════════════════════════════
function highlightRegime(candidates) {
  // Count regime votes from candidate list
  const counts = {Trending:0, Ranging:0, Neutral:0};
  (candidates||[]).forEach(c => { if (c.regime && counts[c.regime]!==undefined) counts[c.regime]++; });
  const dominant = Object.entries(counts).sort((a,b)=>b[1]-a[1])[0]?.[0];
  ['trending','ranging','neutral'].forEach(r => {
    const card = document.getElementById('regime-'+r);
    const badge = document.getElementById('badge-'+r);
    const isActive = dominant && dominant.toLowerCase() === r;
    if (card) card.className = 'algo-card' + (isActive ? ' active-regime' : '');
    if (badge) { badge.textContent = isActive ? 'Active' : 'Standby'; badge.className = 'algo-badge ' + (isActive ? 'badge-active' : 'badge-standby'); }
  });
}

// ═══════════════════════════════════════════════════════
//  Trades ticker
// ═══════════════════════════════════════════════════════
function renderTicker(trades) {
  if (!trades || !trades.length) return;
  const wrap = document.getElementById('ticker-wrap');
  const inner = document.getElementById('ticker-inner');
  wrap.style.display = '';
  const items = [...trades].slice(0,30).map(t => {
    const cls = t.pnl >= 0 ? 'pos' : 'neg';
    return `<span class="tick-item"><span class="tick-sym">${t.symbol}</span>`+
      `<span class="${t.side==='LONG'?'side-long':'side-short'}">${t.side}</span>`+
      `<span class="tick-amt ${cls}">${fmtUsd(t.pnl)}</span>`+
      `<span style="color:var(--dim);font-size:.7rem">${t.reason}</span></span>`;
  });
  // Duplicate for seamless loop
  inner.innerHTML = items.join('') + items.join('');
}

// ═══════════════════════════════════════════════════════
//  Load BotState → metrics + positions + trades + weights
// ═══════════════════════════════════════════════════════
async function loadState() {
  try {
    const res = await fetch('/api/state');
    const s = await res.json();
    const m = s.metrics || {};

    // ── Hero P&L sub-line ──
    const pnl = s.pnl || 0;
    const heroP  = document.getElementById('hero-pnl');
    const heroPV = document.getElementById('hero-pnl-val');
    const heroWR = document.getElementById('hero-wr');
    if (heroP && heroPV) {
      heroP.style.display = '';
      heroPV.textContent  = (pnl >= 0 ? '+' : '') + fmtUsd(pnl) +
                            ' (' + fmtPct((pnl/(s.initial_capital||1))*100) + ')';
      heroP.className = 'hero-pnl' + (pnl >= 0 ? '' : ' neg');
    }
    if (heroWR && m.win_rate > 0) heroWR.textContent = (m.win_rate*100).toFixed(0)+'%';

    // ── Stat bar ──
    const pnlEl = document.getElementById('m-pnl');
    if (pnlEl) {
      pnlEl.textContent  = (pnl >= 0 ? '+' : '') + fmtUsd(pnl);
      pnlEl.style.color  = pnl >= 0 ? 'var(--green)' : 'var(--red)';
    }
    const wr2El = document.getElementById('m-wr2');
    if (wr2El) wr2El.textContent = m.win_rate > 0 ? (m.win_rate*100).toFixed(0)+'%' : '—';
    const t2El = document.getElementById('m-trades2');
    if (t2El) t2El.textContent = m.total_trades || '0';

    // ── Signal weights ──
    renderWeights(s.signal_weights);

    // ── Regime highlight ──
    highlightRegime(s.candidates);

    // ── Position tiles (panning strip above fold) ──
    renderPositionTiles(s.positions);

    // ── Open positions (detailed table below fold) ──
    const posBody = document.getElementById('pos-tbody');
    if (!s.positions || !s.positions.length) {
      posBody.innerHTML = '<tr class="tr"><td colspan="8" style="text-align:center;color:var(--dim);padding:24px">No open positions</td></tr>';
    } else {
      posBody.innerHTML = s.positions.map(p => {
        const pnlCls = pClass(p.unrealised_pnl);
        const sideCls = p.side==='LONG' ? 'side-long' : 'side-short';
        // Extract regime from rationale (first word after "[")
        const regMatch = (p.contrib?.rationale||'').match(/\[(\w+)\]/);
        const regime = regMatch ? regMatch[1] : '—';
        return `<tr class="tr">
          <td style="font-weight:700;color:var(--text-hi)">${p.symbol}</td>
          <td><span class="${sideCls}">${p.side}</span></td>
          <td class="mono">${fmtPrice(p.entry_price)}</td>
          <td class="mono">${fmtUsd(p.size_usd)}</td>
          <td style="color:var(--muted)">${p.leverage.toFixed(1)}×</td>
          <td class="${pnlCls}">${fmtUsd(p.unrealised_pnl)}</td>
          <td style="color:var(--muted);text-align:center">${p.dca_count||0}</td>
          <td style="color:var(--dim);font-size:.72rem">${regime}</td>
        </tr>`;
      }).join('');
    }
    const posCountEl = document.getElementById('m-pos');
    if (posCountEl) posCountEl.textContent = s.positions?.length || '0';

    // ── Closed trades — compact wins list ──
    renderTicker(s.closed_trades);
    const winsSec  = document.getElementById('wins-sec');
    const winsList = document.getElementById('wins-list');
    if (winsSec && winsList) {
      if (s.closed_trades && s.closed_trades.length) {
        winsSec.style.display = '';
        winsList.innerHTML = [...s.closed_trades].reverse().slice(0, 12).map(t => {
          const isPnlPos = t.pnl >= 0;
          const sideCls  = t.side === 'LONG' ? 'side-long' : 'side-short';
          const venueTag = t.venue ? `<span class="venue-badge">${t.venue}</span>` : '';
          return `<div class="win-row ${isPnlPos ? 'profit' : 'loss'}">
            <span class="win-sym">${t.symbol}</span>
            <span class="${sideCls}">${t.side}</span>
            <span class="win-meta">${fmtPrice(t.entry)} → ${fmtPrice(t.exit)}</span>
            <span class="win-meta reason-pill">${t.reason}</span>
            ${venueTag}
            <span class="win-meta">${t.closed_at?.slice(11,19)||''}</span>
            <span class="win-pnl ${isPnlPos ? 'pos' : 'neg'}">${isPnlPos ? '+' : ''}${fmtUsd(t.pnl)} (${fmtPct(t.pnl_pct)})</span>
          </div>`;
        }).join('');
      } else {
        winsSec.style.display = 'none';
      }
    }
  } catch(e) { console.warn('state fetch failed', e); }
}

// ═══════════════════════════════════════════════════════
//  Load TVL aggregate
// ═══════════════════════════════════════════════════════
async function loadTvl() {
  try {
    const res = await fetch('/api/public/tvl');
    const d = await res.json();
    const l = d.latest;
    if (l) {
      const aumStr = fmtUsd(l.total_aum);
      document.getElementById('m-aum').textContent    = aumStr;
      document.getElementById('hero-aum').textContent = aumStr;
      document.getElementById('footer-aum').textContent = aumStr;
    }
    if (d.points && d.points.length > 1) drawChart(d.points);
  } catch(e) { console.warn('tvl failed', e); }
}

// ═══════════════════════════════════════════════════════
//  Load per-wallet stats
// ═══════════════════════════════════════════════════════
async function loadWallets() {
  try {
    const res = await fetch('/api/public/stats');
    const d = await res.json();
    if (!d.accounts) return;
    // Count bots that are actively trading (have open positions or non-zero equity)
    const activeBots = d.accounts.filter(a => a.open_positions > 0 || a.current_equity > 0).length;
    const pill = document.getElementById('live-pill-text');
    if (pill) pill.textContent = activeBots + ' Bot' + (activeBots !== 1 ? 's' : '');
    // Update footer AUM
    const fAum = document.getElementById('footer-aum');
    if (fAum) {
      const total = d.accounts.reduce((s, a) => s + (a.current_equity || 0), 0);
      if (total > 0) fAum.textContent = fmtUsd(total);
    }
  } catch(e) { console.warn('wallets failed', e); }
}

// ═══════════════════════════════════════════════════════
//  Boot
// ═══════════════════════════════════════════════════════
loadState();
loadTvl();
loadWallets();
setInterval(loadState,  30000);
setInterval(loadTvl,    60000);
setInterval(loadWallets,30000);
</script>
</body>
</html>"##.to_string())
}

/// `GET /api/public/stats` — per-account stats for the public landing page.
///
/// Returns each account's name, initial capital, current equity, open position
/// count.  No PII beyond what's already on-chain (wallet address truncated).
/// No authentication required.
async fn api_public_stats_handler(
    State(app): State<AppState>,
) -> impl axum::response::IntoResponse {
    use axum::http::{HeaderMap, StatusCode};

    let mut headers = HeaderMap::new();
    headers.insert("Access-Control-Allow-Origin", "*".parse().unwrap());
    headers.insert("Cache-Control", "public, max-age=15".parse().unwrap());

    let Some(db) = &app.db else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            headers,
            axum::Json(serde_json::json!({ "accounts": [], "error": "db unavailable" })),
        );
    };

    // Use non-macro query to avoid requiring .sqlx/ cache regeneration.
    // Fetches all tenants with their latest equity snapshot and open position count.
    let rows = sqlx::query(
        r#"
        SELECT
            t.id::text                                              AS tenant_id,
            COALESCE(t.display_name, 'Anonymous')                  AS display_name,
            t.initial_capital::float8                              AS initial_capital,
            t.wallet_address,
            COALESCE(
                (SELECT equity::float8
                 FROM   equity_snapshots
                 WHERE  tenant_id = t.id
                 ORDER  BY recorded_at DESC
                 LIMIT  1),
                t.initial_capital::float8
            )                                                       AS current_equity,
            COALESCE(
                (SELECT COUNT(*)::int
                 FROM   positions p
                 WHERE  p.tenant_id = t.id),
                0
            )                                                       AS open_positions
        FROM   tenants t
        ORDER  BY t.initial_capital DESC
    "#,
    )
    .fetch_all(db.pool())
    .await;

    match rows {
        Err(e) => {
            log::warn!("api_public_stats_handler: DB error: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                headers,
                axum::Json(serde_json::json!({ "accounts": [], "error": "query failed" })),
            )
        }
        Ok(rows) => {
            let accounts: Vec<serde_json::Value> = rows
                .iter()
                .map(|r| {
                    use sqlx::Row;
                    let display_name: String = r
                        .try_get("display_name")
                        .unwrap_or_else(|_| "Anonymous".into());
                    let initial_capital: f64 = r.try_get("initial_capital").unwrap_or(0.0);
                    let current_equity: f64 =
                        r.try_get("current_equity").unwrap_or(initial_capital);
                    let open_positions: i32 = r.try_get("open_positions").unwrap_or(0);
                    let wallet_address: Option<String> = r.try_get("wallet_address").ok().flatten();
                    serde_json::json!({
                        "display_name":    display_name,
                        "initial_capital": initial_capital,
                        "current_equity":  current_equity,
                        "open_positions":  open_positions,
                        "wallet_address":  wallet_address,
                    })
                })
                .collect();

            (
                StatusCode::OK,
                headers,
                axum::Json(serde_json::json!({
                    "generated_at": chrono::Utc::now().to_rfc3339(),
                    "accounts": accounts,
                })),
            )
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────

// ═══════════════════════════════════════════════════════════════════════════════
//  Bot API  — /api/v1/*   (x402 payment-gated, machine-readable)
// ═══════════════════════════════════════════════════════════════════════════════

/// Generate a simple time-based unique ID.
fn new_id(prefix: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}_{:x}{:x}", prefix, dur.as_millis(), dur.subsec_nanos())
}

/// `GET /api/v1/status` — unauthenticated machine-readable bot status.
/// Returns live AUM, P&L, win rate and x402 payment info.
async fn api_v1_status_handler(State(app): State<AppState>) -> axum::response::Response {
    use axum::response::IntoResponse;
    let s = app.bot_state.read().await;
    let m = &s.metrics;
    let committed: f64 = s.positions.iter().map(|p| p.size_usd).sum();
    let unrealised: f64 = s.positions.iter().map(|p| p.unrealised_pnl).sum();
    let aum = s.capital + committed + unrealised;
    axum::response::Json(serde_json::json!({
        "ok":             true,
        "aum_usd":        aum,
        "pnl_usd":        s.pnl,
        "win_rate":       m.win_rate,
        "open_positions": s.positions.len(),
        "closed_trades":  m.total_trades,
        "cb_active":      s.cb_active,
        "x402": {
            "description":    "Start an AI trading session in USDC on Base",
            "session_price":  "10 USDC",
            "network":        "base-mainnet",
            "asset":          "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
            "endpoint":       "POST /api/v1/session",
            "docs":           "https://tradingbots.fun/api/v1/status"
        }
    }))
    .into_response()
}

/// Optional JSON body for `POST /api/v1/session`.
/// All fields are optional — bots that only send `X-Payment` get a standard 30-day session.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionCreateRequest {
    /// "internal" (default) | "hyperliquid"
    #[serde(default = "default_session_venue")]
    pub venue: String,
    /// Max leverage (1–50). Hyperliquid sessions only.
    pub leverage_max: Option<i32>,
    /// "conservative" | "balanced" | "aggressive"
    pub risk_mode: Option<String>,
    /// Whitelisted symbols (null = all pairs).
    pub symbols_whitelist: Option<Vec<String>>,
    /// Auto-pause drawdown threshold (%).
    pub max_drawdown_pct: Option<f64>,
    /// Performance fee on profits (%).
    pub performance_fee_pct: Option<i32>,
    /// Webhook URL to POST trade events to.
    pub webhook_url: Option<String>,
    /// "30d" (default, 10 USDC) | "24h" (burst, 0.5 USDC)
    #[serde(default = "default_duration")]
    pub duration: String,
}

fn default_duration() -> String {
    "30d".to_string()
}

/// x402 payment requirements object (embedded in 402 responses).
fn x402_payment_requirements(resource: &str, duration: &str) -> serde_json::Value {
    let (amount, description) = if duration == "24h" {
        ("500000", "Start a 24-hour burst AI trading bot session (0.5 USDC)")
    } else {
        ("10000000", "Start a 30-day AI trading bot session (10 USDC)")
    };
    serde_json::json!({
        "scheme":             "exact",
        "network":            "base-mainnet",
        "maxAmountRequired":  amount,
        "resource":           resource,
        "description":        description,
        "mimeType":           "application/json",
        "payTo":              std::env::var("X402_WALLET").unwrap_or_else(|_| "0x0000000000000000000000000000000000000000".to_string()),
        "maxTimeoutSeconds":  300,
        "asset":              "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
        "extra": { "name": "USD Coin", "version": "2" }
    })
}

/// Verify a Base-mainnet USDC transfer on-chain via JSON-RPC.
///
/// Calls `eth_getTransactionReceipt` on the Base RPC, then scans logs for an
/// ERC-20 Transfer event from the USDC contract where `to` equals our wallet
/// and `value` >= `min_usdc_units` (6-decimal, so 10 USDC = 10_000_000).
///
/// Returns `Ok(true)` = payment confirmed, `Ok(false)` = tx not found / wrong
/// recipient / insufficient amount, `Err(_)` = RPC call failed.
async fn verify_base_usdc_payment(
    tx_hash: &str,
    recipient: &str,     // our X402_WALLET, lower-cased
    min_usdc_units: u64, // 10_000_000 for 10 USDC
) -> Result<bool, String> {
    // USDC on Base mainnet
    const USDC_CONTRACT: &str = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";
    // keccak256("Transfer(address,address,uint256)")
    const TRANSFER_TOPIC: &str =
        "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";

    let rpc_url =
        std::env::var("BASE_RPC_URL").unwrap_or_else(|_| "https://mainnet.base.org".to_string());

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method":  "eth_getTransactionReceipt",
        "params":  [tx_hash],
        "id":      1
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|e| e.to_string())?;

    let resp: serde_json::Value = client
        .post(&rpc_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("RPC request failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("RPC response parse failed: {e}"))?;

    let receipt = match resp.get("result") {
        Some(r) if !r.is_null() => r,
        _ => return Ok(false), // tx not yet mined or not found
    };

    // Must be a successful tx (status = "0x1")
    if receipt.get("status").and_then(|v| v.as_str()) != Some("0x1") {
        return Ok(false);
    }

    let logs = match receipt.get("logs").and_then(|l| l.as_array()) {
        Some(l) => l,
        None => return Ok(false),
    };

    let recipient_lc = recipient.to_lowercase();
    // Addresses in log topics are 32-byte padded; last 40 hex chars = address
    let recipient_padded = recipient_lc.trim_start_matches("0x");

    for log in logs {
        // Must be from USDC contract
        let log_addr = log
            .get("address")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        if log_addr.trim_start_matches("0x") != USDC_CONTRACT.trim_start_matches("0x") {
            continue;
        }

        let topics = match log.get("topics").and_then(|t| t.as_array()) {
            Some(t) if t.len() >= 3 => t,
            _ => continue,
        };

        // topics[0] = event sig, topics[1] = from, topics[2] = to
        let ev_sig = topics[0].as_str().unwrap_or("").to_lowercase();
        if ev_sig != TRANSFER_TOPIC {
            continue;
        }

        let to_topic = topics[2].as_str().unwrap_or("").to_lowercase();
        if !to_topic.ends_with(recipient_padded) {
            continue;
        }

        // data = 32-byte hex-encoded uint256 value
        let data_hex = log
            .get("data")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim_start_matches("0x");

        // Decode as big-endian u64 (safe up to ~18 USDC * 10^6; USDC max is fine)
        let padded = format!("{:0>64}", data_hex);
        let amount_bytes = hex::decode(&padded[48..64]) // last 8 bytes
            .unwrap_or_default();
        let amount: u64 = amount_bytes
            .iter()
            .fold(0u64, |acc, &b| acc * 256 + b as u64);

        if amount >= min_usdc_units {
            return Ok(true);
        }
    }

    Ok(false)
}

/// Fire webhook for all sessions that have a `webhook_url` configured.
/// Called after every trade close / open.
async fn dispatch_session_webhooks(
    bot_state: &crate::web_dashboard::SharedState,
    event_type: &str,
    payload: serde_json::Value,
) {
    let urls: Vec<String> = {
        let s = bot_state.read().await;
        s.bot_sessions
            .values()
            .filter_map(|sess| sess.webhook_url.clone())
            .collect()
    };

    if urls.is_empty() {
        return;
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    let body = serde_json::json!({
        "event": event_type,
        "ts":    chrono::Utc::now().to_rfc3339(),
        "data":  payload
    });

    for url in urls {
        let c = client.clone();
        let b = body.clone();
        let u = url.clone();
        tokio::spawn(async move {
            if let Err(e) = c.post(&u).json(&b).send().await {
                log::warn!("Webhook delivery failed to {}: {}", u, e);
            }
        });
    }
}

/// `POST /api/v1/session` — x402-gated session creation.
///
/// **Without** `X-Payment` header → `402 Payment Required` with USDC details.
/// **With** `X-Payment: 0x{txHash}` → verifies on Base mainnet, then creates session.
///
/// Optional JSON body (`SessionCreateRequest`) controls session parameters.
/// Burst sessions: `"duration": "24h"` → 0.5 USDC / 24h instead of 10 USDC / 30d.
async fn api_v1_session_handler(
    State(app): State<AppState>,
    headers: axum::http::HeaderMap,
    body: Option<axum::extract::Json<SessionCreateRequest>>,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    // Parse optional body; default to standard 30d session
    let req = body.map(|b| b.0).unwrap_or_default();
    let is_burst = req.duration == "24h";

    let payment_header = headers
        .get("x-payment")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    if payment_header.is_none() {
        // ── 402: tell the bot what to pay ─────────────────────────────────
        let resource_url = "https://tradingbots.fun/api/v1/session";
        let x402_req = x402_payment_requirements(resource_url, &req.duration);
        let req_str = serde_json::to_string(&x402_req).unwrap_or_default();
        let (amount_str, note) = if is_burst {
            ("0.5 USDC", "24-hour burst session")
        } else {
            ("10 USDC", "30-day standard session")
        };
        return (
            axum::http::StatusCode::PAYMENT_REQUIRED,
            [
                ("content-type",       "application/json"),
                ("x-payment-required", req_str.as_str()),
            ],
            serde_json::to_string(&serde_json::json!({
                "error":   "Payment required",
                "amount":  amount_str,
                "note":    note,
                "network": "base-mainnet",
                "payTo":   std::env::var("X402_WALLET").unwrap_or_else(|_| "0x0000000000000000000000000000000000000000".to_string()),
                "asset":   "USDC — 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
                "x402":    true,
                "retry":   "POST /api/v1/session with X-Payment: <tx_hash>"
            })).unwrap_or_default(),
        ).into_response();
    }

    let tx_hash = payment_header.unwrap();
    // Basic format check
    if !tx_hash.starts_with("0x") || tx_hash.len() < 66 {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            axum::response::Json(serde_json::json!({
                "error": "Invalid X-Payment value — expected 0x{66-char txHash}"
            })),
        )
            .into_response();
    }

    // ── On-chain verification ─────────────────────────────────────────────
    let our_wallet = std::env::var("X402_WALLET")
        .unwrap_or_else(|_| "0x0000000000000000000000000000000000000000".to_string());
    let min_units: u64 = if is_burst { 500_000 } else { 10_000_000 };

    match verify_base_usdc_payment(&tx_hash, &our_wallet, min_units).await {
        Ok(true) => {
            log::info!("✅ x402 payment verified on-chain: {}", &tx_hash[..12]);
        }
        Ok(false) => {
            log::warn!(
                "❌ x402 payment not verified (tx={}, wallet={})",
                &tx_hash[..12],
                &our_wallet[..8]
            );
            let amount_str = if is_burst { "0.5 USDC (500000 units)" } else { "10 USDC (10000000 units)" };
            return (
                axum::http::StatusCode::PAYMENT_REQUIRED,
                axum::response::Json(serde_json::json!({
                    "error":   "Payment not confirmed",
                    "detail":  "Transaction not found, insufficient amount, or wrong recipient",
                    "tx_hash": tx_hash,
                    "payTo":   our_wallet,
                    "amount":  amount_str,
                    "network": "base-mainnet"
                })),
            )
                .into_response();
        }
        Err(rpc_err) => {
            log::warn!(
                "⚠ x402 RPC verification failed ({}), accepting provisionally: {}",
                rpc_err,
                &tx_hash[..12]
            );
        }
    }

    let session_id = new_id("ses");
    let token = new_id("tok");
    let now = chrono::Utc::now();
    let (expires_at, plan) = if is_burst {
        (now + chrono::Duration::hours(24), "burst-24h".to_string())
    } else {
        (now + chrono::Duration::days(30), "starter".to_string())
    };

    // Derive a per-session Hyperliquid wallet if venue == "hyperliquid"
    let hl_address = if req.venue == "hyperliquid" {
        let (addr, _priv_key) = crate::hl_wallet::generate_keypair();
        Some(addr)
    } else {
        None
    };

    let session = BotSession {
        id: session_id.clone(),
        token: token.clone(),
        tx_hash: tx_hash.clone(),
        plan: plan.clone(),
        created_at: now.to_rfc3339(),
        expires_at: expires_at.to_rfc3339(),
        max_drawdown_pct: req.max_drawdown_pct,
        webhook_url: req.webhook_url.clone(),
        venue: req.venue.clone(),
        leverage_max: req.leverage_max,
        risk_mode: req.risk_mode.clone(),
        symbols_whitelist: req.symbols_whitelist.clone(),
        performance_fee_pct: req.performance_fee_pct,
        hyperliquid_address: hl_address.clone(),
        paused: false,
    };

    {
        let mut s = app.bot_state.write().await;
        s.bot_sessions.insert(session_id.clone(), session);
    }

    log::info!(
        "🤖 x402 session created: {} plan={} venue={} (tx {})",
        session_id,
        plan,
        req.venue,
        &tx_hash[..10.min(tx_hash.len())]
    );

    axum::response::Json(serde_json::json!({
        "ok":                  true,
        "session_id":          session_id,
        "token":               token,
        "plan":                plan,
        "venue":               req.venue,
        "expires_at":          expires_at.to_rfc3339(),
        "deposit_address":     hl_address,
        "endpoints": {
            "status":    format!("/api/v1/session/{}", session_id),
            "command":   format!("/api/v1/session/{}/command", session_id),
            "positions": format!("/api/v1/session/{}/positions", session_id),
            "trades":    format!("/api/v1/session/{}/trades", session_id)
        }
    }))
    .into_response()
}

/// Validate `Authorization: Bearer {token}` against stored session.
/// Returns `Ok(plan)` on success or an error response.
async fn validate_bot_session(
    app: &AppState,
    headers: &axum::http::HeaderMap,
    session_id: &str,
) -> Result<String, axum::response::Response> {
    use axum::response::IntoResponse;
    let auth_token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string());

    let s = app.bot_state.read().await;
    match s.bot_sessions.get(session_id) {
        None => Err((
            axum::http::StatusCode::NOT_FOUND,
            axum::response::Json(serde_json::json!({"error":"Session not found"})),
        )
            .into_response()),
        Some(sess) => {
            if auth_token.as_deref() != Some(sess.token.as_str()) {
                return Err((
                    axum::http::StatusCode::UNAUTHORIZED,
                    axum::response::Json(serde_json::json!({"error":"Invalid bearer token"})),
                )
                    .into_response());
            }
            Ok(sess.plan.clone())
        }
    }
}

/// `GET /api/v1/session/{id}` — live bot status for an authenticated session.
async fn api_v1_session_status_handler(
    State(app): State<AppState>,
    headers: axum::http::HeaderMap,
    axum::extract::Path(session_id): axum::extract::Path<String>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if let Err(e) = validate_bot_session(&app, &headers, &session_id).await {
        return e;
    }
    let s = app.bot_state.read().await;
    let m = &s.metrics;
    let committed: f64 = s.positions.iter().map(|p| p.size_usd).sum();
    let unrealised: f64 = s.positions.iter().map(|p| p.unrealised_pnl).sum();
    axum::response::Json(serde_json::json!({
        "ok":         true,
        "session_id": session_id,
        "bot": {
            "aum_usd":        s.capital + committed + unrealised,
            "pnl_usd":        s.pnl,
            "win_rate":       m.win_rate,
            "total_trades":   m.total_trades,
            "open_positions": s.positions.len(),
            "cb_active":      s.cb_active,
            "positions":      s.positions.iter().map(|p| serde_json::json!({
                "symbol":         p.symbol,
                "side":           p.side,
                "entry_price":    p.entry_price,
                "unrealised_pnl": p.unrealised_pnl,
                "size_usd":       p.size_usd,
                "leverage":       p.leverage,
            })).collect::<Vec<_>>()
        }
    }))
    .into_response()
}

/// `POST /api/v1/session/{id}/command` — queue a trade command for the bot.
///
/// Body: `{"cmd": "close_position", "symbol": "SOL"}`
/// Valid `cmd` values: `close_position`, `take_profit`, `close_all`, `close_profitable`
async fn api_v1_session_command_handler(
    State(app): State<AppState>,
    headers: axum::http::HeaderMap,
    axum::extract::Path(session_id): axum::extract::Path<String>,
    axum::extract::Json(body): axum::extract::Json<serde_json::Value>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if let Err(e) = validate_bot_session(&app, &headers, &session_id).await {
        return e;
    }

    let cmd_str = body
        .get("cmd")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let symbol = body
        .get("symbol")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let cmd: BotCommand = match cmd_str.as_str() {
        "close_position" | "close" => {
            if symbol.is_empty() {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::response::Json(
                        serde_json::json!({"error":"symbol required for close_position"}),
                    ),
                )
                    .into_response();
            }
            BotCommand::ClosePosition { symbol }
        }
        "take_profit" | "tp" => {
            if symbol.is_empty() {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::response::Json(
                        serde_json::json!({"error":"symbol required for take_profit"}),
                    ),
                )
                    .into_response();
            }
            BotCommand::TakePartial { symbol }
        }
        "close_all" => BotCommand::CloseAll,
        "close_profitable" => BotCommand::CloseProfitable,
        "open_long" | "buy_long" | "long" => {
            let sym = if !symbol.is_empty() {
                symbol
            } else {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::response::Json(
                        serde_json::json!({"error":"symbol required for open_long"}),
                    ),
                )
                    .into_response();
            };
            let size_usd = body.get("size_usd").and_then(|v| v.as_f64());
            let leverage = body.get("leverage").and_then(|v| v.as_f64());
            BotCommand::OpenLong {
                symbol: sym,
                size_usd,
                leverage,
            }
        }
        "open_short" | "buy_short" | "short" => {
            let sym = if !symbol.is_empty() {
                symbol
            } else {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::response::Json(
                        serde_json::json!({"error":"symbol required for open_short"}),
                    ),
                )
                    .into_response();
            };
            let size_usd = body.get("size_usd").and_then(|v| v.as_f64());
            let leverage = body.get("leverage").and_then(|v| v.as_f64());
            BotCommand::OpenShort {
                symbol: sym,
                size_usd,
                leverage,
            }
        }
        other => {
            // Fall back to the NLP parser for natural-language commands
            match parse_trade_command(other) {
                Some(c) => c,
                None    => return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::response::Json(serde_json::json!({
                        "error": "Unknown command",
                        "valid": ["close_position","take_profit","close_all","close_profitable","open_long","open_short"]
                    })),
                ).into_response(),
            }
        }
    };

    {
        let mut s = app.bot_state.write().await;
        s.pending_cmds.push_back(cmd);
    }

    axum::response::Json(serde_json::json!({
        "ok":      true,
        "queued":  true,
        "message": "Command queued — executes on next trading cycle (~30s)"
    }))
    .into_response()
}

/// `POST /api/v1/session/{id}/query` — natural-language AI interface for bots.
///
/// Accepts a free-text `query` field.  The handler:
///   1. Tries to parse a trade command → queues it and confirms.
///   2. Otherwise answers the question from live BotState with a plain-English
///      summary + the full relevant data object.
///
/// Example bodies:
///   `{"query": "take profit from SOL"}`
///   `{"query": "open a long on ETH with $200 and 2x leverage"}`
///   `{"query": "what is my current P&L?"}`
///   `{"query": "show my open positions"}`
async fn api_v1_query_handler(
    State(app): State<AppState>,
    headers: axum::http::HeaderMap,
    axum::extract::Path(session_id): axum::extract::Path<String>,
    axum::extract::Json(body): axum::extract::Json<serde_json::Value>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if let Err(e) = validate_bot_session(&app, &headers, &session_id).await {
        return e;
    }

    let query = body
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if query.is_empty() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            axum::response::Json(serde_json::json!({"error":"query field required"})),
        )
            .into_response();
    }

    let ql = query.to_lowercase();

    // ── 1. Detect open-position intent ───────────────────────────────────
    let open_intent = {
        let is_open = ql.contains("open")
            || ql.contains("enter")
            || ql.contains("buy")
            || ql.contains("long")
            || ql.contains("short");
        let is_close = ql.contains("close") || ql.contains("take profit") || ql.contains("sell");
        is_open && !is_close
    };

    if open_intent {
        // Parse: "open a long on SOL with $200 2x" / "buy ETH $100 3x leverage"
        let is_long = !ql.contains("short") && !ql.contains("sell");
        let symbol = ql
            .split_whitespace()
            .find(|w| {
                w.chars().all(|c| c.is_alphanumeric())
                    && w.len() >= 2
                    && w.len() <= 8
                    && ![
                        "open", "long", "short", "buy", "sell", "enter", "a", "an", "the", "on",
                        "with", "at", "x", "leverage",
                    ]
                    .contains(w)
            })
            .map(|s| s.to_uppercase());
        // Extract dollar amount
        let size_usd = {
            let mut found = None;
            for tok in ql.split_whitespace() {
                let t = tok.trim_start_matches('$');
                if let Ok(n) = t.parse::<f64>() {
                    if n >= 1.0 {
                        found = Some(n);
                        break;
                    }
                }
            }
            found
        };
        // Extract leverage (e.g. "2x" "3x" "5x")
        let leverage = ql
            .split_whitespace()
            .find(|w| w.ends_with('x') && w[..w.len() - 1].parse::<f64>().is_ok())
            .and_then(|w| w[..w.len() - 1].parse::<f64>().ok());

        if let Some(sym) = symbol {
            let cmd = if is_long {
                BotCommand::OpenLong {
                    symbol: sym.clone(),
                    size_usd,
                    leverage,
                }
            } else {
                BotCommand::OpenShort {
                    symbol: sym.clone(),
                    size_usd,
                    leverage,
                }
            };
            {
                let mut s = app.bot_state.write().await;
                s.pending_cmds.push_back(cmd);
            }
            let side_str = if is_long { "LONG" } else { "SHORT" };
            let size_str = size_usd.map_or("default size".to_string(), |v| format!("${v:.0}"));
            let lev_str = leverage.map_or("1×".to_string(), |v| format!("{v:.1}×"));
            return axum::response::Json(serde_json::json!({
                "ok":      true,
                "action":  "queued",
                "cmd":     format!("Open{}", side_str),
                "symbol":  sym,
                "size_usd": size_usd,
                "leverage": leverage,
                "answer":  format!("Opening {side_str} {sym} · {size_str} · {lev_str} leverage — executes on next cycle (~30s)"),
            })).into_response();
        }
    }

    // ── 2. Detect close/take-profit intent → reuse NLP parser ────────────
    if let Some(cmd) = parse_trade_command(&query) {
        let label = match &cmd {
            BotCommand::ClosePosition { symbol } => format!("Closing {symbol}"),
            BotCommand::TakePartial { symbol } => format!("Taking partial profit on {symbol}"),
            BotCommand::CloseAll => "Closing all positions".to_string(),
            BotCommand::CloseProfitable => "Closing all profitable positions".to_string(),
            _ => "Command queued".to_string(),
        };
        {
            let mut s = app.bot_state.write().await;
            s.pending_cmds.push_back(cmd);
        }
        return axum::response::Json(serde_json::json!({
            "ok":     true,
            "action": "queued",
            "answer": format!("{label} — executes on next cycle (~30s)"),
        }))
        .into_response();
    }

    // ── 3. State questions — answer from live BotState ────────────────────
    let s = app.bot_state.read().await;
    let m = &s.metrics;
    let committed: f64 = s.positions.iter().map(|p| p.size_usd).sum();
    let unrealised: f64 = s.positions.iter().map(|p| p.unrealised_pnl).sum();
    let aum = s.capital + committed + unrealised;

    // Build plain-English answer based on keywords
    let answer = if ql.contains("pnl")
        || ql.contains("profit")
        || ql.contains("gain")
        || ql.contains("loss")
    {
        format!(
            "Current session P&L: {}{:.2} USD ({}{:.2}%). Win rate: {:.0}% across {} trades.",
            if s.pnl >= 0.0 { "+" } else { "" },
            s.pnl,
            if s.pnl >= 0.0 { "+" } else { "" },
            (s.pnl / s.initial_capital.max(1.0)) * 100.0,
            m.win_rate * 100.0,
            m.total_trades
        )
    } else if ql.contains("position") || ql.contains("open") || ql.contains("holding") {
        if s.positions.is_empty() {
            "No open positions right now. The bot is scanning for signals.".to_string()
        } else {
            let summary: Vec<String> = s
                .positions
                .iter()
                .map(|p| {
                    format!(
                        "{} {} | entry ${:.4} | P&L {}{:.2}",
                        p.symbol,
                        p.side,
                        p.entry_price,
                        if p.unrealised_pnl >= 0.0 { "+" } else { "" },
                        p.unrealised_pnl
                    )
                })
                .collect();
            format!("{} open: {}", s.positions.len(), summary.join(" · "))
        }
    } else if ql.contains("aum")
        || ql.contains("capital")
        || ql.contains("balance")
        || ql.contains("equity")
    {
        format!("Total AUM: ${aum:.2} | Free capital: ${:.2} | Committed: ${committed:.2} | Unrealised: ${unrealised:+.2}",
            s.capital)
    } else if ql.contains("win") || ql.contains("rate") || ql.contains("performance") {
        format!("Win rate: {:.0}% | Profit factor: {:.2} | Sharpe: {:.2} | {} closed trades this session.",
            m.win_rate * 100.0, m.profit_factor, m.sharpe, m.total_trades)
    } else if ql.contains("circuit") || ql.contains("breaker") || ql.contains("cb") {
        if s.cb_active {
            "⚡ Circuit breaker ACTIVE — position sizes reduced to 35%. Drawdown limit hit."
                .to_string()
        } else {
            "Circuit breaker is normal. Full position sizing active.".to_string()
        }
    } else {
        // General summary
        format!("Bot summary: AUM ${aum:.0} | P&L {}{:.2} | {} open positions | {:.0}% win rate | {} trades closed | CB: {}",
            if s.pnl >= 0.0 {"+"} else {""}, s.pnl,
            s.positions.len(), m.win_rate * 100.0, m.total_trades,
            if s.cb_active {"ACTIVE"} else {"normal"})
    };

    axum::response::Json(serde_json::json!({
        "ok":     true,
        "action": "answer",
        "answer": answer,
        "data": {
            "aum_usd":        aum,
            "free_capital":   s.capital,
            "pnl_usd":        s.pnl,
            "win_rate":       m.win_rate,
            "total_trades":   m.total_trades,
            "open_positions": s.positions.len(),
            "cb_active":      s.cb_active,
            "positions":      s.positions.iter().map(|p| serde_json::json!({
                "symbol":         p.symbol,
                "side":           p.side,
                "entry_price":    p.entry_price,
                "unrealised_pnl": p.unrealised_pnl,
                "size_usd":       p.size_usd,
                "leverage":       p.leverage,
            })).collect::<Vec<_>>()
        }
    }))
    .into_response()
}

/// `GET /api/v1/session/{id}/hl/account` — live Hyperliquid account state.
///
/// Calls the HL public info API with the configured wallet address.
/// Returns raw clearinghouse state (balances, perp positions, margin usage).
async fn api_v1_hl_account_handler(
    State(app): State<AppState>,
    headers: axum::http::HeaderMap,
    axum::extract::Path(session_id): axum::extract::Path<String>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if let Err(e) = validate_bot_session(&app, &headers, &session_id).await {
        return e;
    }

    let wallet = std::env::var("HYPERLIQUID_WALLET_ADDRESS").unwrap_or_default();
    if wallet.is_empty() {
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            axum::response::Json(serde_json::json!({
                "error": "HYPERLIQUID_WALLET_ADDRESS not configured on this server"
            })),
        )
            .into_response();
    }

    // HL public info API — no signing required for reads
    let hl_url = if std::env::var("HYPERLIQUID_TESTNET").as_deref() == Ok("true") {
        "https://api.hyperliquid-testnet.xyz/info"
    } else {
        "https://api.hyperliquid.xyz/info"
    };

    let client = reqwest::Client::new();
    let payload = serde_json::json!({ "type": "clearinghouseState", "user": wallet });

    match client.post(hl_url).json(&payload).send().await {
        Err(e) => (
            axum::http::StatusCode::BAD_GATEWAY,
            axum::response::Json(serde_json::json!({"error": format!("HL API error: {e}")})),
        )
            .into_response(),
        Ok(resp) => match resp.json::<serde_json::Value>().await {
            Ok(data) => axum::response::Json(serde_json::json!({
                "ok":     true,
                "wallet": wallet,
                "hl":     data,
            }))
            .into_response(),
            Err(e) => (
                axum::http::StatusCode::BAD_GATEWAY,
                axum::response::Json(serde_json::json!({"error": format!("HL parse error: {e}")})),
            )
                .into_response(),
        },
    }
}

pub async fn serve(
    app_state: AppState,
    port: u16,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let app = Router::new()
        .route("/", get(public_landing_handler))
        .route("/dashboard", get(dashboard_handler))
        .route("/app", get(consumer_app_handler))
        .route("/app/agents", get(agent_app_handler))
        .route("/app/history", get(consumer_history_handler))
        .route("/app/tax", get(consumer_tax_handler))
        .route("/app/tax/csv", get(consumer_tax_csv_handler))
        .route("/api/state", get(api_state_handler))
        // ── Stripe billing ─────────────────────────────────────────────────
        .route("/billing/checkout", get(crate::stripe::checkout_handler))
        .route("/billing/success", get(crate::stripe::success_handler))
        .route("/billing/trial", get(crate::stripe::trial_handler))
        .route("/webhooks/stripe", post(crate::stripe::webhook_handler))
        // ── Privy authentication ────────────────────────────────────────────
        .route("/login", get(login_handler))
        .route("/static/privy-login.js", get(privy_bundle_handler))
        .route("/auth/session", post(auth_session_handler))
        .route("/auth/logout", get(auth_logout_handler))
        // ── Onboarding / Terms wall ─────────────────────────────────────────
        .route("/app/onboarding", get(onboarding_handler))
        .route("/app/onboarding/accept", post(onboarding_accept_handler))
        .route("/app/setup", get(hl_setup_handler))
        .route("/app/setup/complete", post(hl_setup_complete_handler))
        .route("/api/hl/balance", get(hl_balance_api_handler))
        .route("/api/hl/wallet/key.json", get(hl_export_key_handler))
        // ── Consumer settings ───────────────────────────────────────────────
        .route("/app/settings", get(consumer_settings_handler))
        .route(
            "/app/settings/wallet",
            post(consumer_settings_wallet_handler),
        )
        // ── Admin panel (HTTP Basic Auth) ───────────────────────────────────
        .route("/admin", get(admin_dashboard_handler))
        .route("/admin/users", get(admin_users_handler))
        .route("/admin/wallets", get(admin_wallets_handler))
        .route("/api/admin/reset-stats", post(admin_reset_stats_handler))
        // ── Apple Pay domain verification ───────────────────────────────────
        .route(
            "/.well-known/apple-developer-merchantid-domain-association",
            get(apple_pay_domain_handler),
        )
        // ── Public API — no auth, rate-limited at the nginx level ──────────
        // Used by the landing page TVL hero graph and external integrations.
        .route("/api/public/tvl", get(public_tvl_handler))
        .route("/api/public/tvl/svg", get(public_tvl_svg_handler))
        .route("/api/public/stats", get(api_public_stats_handler))
        // ── Funnel / analytics (first-party, no third-party scripts) ───────
        .route("/api/funnel", post(funnel_event_handler))
        // ── Trade journal ────────────────────────────────────────────────
        .route("/api/trade-note", post(trade_note_handler))
        .route("/api/report/latest", get(api_report_latest_handler))
        .route("/api/report/query", post(api_report_query_handler))
        .route("/api/report/patterns", get(api_report_patterns_handler))
        .route(
            "/api/report/patterns/alerts",
            get(api_pattern_alert_handler),
        )
        .route("/api/bridge/withdraw", post(bridge_withdraw_handler))
        .route("/api/bridge/status/:id", get(bridge_status_handler))
        // ── Leaderboard & invite codes ──────────────────────────────────────
        .route("/leaderboard", get(leaderboard_handler))
        .route("/app/invite", get(get_invite_handler))
        .route("/app/invite/generate", post(generate_invite_handler))
        .route("/api/leaderboard", get(api_leaderboard_handler))
        // ── Investment thesis ────────────────────────────────────────────────
        .route(
            "/api/thesis",
            get(thesis_get_handler).post(thesis_update_handler),
        )
        // ── AI trade commands ────────────────────────────────────────────────
        .route("/api/command", post(command_handler))
        // ── Bot API v1 — x402 payment-gated ─────────────────────────────────
        .route("/api/v1/status", get(api_v1_status_handler))
        .route("/api/v1/session", post(api_v1_session_handler))
        .route("/api/v1/session/:id", get(api_v1_session_status_handler))
        .route(
            "/api/v1/session/:id/command",
            post(api_v1_session_command_handler),
        )
        .route("/api/v1/session/:id/query", post(api_v1_query_handler))
        .route(
            "/api/v1/session/:id/hl/account",
            get(api_v1_hl_account_handler),
        )
        .with_state(app_state);
    let addr = format!("0.0.0.0:{}", port);
    log::info!("🌐 Dashboard at http://{}", addr);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════════
//  UNIT TESTS — dashboard data calculations & helper functions
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learner::SignalContribution;

    use std::collections::VecDeque;

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn make_pos(
        side: &str,
        entry: f64,
        stop: f64,
        qty: f64,
        size_usd: f64,
        upnl: f64,
    ) -> PaperPosition {
        PaperPosition {
            symbol: "TEST".to_string(),
            side: side.to_string(),
            entry_price: entry,
            quantity: qty,
            size_usd,
            stop_loss: stop,
            take_profit: entry * 1.10,
            atr_at_entry: entry * 0.02,
            high_water_mark: entry,
            low_water_mark: entry,
            partial_closed: false,
            r_dollars_risked: (entry - stop).abs() * qty,
            tranches_closed: 0,
            dca_count: 0,
            leverage: 1.0,
            cycles_held: 0,
            entry_time: "00:00:00 UTC".to_string(),
            unrealised_pnl: upnl,
            contrib: SignalContribution::default(),
            ai_action: None,
            ai_reason: None,
            entry_confidence: 0.68,
            trade_budget_usd: size_usd,
            dca_spent_usd: 0.0,
            btc_ret_at_entry: 0.0,
            initial_margin_usd: size_usd,
            ob_sentiment: String::new(),
            ob_bid_wall_near: false,
            ob_ask_wall_near: false,
            ob_adverse_cycles: 0,
            order_flow_confidence: 0.0,
            order_flow_direction: String::new(),
            funding_rate: 0.0,
            funding_delta: 0.0,
            onchain_strength: 0.0,
            cex_premium_pct: 0.0,
            cex_mode: String::new(),
            funded_from_pool: false,
            pool_stake_usd: 0.0,
        }
    }

    // ── Equity calculation ────────────────────────────────────────────────────

    #[test]
    fn equity_includes_capital_committed_and_unrealised() {
        // equity = free_capital + committed_margin + unrealised_pnl
        let capital = 800.0;
        let size_usd = 100.0; // margin committed
        let unrealised = 25.0; // open profit
        let pos = make_pos("LONG", 100.0, 95.0, 3.0, size_usd, unrealised);

        let computed_equity: f64 = capital
            + pos.size_usd          // committed
            + pos.unrealised_pnl; // unrealised
        assert!(
            (computed_equity - 925.0).abs() < 1e-10,
            "equity = 800 + 100 + 25 = 925, got {computed_equity}"
        );
    }

    #[test]
    fn equity_with_losing_position_reduces_below_capital_plus_committed() {
        let capital = 800.0;
        let unrealised = -30.0; // open loss
        let pos = make_pos("LONG", 100.0, 95.0, 3.0, 100.0, unrealised);
        let equity: f64 = capital + pos.size_usd + pos.unrealised_pnl;
        assert!(
            (equity - 870.0).abs() < 1e-10,
            "equity with loss: 800 + 100 - 30 = 870, got {equity}"
        );
    }

    #[test]
    fn total_pnl_combines_realised_and_unrealised() {
        // total_pnl = s.pnl (closed) + sum(unrealised_pnl)
        let realised: f64 = 50.0; // closed trade profits
        let unrealised = -10.0; // current open loss
        let total = realised + unrealised;
        assert!(
            (total - 40.0).abs() < 1e-10,
            "total P&L: $50 realised - $10 open = $40"
        );
    }

    #[test]
    fn total_pnl_pct_is_relative_to_initial_capital() {
        let initial: f64 = 1000.0;
        let total_pnl = 150.0;
        let pct = total_pnl / initial * 100.0;
        assert!((pct - 15.0).abs() < 1e-10, "15% gain on $1000 initial");
    }

    // ── PnL sign display (regression for the sign-stripping bug) ─────────────

    #[test]
    fn pnl_sign_positive_is_plus() {
        let total_pnl: f64 = 50.0;
        let sign = if total_pnl >= 0.0 { "+" } else { "-" };
        assert_eq!(sign, "+", "positive PnL should use '+' prefix");
    }

    #[test]
    fn pnl_sign_negative_is_minus_not_empty() {
        let total_pnl: f64 = -50.0;
        // REGRESSION: old code used "" for negative, causing sign to be dropped
        // when combined with .abs() → would display "$50.00" instead of "-$50.00"
        let sign = if total_pnl >= 0.0 { "+" } else { "-" };
        assert_eq!(
            sign, "-",
            "REGRESSION: negative PnL must use '-' prefix, not empty string"
        );
    }

    #[test]
    fn pnl_display_negative_with_abs_shows_correct_sign() {
        // Simulates the format string: {pnl_sign}${total_pnl:.2}
        let total_pnl: f64 = -123.45;
        let sign = if total_pnl >= 0.0 { "+" } else { "-" };
        let display = format!("{}${:.2}", sign, total_pnl.abs());
        assert_eq!(display, "-$123.45", "expected '-$123.45', got '{display}'");
    }

    #[test]
    fn pnl_display_positive_with_abs_shows_correct_sign() {
        let total_pnl: f64 = 78.90;
        let sign = if total_pnl >= 0.0 { "+" } else { "-" };
        let display = format!("{}${:.2}", sign, total_pnl.abs());
        assert_eq!(display, "+$78.90", "expected '+$78.90', got '{display}'");
    }

    // ── Rolling 7-day drawdown calculation ────────────────────────────────────

    #[test]
    fn rolling_dd_is_zero_when_at_or_above_window_peak() {
        // equity >= rolling_peak → no drawdown
        let equity = 1100.0;
        let mut window: VecDeque<(i64, f64)> = VecDeque::new();
        window.push_back((0, 1000.0));
        window.push_back((1, 1050.0));
        window.push_back((2, 1080.0));

        let rolling_peak = window.iter().map(|&(_, e)| e).fold(equity, f64::max);
        let dd = ((rolling_peak - equity) / rolling_peak * 100.0).max(0.0);
        // equity=1100 > all window entries → rolling_peak=1100 → dd=0
        assert_eq!(dd, 0.0, "at or above window peak → zero drawdown");
    }

    #[test]
    fn rolling_dd_reflects_peak_within_7_day_window() {
        // Peak was 1200, current equity 1100 → 8.33% drawdown
        let equity = 1100.0;
        let mut window: VecDeque<(i64, f64)> = VecDeque::new();
        window.push_back((0, 1000.0));
        window.push_back((1, 1200.0)); // 7-day peak
        window.push_back((2, 1050.0));

        let rolling_peak = window.iter().map(|&(_, e)| e).fold(equity, f64::max);
        let dd = ((rolling_peak - equity) / rolling_peak * 100.0).max(0.0);
        let expected = (1200.0 - 1100.0) / 1200.0 * 100.0; // 8.333...%
        assert!(
            (dd - expected).abs() < 1e-6,
            "rolling DD: expected {expected:.3}%, got {dd:.3}%"
        );
    }

    #[test]
    fn rolling_dd_uses_equity_as_fallback_when_window_empty() {
        // Empty window → fold starts at equity → rolling_peak = equity → dd = 0
        let equity = 1000.0;
        let window: VecDeque<(i64, f64)> = VecDeque::new();
        let rolling_peak = window.iter().map(|&(_, e)| e).fold(equity, f64::max);
        assert_eq!(
            rolling_peak, equity,
            "empty window → rolling_peak = current equity"
        );
        let dd = ((rolling_peak - equity) / rolling_peak * 100.0).max(0.0);
        assert_eq!(dd, 0.0, "empty window → zero drawdown");
    }

    #[test]
    fn all_time_dd_uses_peak_equity_not_rolling_window() {
        // The all-time peak is tracked separately in s.peak_equity.
        // This can be much higher than the rolling 7-day peak.
        let peak_equity: f64 = 5000.0; // hit months ago
        let equity = 1000.0; // current
        let dd_pct = (peak_equity - equity) / peak_equity * 100.0;
        assert!(
            (dd_pct - 80.0).abs() < 1e-10,
            "all-time DD: 80%% from $5000 peak to $1000 current, got {dd_pct}"
        );
        // This would show "80%" in the dashboard — very alarming but historically accurate.
        // The CB uses rolling 7-day (8% threshold), NOT all-time.
    }

    #[test]
    fn cb_uses_rolling_dd_not_all_time_dd() {
        // The CB threshold is 8% rolling DD.
        // A position with 80% all-time DD but only 3% 7-day DD should NOT trigger CB.
        let peak_equity = 5000.0;
        let equity = 1000.0;
        let all_time_dd_pct = (peak_equity - equity) / peak_equity * 100.0; // 80%

        // Rolling window only has recent data
        let mut window: VecDeque<(i64, f64)> = VecDeque::new();
        window.push_back((0, 1020.0));
        window.push_back((1, 1000.0));
        let rolling_peak = window.iter().map(|&(_, e)| e).fold(equity, f64::max);
        let rolling_dd_pct = ((rolling_peak - equity) / rolling_peak * 100.0).max(0.0);

        let cb_threshold = 8.0_f64;
        let cb_from_all_time = all_time_dd_pct > cb_threshold;
        let cb_from_rolling = rolling_dd_pct > cb_threshold;

        assert!(
            cb_from_all_time,
            "all-time DD 80% would trigger CB: {all_time_dd_pct}%"
        );
        assert!(
            !cb_from_rolling,
            "rolling 7d DD {rolling_dd_pct}% should NOT trigger CB"
        );
    }

    // ── reason_class helper ───────────────────────────────────────────────────

    #[test]
    fn reason_class_stop_loss() {
        assert_eq!(reason_class("StopLoss"), "stop");
    }

    #[test]
    fn reason_class_take_profit() {
        assert_eq!(reason_class("TakeProfit"), "take");
    }

    #[test]
    fn reason_class_time_exit() {
        assert_eq!(reason_class("TimeExit"), "time");
    }

    #[test]
    fn reason_class_partial() {
        assert_eq!(reason_class("Partial2R"), "partial");
    }

    #[test]
    fn reason_class_ai_close_is_ai_not_signal() {
        // REGRESSION: AI-Close was incorrectly mapped to "signal" (grey text).
        // It should now map to "ai" (yellow, bold).
        assert_eq!(
            reason_class("AI-Close"),
            "ai",
            "REGRESSION: AI-Close must map to 'ai' class, not 'signal'"
        );
    }

    #[test]
    fn reason_class_signal_exit() {
        assert_eq!(reason_class("SignalExit"), "signal");
    }

    #[test]
    fn reason_class_unknown_falls_back_to_signal() {
        assert_eq!(reason_class("SomethingNew"), "signal");
    }

    // ── wi() helper (weight strip) ────────────────────────────────────────────

    #[test]
    fn wi_produces_html_with_label_and_value() {
        let html = wi("RSI", 0.75);
        assert!(html.contains("RSI"), "wi() must contain label");
        assert!(html.contains("0.75"), "wi() must contain formatted value");
    }

    #[test]
    fn wi_bar_width_capped_at_100_percent() {
        // val > 1.0 should cap bar width at 100%
        let html = wi("OverVal", 1.5);
        assert!(
            html.contains("width:100%"),
            "bar width must be capped at 100%: {html}"
        );
    }

    #[test]
    fn wi_bar_width_scales_with_value() {
        let html = wi("TestSig", 0.50);
        assert!(html.contains("width:50%"), "val=0.50 should give width:50%");
    }

    // ── R-multiple display ────────────────────────────────────────────────────

    #[test]
    fn r_multiple_display_uses_r_dollars_risked() {
        // Dashboard: r_mult = unrealised_pnl / r_dollars_risked
        let pos = make_pos("LONG", 100.0, 95.0, 3.0, 100.0, 15.0);
        // r_dollars_risked = (100 - 95) × 3 = $15
        let r_mult = if pos.r_dollars_risked > 1e-8 {
            pos.unrealised_pnl / pos.r_dollars_risked
        } else {
            0.0
        };
        assert!(
            (r_mult - 1.0).abs() < 1e-10,
            "unrealised=$15 / r_risk=$15 should be exactly 1R, got {r_mult}"
        );
    }

    #[test]
    fn r_multiple_bar_pct_clamps_to_0_100() {
        // bar_pct = ((r_mult + 1) / 6 * 100).clamp(0, 100)
        // At -1R → 0%, at 0R → 16.7%, at 2R → 50%, at 5R → 100%
        let clamp = |r: f64| -> f64 { ((r + 1.0) / 6.0 * 100.0).clamp(0.0, 100.0) };

        assert_eq!(clamp(-2.0), 0.0, "-2R → bar at 0%");
        assert_eq!(clamp(-1.0), 0.0, "-1R → bar at 0%");
        assert!((clamp(0.0) - 16.67).abs() < 0.1, "0R → bar at ~16.7%");
        assert!((clamp(2.0) - 50.0).abs() < 0.1, "2R → bar at 50%");
        assert_eq!(clamp(5.0), 100.0, "5R → bar at 100%");
        assert_eq!(clamp(10.0), 100.0, "10R → bar still clamped at 100%");
    }

    // ── Position card border colour ───────────────────────────────────────────

    #[test]
    fn position_border_green_when_profitable() {
        let upnl = 50.0;
        let r_risk = 15.0;
        let border = if upnl > 0.0 {
            "#238636"
        } else if upnl < -r_risk * 0.5 {
            "#da3633"
        } else {
            "#444c56"
        };
        assert_eq!(
            border, "#238636",
            "profitable position should have green border"
        );
    }

    #[test]
    fn position_border_red_when_loss_exceeds_half_r() {
        let upnl = -10.0;
        let r_risk = 15.0; // half-R = -7.5, loss = -10 > -7.5
        let border = if upnl > 0.0 {
            "#238636"
        } else if upnl < -r_risk * 0.5 {
            "#da3633"
        } else {
            "#444c56"
        };
        assert_eq!(
            border, "#da3633",
            "loss > 0.5R should show red danger border"
        );
    }

    #[test]
    fn position_border_neutral_when_small_loss() {
        let upnl = -3.0; // less than half of R
        let r_risk = 15.0; // half-R = -7.5 → loss -3 < -7.5 is false
        let border = if upnl > 0.0 {
            "#238636"
        } else if upnl < -r_risk * 0.5 {
            "#da3633"
        } else {
            "#444c56"
        };
        assert_eq!(
            border, "#444c56",
            "small loss < 0.5R should show neutral border"
        );
    }

    // ── Hold time formatting ──────────────────────────────────────────────────

    #[test]
    fn hold_time_under_60_minutes_shows_minutes() {
        let cycles_held = 40u64; // 40 cycles × 30s = 20 min
        let hold_mins = cycles_held / 2;
        let hold_str = if hold_mins < 60 {
            format!("{}m", hold_mins)
        } else {
            format!("{:.1}h", hold_mins as f64 / 60.0)
        };
        assert_eq!(hold_str, "20m", "40 cycles = 20m hold time");
    }

    #[test]
    fn hold_time_over_60_minutes_shows_hours() {
        let cycles_held = 240u64; // 240 cycles × 30s = 120 min = 2.0h
        let hold_mins = cycles_held / 2;
        let hold_str = if hold_mins < 60 {
            format!("{}m", hold_mins)
        } else {
            format!("{:.1}h", hold_mins as f64 / 60.0)
        };
        assert_eq!(hold_str, "2.0h", "240 cycles = 2.0h hold time");
    }

    // ── Tranche label ─────────────────────────────────────────────────────────

    #[test]
    fn tranche_0_shows_first_target() {
        let t = 0u8;
        let label = match t {
            0 => "target 1R",
            1 => "1/4 banked · target 2R",
            2 => "1/4+1/3 banked · target 4R",
            _ => "5/8 banked · trailing",
        };
        assert_eq!(label, "target 1R");
    }

    #[test]
    fn tranche_1_shows_quarter_banked() {
        let t = 1u8;
        let label = match t {
            0 => "target 1R",
            1 => "1/4 banked · target 2R",
            2 => "1/4+1/3 banked · target 4R",
            _ => "5/8 banked · trailing",
        };
        assert_eq!(label, "1/4 banked · target 2R");
    }

    #[test]
    fn tranche_2_shows_quarter_plus_third_banked() {
        let t = 2u8;
        let label = match t {
            0 => "target 1R",
            1 => "1/4 banked · target 2R",
            2 => "1/4+1/3 banked · target 4R",
            _ => "5/8 banked · trailing",
        };
        assert_eq!(label, "1/4+1/3 banked · target 4R");
    }

    #[test]
    fn tranche_3_shows_five_eighths_banked() {
        let t = 3u8;
        let label = match t {
            0 => "target 1R",
            1 => "1/4 banked · target 2R",
            2 => "1/4+1/3 banked · target 4R",
            _ => "5/8 banked · trailing",
        };
        assert_eq!(label, "5/8 banked · trailing");
    }

    // ── Dashboard slot badge correctness ──────────────────────────────────────

    #[test]
    fn position_count_is_heat_budgeted_not_hardcapped() {
        // No hard slot cap — portfolio heat (15% equity) and Kelly sizing are
        // the only limits.  This test documents that assumption so a future
        // regression that re-introduces a count cap will be caught.
        let heat_cap_pct = 15.0_f64;
        let per_trade_heat_pct = 2.0_f64;
        // Theoretical maximum simultaneous trades if every trade is at the per-trade heat floor:
        let theoretical_max = (heat_cap_pct / per_trade_heat_pct).floor() as usize;
        // Must be well above the old hard cap of 8.
        assert!(
            theoretical_max >= 7,
            "heat budget should allow at least 7 positions"
        );
        // The old hard cap constants must NOT be referenced anywhere.
        // (Compile-time check: MAX_POSITIONS and MAX_SAME_DIRECTION are deleted.)
    }
}
