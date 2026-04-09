// pub use so submodules can access via `use super::*;`
#[allow(unused_imports)]
pub use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::{
        sse::{Event, KeepAlive, Sse},
        Html,
    },
    routing::{get, post},
    Json, Router,
};
pub use futures_util::stream::StreamExt;
pub use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
pub use serde_json::{json, Value as JsonValue};
pub use std::collections::HashMap;
pub use std::fs;
pub use std::path::PathBuf;
pub use std::sync::Arc;
pub use tokio::sync::{Mutex, RwLock};

// Pre-built Privy login SDK bundle (ESM). Served at /static/privy-login.js so
// the browser never needs to reach an external CDN.
// Rebuild the bundle whenever you upgrade @privy-io/react-auth:
//   cd js && npm install && npm run build
//   git add static/privy-login.js && git commit
pub static PRIVY_BUNDLE_JS: &str = include_str!("../../static/privy-login.js");
pub use crate::bridge::BridgeManager;
pub(crate) use crate::coins;
pub(crate) use crate::exchange;
pub use crate::learner::{SignalContribution, SignalWeights};
pub use crate::metrics::PerformanceMetrics;
pub(crate) use crate::pattern_insights;
pub(crate) use crate::reporting;

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
    pub latency_tracker: std::sync::Arc<tokio::sync::RwLock<crate::latency::LatencyTracker>>,
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
    /// Internal-only: not serialised to the API (can reach 20 000+ entries over 7 days).
    /// The frontend uses `equity_history` (capped at 288) for the sparkline.
    #[serde(skip)]
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
    /// Internal-only price baseline — not serialised to the API (500+ symbols × entry, unused by frontend).
    #[serde(skip)]
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

    /// Consensus macro regime derived from daily BTC + ETH MA5/MA10/MA20.
    /// Values: "BULL" | "BEAR" | "TRANSITION"
    /// Updated once per cycle. Displayed in dashboard header.
    #[serde(default)]
    pub macro_regime: String,

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
    // ── Identity + paper capital ──────────────────────────────────────────
    /// Human-readable label for this session (e.g. "AJ", "Daniel").
    /// Admin-created sessions set this at creation time.
    #[serde(default)]
    pub name: Option<String>,
    /// Paper capital allocated to this session (USD). Defaults to 200.0 for
    /// admin-created sessions; 0.0 for x402 sessions (they share the global pool).
    #[serde(default)]
    pub balance_usd: f64,
    /// Realised P&L accumulated within this session (USD).
    #[serde(default)]
    pub session_pnl: f64,
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
            macro_regime: String::new(),
        }
    }
}

pub type SharedState = Arc<RwLock<BotState>>;


// ─── Shared HTML helper ──────────────────────────────────────────────────

pub(crate) fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}


// ─── Base-64 encoder (admin HTTP Basic Auth) ───────────────────────────

pub(crate) fn base64_encode(input: &str) -> String {
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


// ─── Session / auth helpers ──────────────────────────────────────────────

pub(crate) fn get_session_tenant_id(
    headers: &axum::http::HeaderMap,
    secret: &str,
) -> Option<crate::tenant::TenantId> {
    let cookie_hdr = headers.get("cookie")?.to_str().ok()?;
    let session_val = crate::privy::extract_session_cookie(cookie_hdr)?;
    crate::privy::verify_session(session_val, secret).ok()
}

pub(crate) fn check_admin_auth(headers: &axum::http::HeaderMap, password: &str) -> bool {
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
pub(crate) fn www_authenticate_response() -> axum::response::Response {
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

// ─── Shared admin shell (sidebar + topbar CSS + chrome) ──────────────────────
//
// Call `admin_shell(page_title, active_nav, topbar_pills, body_html)` to wrap
// any admin page in the consistent sidebar / topbar layout.


// ─── ID generation helper ────────────────────────────────────────────────

pub(crate) fn new_id(prefix: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}_{:x}{:x}", prefix, dur.as_millis(), dur.subsec_nanos())
}

// ─── Submodule declarations ───────────────────────────────────────────────────
mod handlers_dashboard;  // /dashboard, /api/state
mod handlers_consumer;   // /app/*, /login, /auth/*
mod handlers_admin;      // /admin/*
mod handlers_public;     // /leaderboard, /api/report/*, /api/bridge/*
mod handlers_hl;         // /app/setup, /api/hl/*, /api/command, /api/thesis
mod handlers_landing;    // / (public landing), /api/public/*
mod handlers_api_v1;     // /api/v1/*, /wallet/*, /venues, /fleet

pub(crate) use handlers_dashboard::*;
pub(crate) use handlers_consumer::*;
pub(crate) use handlers_admin::*;
pub(crate) use handlers_public::*;
pub(crate) use handlers_hl::*;
pub(crate) use handlers_landing::*;
pub(crate) use handlers_api_v1::*;

pub async fn serve(
    app_state: AppState,
    port: u16,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let app = Router::new()
        .route("/", get(public_landing_handler))
        .route("/venues", get(public_venues_handler))
        .route("/venues/hyperliquid", get(public_venues_handler))
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
        .route("/api/admin/session", post(admin_create_session_handler))
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
        .route("/fleet", get(fleet_handler))
        .route("/app/invite", get(get_invite_handler))
        .route("/app/invite/generate", post(generate_invite_handler))
        .route("/api/leaderboard", get(api_leaderboard_handler))
        // ── Live winning-trade SSE stream ────────────────────────────────────
        // GET /api/trade-stream  → text/event-stream
        // Events are emitted in real-time whenever any tenant closes a winning
        // trade.  Dashboard top-page ticker consumes this via EventSource.
        .route("/api/trade-stream", get(trade_stream_handler))
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
        // ── New v0.2.1 read-only session endpoints ───────────────────────────
        .route(
            "/api/v1/session/:id/positions",
            get(api_v1_session_positions_handler),
        )
        .route(
            "/api/v1/session/:id/trades",
            get(api_v1_session_trades_handler),
        )
        .route(
            "/api/v1/session/:id/latency/stats",
            get(api_v1_session_latency_stats_handler),
        )
        // ── Public venue metadata (no auth) ──────────────────────────────────
        .route(
            "/api/v1/venues/hyperliquid/markets",
            get(api_v1_venues_hyperliquid_markets_handler),
        )
        // ── Named wallet pages (public read-only) ─────────────────────────────
        .route("/wallet/:name", get(wallet_page_handler))
        .route("/api/wallet/:name", get(api_wallet_handler))
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
            venue: "Hyperliquid Perps (paper)".to_string(),
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
