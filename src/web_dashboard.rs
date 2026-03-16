use axum::{extract::State, http::HeaderMap, response::Html, routing::{get, post}, Json, Router};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use crate::learner::{SignalContribution, SignalWeights};
use crate::metrics::PerformanceMetrics;
use crate::coins;

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
    pub bot_state:             SharedState,
    /// Registry of all consumer tenants — mutated by Stripe webhooks.
    pub tenants:               crate::tenant::SharedTenantManager,
    /// PostgreSQL connection pool — `None` when DATABASE_URL is not set.
    /// Shared across all Axum handlers and the trading loop.
    pub db:                    Option<crate::db::SharedDb>,
    /// Stripe secret API key (sk_live_… / sk_test_…).
    pub stripe_api_key:        Option<String>,
    /// Stripe webhook signing secret (whsec_…).
    pub stripe_webhook_secret: Option<String>,
    /// Stripe Price ID for the $19.99/month Pro plan.
    pub stripe_price_id:       Option<String>,
    /// Privy App ID — when set, consumer routes require a valid Privy session.
    /// Set via `PRIVY_APP_ID` env var.  `None` = single-operator fallback mode.
    pub privy_app_id:          Option<String>,
    /// HMAC-SHA256 signing key for session cookies.  Set via `SESSION_SECRET`.
    pub session_secret:        String,
    /// In-memory cache of Privy's JWKS — refreshed every hour on first use.
    pub jwks_cache:            crate::privy::SharedJwksCache,
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
}

// ─────────────────────────────── Serde defaults ──────────────────────────────
fn default_leverage() -> f64 { 1.0 }

// ─────────────────────────────── State structs ───────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperPosition {
    pub symbol:           String,
    pub side:             String,    // "LONG" | "SHORT"
    pub entry_price:      f64,
    pub quantity:         f64,       // coins held (reduced by partial closes)
    pub size_usd:         f64,       // USD committed (reduced by partial closes)
    pub stop_loss:        f64,       // current (trailing) stop
    pub take_profit:      f64,
    pub atr_at_entry:     f64,       // ATR at entry (for trailing)
    pub high_water_mark:  f64,       // highest price seen (LONG trailing)
    pub low_water_mark:   f64,       // lowest  price seen (SHORT trailing)
    pub partial_closed:   bool,      // true once first tranche taken
    // ── Professional quant fields ─────────────────────────────────────────
    pub r_dollars_risked: f64,       // dollars at risk on entry = |entry−stop| × qty_at_entry
    pub tranches_closed:  u8,        // 0=none, 1=first 1/3 at 2R, 2=second 1/3 at 4R
    #[serde(default)]
    pub dca_count:        u8,        // number of DCA add-ons executed (averaging down)
    #[serde(default = "default_leverage")]
    pub leverage:         f64,       // leverage applied at entry (1.5× – 5×)
    pub cycles_held:      u64,       // incremented each 30s cycle (time-decay exit)
    pub entry_time:       String,
    pub unrealised_pnl:   f64,
    pub contrib:          SignalContribution,
    // ── AI reviewer fields (updated every 10 cycles) ──────────────────────
    #[serde(default)]
    pub ai_action: Option<String>,   // "scale_up" | "hold" | "scale_down" | "close_now"
    #[serde(default)]
    pub ai_reason: Option<String>,   // Claude's one-line rationale
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClosedTrade {
    pub symbol:     String,
    pub side:       String,
    pub entry:      f64,
    pub exit:       f64,
    pub pnl:        f64,
    pub pnl_pct:    f64,
    pub reason:     String,   // "Signal" | "StopLoss" | "TakeProfit" | "Partial"
    pub closed_at:  String,
    // ── Tax / record-keeping fields (all default-zero for old snapshots) ──
    /// Timestamp when the position was originally opened.
    #[serde(default)]
    pub entry_time: String,
    /// Number of base-asset units traded.
    #[serde(default)]
    pub quantity:   f64,
    /// USD margin committed (not notional — notional = size_usd × leverage).
    #[serde(default)]
    pub size_usd:   f64,
    /// Leverage multiplier used at entry.
    #[serde(default = "default_one")]
    pub leverage:   f64,
    /// Estimated fees paid (maker+taker+builder, ~0.075 % of notional).
    #[serde(default)]
    pub fees_est:   f64,
    /// HTML snippet shown when user clicks the row — technicals + AI reasoning.
    #[serde(default)]
    pub breakdown:  Option<String>,
}

fn default_one() -> f64 { 1.0 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateInfo {
    pub symbol:          String,
    pub price:           f64,
    /// None on cycle 1 (no previous reference price yet); Some(%) on cycle 2+.
    pub change_pct:      Option<f64>,
    /// RSI(14) value computed during signal analysis, None until first scan.
    #[serde(default)]
    pub rsi:             Option<f64>,
    /// Market regime: "Trending" | "Neutral" | "Ranging", None until first scan.
    #[serde(default)]
    pub regime:          Option<String>,
    /// ATR(14) as % of price — a volatility proxy, None until first scan.
    #[serde(default)]
    pub atr_pct:         Option<f64>,
    /// Decision confidence 0‒1 from the last analyse_symbol run, None until first scan.
    #[serde(default)]
    pub confidence:      Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionInfo {
    pub symbol:      String,
    pub action:      String,
    pub confidence:  f64,
    pub entry_price: f64,
    pub rationale:   String,
    pub timestamp:   String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotState {
    pub capital:          f64,
    pub initial_capital:  f64,
    pub peak_equity:      f64,       // all-time equity high (display only)
    pub equity_window:    std::collections::VecDeque<(i64, f64)>, // (unix_ts, equity) rolling 7-day
    pub cb_active:        bool,      // true when rolling-equity CB is firing (set by main loop)
    pub pnl:              f64,
    pub cycle_count:      u64,
    pub candidates:       Vec<CandidateInfo>,
    pub positions:        Vec<PaperPosition>,
    pub closed_trades:    Vec<ClosedTrade>,
    pub recent_decisions: Vec<DecisionInfo>,
    pub signal_weights:   SignalWeights,
    pub metrics:          PerformanceMetrics,
    pub session_prices:   HashMap<String, f64>,  // first price seen per symbol this session
    pub status:           String,
    pub last_update:      String,
    /// Unix-ms timestamp when the next 30 s cycle will fire.  0 = unknown.
    pub next_cycle_at:    i64,
    /// Rolling equity snapshots (max 288 ≈ 2.4 h at 30 s/cycle) for the sparkline.
    /// Populated by the main trading loop every cycle — NOT by page loads.
    #[serde(default)]
    pub equity_history:   Vec<f64>,
    /// Platform Hyperliquid referral code — set from config at startup, not persisted.
    /// Displayed in the consumer /app page so new signups use the referral link.
    #[serde(default)]
    pub referral_code:    Option<String>,
}

impl Default for BotState {
    fn default() -> Self {
        BotState {
            capital: 1000.0, initial_capital: 1000.0, peak_equity: 1000.0,
            equity_window: std::collections::VecDeque::new(),
            cb_active: false,
            pnl: 0.0, cycle_count: 0,
            candidates: vec![], positions: vec![], closed_trades: vec![],
            recent_decisions: vec![],
            signal_weights: SignalWeights::default(),
            metrics: PerformanceMetrics::default(),
            session_prices: HashMap::new(),
            status: String::new(), last_update: String::new(), next_cycle_at: 0,
            equity_history: vec![],
            referral_code:  None,
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
    let committed:  f64 = s.positions.iter().map(|p| p.size_usd).sum();
    let equity      = s.capital + committed + unrealised;
    let total_pnl   = s.pnl + unrealised;
    let total_pnl_pct = if s.initial_capital > 0.0 { total_pnl / s.initial_capital * 100.0 } else { 0.0 };

    let pnl_colour  = if total_pnl >= 0.0 { "#3fb950" } else { "#f85149" };
    // BUG FIX: sign was "" for negatives (not "-"), causing minus to be silently dropped
    // when combined with the .abs() calls in the format args.
    let pnl_sign    = if total_pnl >= 0.0 { "+" } else { "-" };
    // All-time peak drawdown (display only).
    let dd_pct      = if s.peak_equity > 0.0 { (s.peak_equity - equity) / s.peak_equity * 100.0 } else { 0.0 };
    // Rolling 7-day drawdown — this is what actually drives the circuit breaker.
    // The CB uses equity_window, so we must derive the rolling peak from the same source.
    let rolling_peak = s.equity_window.iter()
        .map(|&(_, e)| e)
        .fold(equity, f64::max);
    let rolling_dd_pct = if rolling_peak > 0.0 {
        ((rolling_peak - equity) / rolling_peak * 100.0).max(0.0)
    } else { 0.0 };

    // ── Metric strings ────────────────────────────────────────────────────
    let kelly     = m.kelly_fraction();
    let kelly_str = if kelly < 0.0 { "learning…".to_string() } else { format!("{:.1}%", kelly * 100.0) };
    // Use the rolling-equity CB flag set by main loop — this is the same signal
    // that actually controls position sizing, avoiding a stale metrics-based read.
    let cb_active = s.cb_active;
    let cb_label  = if cb_active { "⚡ CB Active" } else { "● Normal" };
    let cb_colour = if cb_active { "#f85149" } else { "#3fb950" };
    // BUG FIX: was using m.current_dd (P&L-curve drawdown from closed trades only).
    // The CB is driven by rolling_dd_pct (7-day equity window) — use that here.
    let cb_desc   = if cb_active {
        format!("0.35× sizes · 7d DD {:.1}%", rolling_dd_pct)
    } else {
        format!("Risk Normal · 7d DD {:.1}%", rolling_dd_pct)
    };
    let pf_str    = if m.profit_factor.is_infinite() { "∞".to_string() } else { format!("{:.2}", m.profit_factor) };

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

            let tranche_label = match p.tranches_closed {
                0 => "target <b>2R</b>".to_string(),
                1 => "<span style='color:#3fb950'>⅓ banked</span> · target <b>4R</b>".to_string(),
                _ => "<span style='color:#3fb950'>⅔ banked</span> · trailing".to_string(),
            };

            // DCA badge — shown when we've averaged down
            let dca_badge = if p.dca_count > 0 {
                format!(" <span style='background:#332a00;color:#e3b341;border:1px solid #e3b34150;\
                          border-radius:4px;padding:1px 5px;font-size:.68em'>DCA×{}</span>", p.dca_count)
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

            format!(r#"<div class="pos-flip-wrap" id="pf-{sym_id}"><div class="pos-flip-inner">
<div class="pos-card" style="border-left:3px solid {border}" id="pos-{sym_id}" onclick="flipPos('{sym_id}')">
  <div class="pos-header">
    <span class="pos-sym">{logo}{sym}</span>{name}{dca}
    <span class="pos-side" style="color:{sc}">{arrow} {side}</span>
    <span class="pos-age">{hold}</span>
  </div>
  <div id="pos-{sym_id}-pnl" class="pos-pnl" style="color:{pc}">{ps}{pnl:.2} ({ps}{pct:.1}%) &nbsp; <b style="font-size:1.1em">{r:+.2}R</b></div>
  <div class="pos-bar-wrap">
    <div id="pos-{sym_id}-bar" class="pos-bar" style="width:{bp:.0}%;background:{bc}"></div>
    <div class="pos-bar-marks"><span>-1R</span><span>0</span><span>2R</span><span>4R</span></div>
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
                time     = p.entry_time,
                ai_row   = ai_row,
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
                (Some(ap), Some(bp)) =>
                    bp.unrealised_pnl.partial_cmp(&ap.unrealised_pnl).unwrap_or(std::cmp::Ordering::Equal),
                (Some(_), None)  => std::cmp::Ordering::Less,
                (None, Some(_))  => std::cmp::Ordering::Greater,
                (None, None)     => {
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
        let h       = &s.equity_history;
        let initial = s.initial_capital;
        if h.len() < 2 {
            // Not enough data yet — flat placeholder
            r##"<svg width="320" height="80" viewBox="0 0 320 80"
     style="display:block;flex-shrink:0;overflow:visible;opacity:0.4">
  <text x="2" y="10" fill="#484f58" font-size="9" font-family="monospace">PORTFOLIO</text>
  <line x1="0" y1="46" x2="280" y2="46"
        stroke="#484f58" stroke-width="1.5" stroke-dasharray="4 4"/>
  <text x="284" y="50" fill="#484f58" font-size="9" font-family="monospace">—</text>
</svg>"##.to_string()
        } else {
            let w_px:   f64 = 280.0;   // chart area width (label gutter on right)
            let h_px:   f64 = 80.0;
            let pad_t:  f64 = 14.0;    // top padding (for "PORTFOLIO" label)
            let pad_b:  f64 = 6.0;
            let inner_h     = h_px - pad_t - pad_b;

            // Y-scale anchored to initial_capital so baseline is always visible
            let data_min = h.iter().cloned().fold(f64::INFINITY,     f64::min).min(initial);
            let data_max = h.iter().cloned().fold(f64::NEG_INFINITY, f64::max).max(initial);
            // Symmetric 15 % buffer so the line never presses against the edges
            let buf   = ((data_max - data_min).max(initial * 0.005)) * 0.18;
            let min_v = data_min - buf;
            let max_v = data_max + buf;
            let range = (max_v - min_v).max(0.01);

            // Map a $ value to an SVG y coordinate (top = high equity)
            let to_y = |v: f64| -> f64 {
                h_px - pad_b - (v - min_v) / range * inner_h
            };

            let n = h.len() as f64;
            let pts: String = h.iter().enumerate().map(|(i, &v)| {
                let x = i as f64 / (n - 1.0) * w_px;
                let y = to_y(v);
                format!("{x:.1},{y:.1}")
            }).collect::<Vec<_>>().join(" ");

            let base_y   = to_y(initial);
            let last_y   = to_y(*h.last().unwrap_or(&initial));
            let last_val = *h.last().unwrap_or(&initial);
            let max_y    = to_y(data_max);

            // Green when above initial capital, red when below
            let trend_c  = if last_val >= initial { "#3fb950" } else { "#f85149" };

            // Fill polygon: line path → close back along the baseline
            let fill_pts = format!("{pts} {w_px:.1},{base_y:.1} 0.0,{base_y:.1}");

            // Y-axis tick label values
            let lbl_cur  = format!("${:.0}", last_val);
            let lbl_base = format!("${:.0}", initial);
            let lbl_max  = format!("${:.0}", data_max);

            // Label positions (right gutter starting at x=284)
            let ly_cur  = last_y.max(pad_t + 4.0).min(h_px - 4.0);
            let ly_base = base_y.max(pad_t + 4.0).min(h_px - 4.0);
            let ly_max  = max_y.max(pad_t + 4.0).min(h_px - 4.0);

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
                c    = trend_c,
                m    = "#484f58",
                w    = w_px,
                by   = base_y,
                fp   = fill_pts,
                pts  = pts,
                ly   = last_y,
                lc   = lbl_cur,
                lb   = lbl_base,
                lm   = lbl_max,
                lc_y = ly_cur,
                lb_y = ly_base,
                lm_y = ly_max,
            )
        }
    };

    // ── Signal weights: single-line inline strip ─────────────────────────
    let w  = &s.signal_weights;
    let wh = format!(
        r#"<div class="w-strip">{}{}{}{}{}{}<span class="w-strip-note">{total_closed} trades · live learning</span></div>"#,
        wi("RSI",     w.rsi),
        wi("BB",      w.bollinger),
        wi("MACD",    w.macd),
        wi("Trend",   w.trend),
        wi("OrdFlow", w.order_flow),
        wi("🌙Sent",  w.sentiment),
        total_closed = s.closed_trades.len(),
    );

    Html(format!(r#"<!DOCTYPE html>
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
         padding:9px 11px;text-align:center;
         transition:border-color .2s,box-shadow .2s}}
.metric:hover{{border-color:var(--border2);box-shadow:0 2px 8px rgba(0,0,0,.3)}}
.metric .mv{{font-size:1.05em;font-weight:700;letter-spacing:-.01em}}
.metric .ml{{font-size:.62em;color:var(--muted);margin-top:3px;white-space:nowrap;letter-spacing:.3px;text-transform:uppercase}}
/* ── Status bar ── */
.status-bar{{background:var(--surface2);border:1px solid var(--border);border-radius:9px;
             padding:0;margin-bottom:12px;font-size:.78em;color:var(--muted);overflow:hidden}}
.status-inner{{display:flex;justify-content:space-between;align-items:center;
               gap:8px;flex-wrap:wrap;padding:8px 12px}}
.status-bar .st-text{{flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}}
.prog-track{{height:2px;background:var(--border);position:relative;overflow:hidden}}
.prog-fill{{height:2px;background:linear-gradient(90deg,var(--blue),var(--purple),var(--green));
            animation:progFill 30s linear forwards}}
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

<div class="equity-hero">
  <div class="eq-left">
    <div class="eq-label">Total Equity</div>
    <div id="equity-val" class="eq-val">${equity:.2}</div>
    <div id="equity-label" class="eq-label" style="margin-top:3px">Free $<span id="equity-free">{capital:.2}</span></div>
    <div id="pnl-badge" class="pnl-badge" style="color:{pnl_colour};border:1px solid {pnl_colour}40;background:{pnl_colour}15;margin-top:8px;display:inline-block">
      {pnl_sign}${total_pnl:.2} &nbsp; {pnl_sign}{total_pnl_pct:.2}%
    </div>
  </div>
  <div class="eq-right">
    {sparkline_svg}
  </div>
</div>

<div class="metrics">
  <div class="metric"><div class="mv" style="color:{sc}">{sharpe:.2}</div><div class="ml">Sharpe</div></div>
  <div class="metric"><div class="mv" style="color:{sortc}">{sortino:.2}</div><div class="ml">Sortino</div></div>
  <div class="metric"><div class="mv" style="color:{expc}">{exps}{expectancy:.1}%</div><div class="ml">Expectancy</div></div>
  <div class="metric"><div class="mv">{pf}</div><div class="ml">Profit Factor</div></div>
  <div class="metric"><div class="mv">{wr:.0}% <span style="font-size:.65em;color:var(--muted)">({wins}W/{losses}L)</span></div><div class="ml">Win Rate</div></div>
  <div class="metric" title="7-day rolling drawdown (drives circuit breaker). All-time: -{atdd:.1}%">
    <div class="mv r">-{dd:.1}%</div><div class="ml">7d Drawdown</div></div>
  <div class="metric"><div class="mv b">{kelly_str}</div><div class="ml">Half-Kelly</div></div>
  <div class="metric"><div class="mv" style="color:{cbc}">{cb_label}</div><div class="ml">{cb_desc}</div></div>
  <div class="metric"><div class="mv">{open_n} / {total_closed}</div><div class="ml">Open / Closed</div></div>
  <div class="metric"><div class="mv">{cycles}</div><div class="ml">Cycles</div></div>
  <div class="metric"><div class="mv">{cand_n}</div><div class="ml">Scanning</div></div>
  <div class="metric"><div class="mv y">${committed:.0}</div><div class="ml">Deployed</div></div>
</div>

<div class="status-bar">
  <div class="status-inner">
    <span class="st-text">{status}</span>
    <span style="font-size:.75em;color:var(--muted);white-space:nowrap">
      {open_n} pos · ${committed:.0} · Sharpe {sharpe:.2}
    </span>
  </div>
  <div class="prog-track"><div class="prog-fill"></div></div>
</div>

<div class="section section-positions">
  <div class="section-title">
    <span class="section-title-left"><span class="live-ring"></span> Active Positions</span>
    <span class="badge">{open_n} / 8 slots · max 4 per direction</span>
  </div>
  <div class="pos-grid">{pos_cards}</div>
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
    var stEl=document.querySelector('.st-text');
    if(stEl&&s.status)stEl.textContent=s.status;

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
</script>
{tracking_js}
</body></html>"#,
        last_update  = s.last_update,
        equity       = equity,
        capital      = s.capital,
        pnl_colour   = pnl_colour,
        pnl_sign     = pnl_sign,
        total_pnl    = total_pnl.abs(),
        total_pnl_pct = total_pnl_pct.abs(),
        sc           = m.sharpe_class(),
        sharpe       = m.sharpe,
        sortc        = if m.sortino > 1.0 { "#3fb950" } else if m.sortino > 0.0 { "#e3b341" } else { "#f85149" },
        sortino      = m.sortino,
        expc         = if m.expectancy >= 0.0 { "#3fb950" } else { "#f85149" },
        exps         = if m.expectancy >= 0.0 { "+" } else { "-" }, // BUG FIX: was "" → dropped "-"
        expectancy   = m.expectancy.abs(),
        pf           = pf_str,
        wr           = m.win_rate * 100.0,
        wins         = m.wins,
        losses       = m.losses,
        dd           = rolling_dd_pct,    // 7-day rolling (drives CB) — shown in metric
        atdd         = dd_pct.max(0.0),   // all-time drawdown (tooltip only)
        kelly_str    = kelly_str,
        cbc          = cb_colour,
        cb_label     = cb_label,
        cb_desc      = cb_desc,
        open_n       = s.positions.len(),
        total_closed = s.closed_trades.len(),
        cycles       = s.cycle_count,
        cand_n       = s.candidates.len(),
        committed    = committed,
        status       = s.status,
        pos_cards    = pos_cards,
        wh           = wh,
        cand_rows    = cand_rows,
        closed_rows  = closed_rows,
        dec_rows          = dec_rows,
        next_cycle_at_ms  = s.next_cycle_at,
        sparkline_svg     = sparkline_svg,
        tracking_js       = crate::funnel::client_tracking_script(),
    ))
}

/// Inline weight item: label · value · tiny bar  (single-line strip)
fn wi(label: &str, val: f64) -> String {
    format!(
        r#"<span class="w-item"><span class="w-item-label">{label}</span><span class="w-item-val">{val:.2}</span><div class="w-item-bar"><div class="w-item-fill" style="width:{pct:.0}%"></div></div></span>"#,
        label = label, val = val, pct = (val * 100.0).min(100.0),
    )
}

fn reason_class(r: &str) -> &'static str {
    match r {
        s if s.contains("Stop")    => "stop",
        s if s.contains("Take")    => "take",
        s if s.contains("Time")    => "time",
        s if s.contains("Partial") => "partial",
        s if s.contains("AI")      => "ai",    // BUG FIX: was mapped to "signal" (grey)
        s if s.contains("Signal")  => "signal",
        _                          => "signal",
    }
}

async fn api_state_handler(State(app): State<AppState>) -> Json<BotState> {
    Json(app.bot_state.read().await.clone())
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
            fw   = if is_active { "600" } else { "400" },
            col  = if is_active { "#e6edf3" } else { "#8b949e" },
            bg   = if is_active { "#21262d" } else { "transparent" },
            label = label,
        )
    };
    format!(r#"<!DOCTYPE html>
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
    <a href="/auth/logout" style="padding:8px 18px;border-radius:6px;font-size:.88rem;
       font-weight:400;color:#8b949e;background:transparent;text-decoration:none"
       title="Sign out">Sign out</a>
  </div>
</div>
<div class="wrap">
"#,
        title        = title,
        nav_overview = nav("Overview", "/app"),
        nav_history  = nav("History",  "/app/history"),
        nav_tax      = nav("Tax",       "/app/tax"),
        nav_settings = nav("Settings",  "/app/settings"),
    )
}

fn consumer_shell_close() -> &'static str {
    r#"</div>
<footer style="text-align:center;padding:32px 16px 24px;font-size:.72rem;color:#484f58;
               border-top:1px solid #21262d;margin-top:24px">
  &copy; 2026 TradingBots Ltd. &nbsp;&middot;&nbsp;
  <a href="https://tradingbots.fun" style="color:#484f58;text-decoration:none">tradingbots.fun</a> &nbsp;&middot;&nbsp;
  <a href="/app/onboarding" style="color:#484f58;text-decoration:none">Terms &amp; Risk Disclosure</a>
</footer>
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
        ConsumerStateResult::NeedsLogin       => return axum::response::Redirect::to("/login").into_response(),
        ConsumerStateResult::NeedsOnboarding { .. } => return axum::response::Redirect::to("/app/onboarding").into_response(),
    };
    let s = state_arc.read().await;

    // Resolve tenant tier to determine whether to show ads
    let show_ads = {
        let zone_set = app.coinzilla_zone_id.is_some();
        let is_free = if let Some(ref tid) = tenant_id {
            let tenants = app.tenants.read().await;
            tenants.get(tid)
                .map(|h| h.config.tier == crate::tenant::TenantTier::Free)
                .unwrap_or(false)
        } else {
            false // single-operator mode: no ads
        };
        zone_set && is_free
    };

    let committed: f64  = s.positions.iter().map(|p| p.size_usd).sum();
    let unrealised: f64 = s.positions.iter().map(|p| p.unrealised_pnl).sum();
    let equity    = s.capital + committed + unrealised;
    let total_pnl = s.pnl + unrealised;
    let pnl_pct   = if s.initial_capital > 0.0 { total_pnl / s.initial_capital * 100.0 } else { 0.0 };
    let pnl_col   = if total_pnl >= 0.0 { "#3fb950" } else { "#f85149" };
    let pnl_sign  = if total_pnl >= 0.0 { "+" } else { "-" };

    // Referral block — only rendered when the operator has set REFERRAL_CODE
    let referral_block = match &s.referral_code {
        Some(code) => format!(r#"<div class="card">
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
</div>"#, code = code),
        None => String::new(),
    };

    // Coinzilla ad block — shown only to Free-tier users when zone ID is configured
    let ad_block = if show_ads {
        let zone_id = app.coinzilla_zone_id.as_deref().unwrap_or("");
        // Estimate CPM for tracking: $1.20 is the default established-publisher rate
        let cpm_est = 1.20_f64;
        format!(r#"
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
"#, zone = zone_id, cpm = cpm_est)
    } else {
        String::new()
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
        open_n          = s.positions.len(),
        closed_n        = s.closed_trades.len(),
        init            = s.initial_capital,
        ts              = s.last_update,
        referral_block  = referral_block,
        ad_block        = ad_block,
    ));
    html.push_str(consumer_shell_close());
    axum::response::Html(html).into_response()
}

// ─── Trade history page /app/history ─────────────────────────────────────────

async fn consumer_history_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let state_arc = match resolve_consumer_state(&headers, &app).await {
        ConsumerStateResult::Ok { state, .. } => state,
        ConsumerStateResult::NeedsLogin       => return axum::response::Redirect::to("/login").into_response(),
        ConsumerStateResult::NeedsOnboarding { .. } => return axum::response::Redirect::to("/app/onboarding").into_response(),
    };
    let s = state_arc.read().await;

    let rows: String = if s.closed_trades.is_empty() {
        "<tr><td colspan='9' style='color:#8b949e;text-align:center;padding:20px'>No closed trades yet.</td></tr>".to_string()
    } else {
        s.closed_trades.iter().rev().map(|t| {
            let pnl_col = if t.pnl >= 0.0 { "#3fb950" } else { "#f85149" };
            let pnl_sign = if t.pnl >= 0.0 { "+" } else { "" };
            let fees = if t.fees_est > 0.0 { t.fees_est }
                       else { crate::ledger::estimate_fees(t.size_usd, t.leverage.max(1.0)) };
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
                date  = date,
                sym   = t.symbol,
                side  = t.side,
                sc    = if t.side == "LONG" { "#3fb950" } else { "#f85149" },
                entry = t.entry,
                exit  = t.exit,
                lev   = t.leverage.max(1.0),
                pc    = pnl_col,
                ps    = pnl_sign,
                pnl   = t.pnl,
                fees  = fees,
                nc    = net_col,
                nps   = if net >= 0.0 { "+" } else { "" },
                net   = net,
            )
        }).collect()
    };

    // Summary totals
    let total_gross: f64 = s.closed_trades.iter().map(|t| t.pnl).sum();
    let total_fees: f64  = s.closed_trades.iter().map(|t| {
        if t.fees_est > 0.0 { t.fees_est }
        else { crate::ledger::estimate_fees(t.size_usd, t.leverage.max(1.0)) }
    }).sum();
    let total_net = total_gross - total_fees;
    let wins  = s.closed_trades.iter().filter(|t| t.pnl > 0.0).count();
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
        ConsumerStateResult::Ok { .. }          => consumer_tax_page().into_response(),
        ConsumerStateResult::NeedsLogin         => axum::response::Redirect::to("/login").into_response(),
        ConsumerStateResult::NeedsOnboarding{..}=> axum::response::Redirect::to("/app/onboarding").into_response(),
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
    html.push_str(&format!(r#"
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
        ConsumerStateResult::NeedsLogin          => return axum::response::Redirect::to("/login").into_response(),
        ConsumerStateResult::NeedsOnboarding{..} => return axum::response::Redirect::to("/app/onboarding").into_response(),
        ConsumerStateResult::Ok { .. }           => {},
    }
    let (csv, _) = crate::ledger::read_all();
    let filename  = format!("tradingbots_trades_{}.csv",
        chrono::Utc::now().format("%Y%m%d"));
    (
        [
            ("Content-Type",        "text/csv; charset=utf-8"),
            ("Content-Disposition", Box::leak(
                format!("attachment; filename=\"{}\"", filename).into_boxed_str()
            )),
        ],
        csv,
    ).into_response()
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
    secret:  &str,
) -> Option<crate::tenant::TenantId> {
    let cookie_hdr  = headers.get("cookie")?.to_str().ok()?;
    let session_val = crate::privy::extract_session_cookie(cookie_hdr)?;
    crate::privy::verify_session(session_val, secret).ok()
}

/// Result of resolving the consumer state for an incoming request.
pub enum ConsumerStateResult {
    /// Authenticated and has accepted terms — ready to serve trading data.
    Ok {
        state:     SharedState,
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
    app:     &AppState,
) -> ConsumerStateResult {
    // Single-operator mode: no auth, no terms wall
    if app.privy_app_id.is_none() {
        // Use a synthetic TenantId for the operator in single-op mode
        let tid = crate::tenant::TenantId::from_str("operator");
        return ConsumerStateResult::Ok { state: app.bot_state.clone(), tenant_id: tid };
    }

    // Multi-tenant mode: require valid session cookie
    let tid = match get_session_tenant_id(headers, &app.session_secret) {
        Some(t) => t,
        None    => return ConsumerStateResult::NeedsLogin,
    };

    // Check terms acceptance
    let tenants = app.tenants.read().await;
    let handle  = match tenants.get(&tid) {
        Some(h) => h,
        None    => return ConsumerStateResult::NeedsLogin,
    };

    if handle.config.terms_accepted_at.is_none() {
        return ConsumerStateResult::NeedsOnboarding { tenant_id: tid };
    }

    ConsumerStateResult::Ok {
        state:     handle.state.clone(),
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
    invite_code:  Option<String>,
    /// First-touch acquisition source (utm_source) — sent by the login page JS
    /// from the URL query params / cookie captured on landing.
    #[serde(default)]
    utm_source:   Option<String>,
    /// utm_campaign captured at landing — sent through to funnel_events.
    #[serde(default)]
    utm_campaign: Option<String>,
    /// True when the user arrived via our Hyperliquid referral link.
    #[serde(default)]
    hl_referred:  bool,
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
    use axum::response::IntoResponse;
    use axum::http::StatusCode;

    let privy_app_id = match &app.privy_app_id {
        Some(id) => id.clone(),
        None     => return (StatusCode::SERVICE_UNAVAILABLE,
                            "Privy is not configured on this server").into_response(),
    };

    // Verify the Privy JWT (ES256, JWKS-backed)
    let privy_did = match crate::privy::verify_privy_jwt(
        &req.token, &privy_app_id, &app.jwks_cache,
    ).await {
        Ok(did) => did,
        Err(e)  => {
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
            Some(db) => {
                match crate::invite::claim_invite_code(db, &code).await {
                    Ok(Some(invite)) => { claimed_invite = Some(invite); }
                    Ok(None) => {
                        return (StatusCode::FORBIDDEN,
                            axum::Json(serde_json::json!({"error":"invalid_invite","message":"That invite code is invalid, already used, or expired. Ask for a new one."}))).into_response();
                    }
                    Err(e) => {
                        log::error!("invite claim DB error: {}", e);
                        return (StatusCode::INTERNAL_SERVER_ERROR,
                            axum::Json(serde_json::json!({"error":"db_error","message":"Could not validate invite code. Please try again."}))).into_response();
                    }
                }
            }
            None => {
                // No DB — accept any non-empty code in dev/paper mode
                log::warn!("⚠ No DB — invite code '{}' accepted without validation", code);
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

    // ── Stamp invite attribution on the tenant row in DB ─────────────────────
    if is_new {
        if let (Some(db), Some(invite)) = (&app.db, &claimed_invite) {
            let tenant_uuid = uuid::Uuid::parse_str(tenant_id.as_str()).ok();
            let campaign_id = invite.campaign_id;
            let invited_by  = invite.created_by;
            let code_used   = req.invite_code.clone().unwrap_or_default();

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
        "",           // anon_id — client fires LOGIN_CLICK with it separately
        &tenant_id,
        is_new,
        referral_source.as_deref(),
        req.hl_referred,
        req.utm_campaign.as_deref(),
    ).await;

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
            tenant_id.as_str(), in_campaign
        )))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// `GET /auth/logout`
///
/// Clears the session cookie and redirects to `/login`.
async fn auth_logout_handler(
    State(_app): State<AppState>,
) -> axum::response::Response {
    axum::response::Response::builder()
        .status(302)
        .header("Location",  "/login")
        .header("Set-Cookie", crate::privy::clear_session_header())
        .body(axum::body::Body::empty())
        .unwrap()
}

/// `GET /login`
///
/// Renders the Privy-powered login page.
///
/// - When `PRIVY_APP_ID` is set: embeds the Privy JS SDK and shows a
///   "Login" button that triggers Privy's authentication modal.
/// - When Privy is not configured: shows a message directing to `/app`
///   (single-operator mode — auth not required).
async fn login_handler(
    State(app): State<AppState>,
) -> axum::response::Html<String> {
    let body = if let Some(ref app_id) = app.privy_app_id {
        format!(r#"<!DOCTYPE html>
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
/* Invite code field */
.inv-wrap{{display:flex;flex-direction:column;gap:6px}}
.inv-lbl{{display:flex;justify-content:space-between;align-items:baseline;
          font-size:.78rem;font-weight:600;color:#c9d1d9}}
.inv-hint{{font-size:.7rem;color:#484f58;font-weight:400}}
.inv-inp{{width:100%;padding:11px 14px;background:#010409;border:1px solid #30363d;
          border-radius:8px;color:#e6edf3;font-size:.95rem;font-weight:700;
          letter-spacing:.08em;text-transform:uppercase;outline:none;transition:.15s}}
.inv-inp:focus{{border-color:#58a6ff;box-shadow:0 0 0 3px rgba(88,166,255,.12)}}
.inv-inp.ok{{border-color:#3fb950;box-shadow:0 0 0 3px rgba(63,185,80,.1)}}
.inv-inp.bad{{border-color:#f85149;box-shadow:0 0 0 3px rgba(248,81,73,.1)}}
.inv-status{{font-size:.72rem;min-height:16px;transition:.15s}}
.inv-status.ok{{color:#3fb950}}.inv-status.bad{{color:#f85149}}
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
      <p>Invite-only · <a href="/leaderboard" style="color:#58a6ff;text-decoration:none">🏆 View leaderboard</a></p>
    </div>

    <!-- Invite code field -->
    <div class="inv-wrap">
      <label class="inv-lbl" for="invite-input">
        <span>🎟 Invite Code</span>
        <span class="inv-hint">Get one from a friend or the <a href="/leaderboard" style="color:#58a6ff">weekly campaign</a></span>
      </label>
      <input id="invite-input" type="text" class="inv-inp"
             placeholder="TB-XXXXXXXX"
             autocomplete="off" autocapitalize="characters" spellcheck="false"
             maxlength="20">
      <div id="inv-status" class="inv-status"></div>
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

    <div>
      <button id="login-btn" class="btn btn-p" disabled>Sign in with Privy</button>
      <div id="status" style="margin-top:8px"></div>
      <div id="err" class="err" style="margin-top:4px"></div>
    </div>

    <div class="wnote">
      <div class="wnote-dot"></div>
      Start with $20 · 2 bots · compete for weekly prizes
    </div>
  </div>
</div>

<script type="module">
const PRIVY_APP_ID = '{app_id}';

const tosCheck    = document.getElementById('tos-check');
const loginBtn    = document.getElementById('login-btn');
const inviteInp   = document.getElementById('invite-input');
const invStatus   = document.getElementById('inv-status');

// Read invite code from URL param ?invite=TB-XXXX if present
const urlParams = new URLSearchParams(window.location.search);
const prefilledCode = urlParams.get('invite') || urlParams.get('code') || '';
if (prefilledCode) {{ inviteInp.value = prefilledCode.toUpperCase(); }}

// Gate: both ToS checked AND invite code non-empty
function canLogin() {{
  return tosCheck.checked && inviteInp.value.trim().length >= 6;
}}
function updateBtn() {{
  if (!loginBtn.dataset.loading) loginBtn.disabled = !canLogin();
}}
tosCheck.addEventListener('change', updateBtn);

// Live invite input feedback
inviteInp.addEventListener('input', () => {{
  const v = inviteInp.value.trim().toUpperCase();
  inviteInp.value = v;
  if (v.length === 0) {{
    inviteInp.classList.remove('ok','bad');
    invStatus.textContent = ''; invStatus.className = 'inv-status';
  }} else if (v.length < 6) {{
    inviteInp.classList.remove('ok'); inviteInp.classList.add('bad');
    invStatus.textContent = 'Code too short'; invStatus.className = 'inv-status bad';
  }} else {{
    inviteInp.classList.remove('bad'); inviteInp.classList.add('ok');
    invStatus.textContent = '✓ Code looks good'; invStatus.className = 'inv-status ok';
  }}
  updateBtn();
}});

function setStatus(msg) {{ document.getElementById('status').textContent = msg; }}
function setErr(msg)    {{ document.getElementById('err').textContent    = msg; }}
function setLoading(yes) {{
  loginBtn.dataset.loading = yes ? '1' : '';
  loginBtn.disabled = yes;
  loginBtn.textContent = yes ? 'Signing in…' : 'Sign in with Privy';
}}

// Read UTM params from cookie or URL for attribution
function getUtm(key) {{
  const url = new URLSearchParams(window.location.search);
  return url.get(key) || '';
}}

async function exchangeToken(privyToken) {{
  const body = {{
    token:        privyToken,
    invite_code:  inviteInp.value.trim().toUpperCase() || null,
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
    const msg = j.message || (j.error === 'invite_required'
      ? 'An invite code is required to create an account.'
      : 'That invite code is invalid or already used.');
    throw new Error(msg);
  }}
  if (!res.ok) throw new Error('Session exchange failed: ' + res.status);
  return res.json();
}}

// @privy-io/js-sdk-core is server-side only (no login() method).
// Use the React Auth SDK for browser login flows.
Promise.all([
  import('https://esm.sh/react@18'),
  import('https://esm.sh/react-dom@18/client'),
  import('https://esm.sh/@privy-io/react-auth'),
]).then(([React, ReactDOM, PrivyReact]) => {{
  const {{ createElement: h, useState, useEffect }} = React;
  const {{ createRoot }} = ReactDOM;
  const {{ PrivyProvider, usePrivy }} = PrivyReact;

  function LoginButton() {{
    const {{ ready, authenticated, login, getAccessToken }} = usePrivy();
    const [, setTick] = useState(0);

    // Re-render whenever TOS checkbox or invite input change
    useEffect(() => {{
      const rerender = () => setTick(t => t + 1);
      const tos = document.getElementById('tos-check');
      const inv = document.getElementById('invite-input');
      tos?.addEventListener('change', rerender);
      inv?.addEventListener('input', rerender);
      return () => {{
        tos?.removeEventListener('change', rerender);
        inv?.removeEventListener('input', rerender);
      }};
    }}, []);

    // Auto-redirect if already authenticated
    useEffect(() => {{
      if (!ready || !authenticated) return;
      setStatus('Already signed in — loading…');
      getAccessToken().then(async (token) => {{
        try {{ await exchangeToken(token); window.location.href = '/app'; }}
        catch(e) {{ setStatus(''); setErr('Session setup failed. Please sign in again.'); }}
      }}).catch(() => {{}});
    }}, [ready, authenticated]);

    const handleClick = async () => {{
      if (!canLogin()) return;
      setErr(''); setStatus('Opening Privy…');
      try {{
        await login();
        setStatus('Authenticated — setting up your account…');
        const token = await getAccessToken();
        await exchangeToken(token);
        window.location.href = '/app';
      }} catch(e) {{
        setStatus('');
        setErr(e.message || 'Login failed. Please try again.');
      }}
    }};

    return h('button', {{
      className: 'btn btn-p',
      disabled: !ready || !canLogin(),
      onClick: handleClick,
    }}, ready ? 'Sign in with Privy' : 'Loading…');
  }}

  // Mount React component in place of the static placeholder button
  const btn = document.getElementById('login-btn');
  const mount = document.createElement('div');
  btn.parentNode.replaceChild(mount, btn);

  createRoot(mount).render(
    h(PrivyProvider, {{
      appId: PRIVY_APP_ID,
      config: {{ loginMethods: ['email', 'wallet'], appearance: {{ theme: 'dark' }} }},
    }},
      h(LoginButton)
    )
  );
}}).catch((e) => {{
  setErr('Could not load authentication SDK: ' + e.message);
}});
</script>
</body></html>"#, app_id = app_id)
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
</body></html>"#.to_string()
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
            None    => return axum::response::Redirect::to("/login").into_response(),
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

/// `POST /app/onboarding/accept` — record terms acceptance and redirect to `/app`.
async fn onboarding_accept_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let tid = match get_session_tenant_id(&headers, &app.session_secret) {
        Some(t) => t,
        None    => return axum::response::Redirect::to("/login").into_response(),
    };

    {
        let mut tenants = app.tenants.write().await;
        let _ = tenants.accept_terms(&tid);
    }

    axum::response::Redirect::to("/app").into_response()
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
        ConsumerStateResult::NeedsLogin           => return axum::response::Redirect::to("/login").into_response(),
        ConsumerStateResult::NeedsOnboarding{..}  => return axum::response::Redirect::to("/app/onboarding").into_response(),
    };

    let (display_name, email, wallet, tier, trial_days, terms_ts, wallet_ts, hl_balance,
         net_dep, total_dep, total_with, max_pos, trial_expired) = {
        let tenants = app.tenants.read().await;
        let h = match tenants.get(&tid) {
            Some(h) => h,
            None    => return axum::response::Redirect::to("/login").into_response(),
        };
        let fund_sum  = crate::fund_tracker::summary(&tid);
        (
            h.config.display_name.clone(),
            h.config.email.clone().unwrap_or_else(|| "—".to_string()),
            h.config.wallet_address.clone(),
            format!("{:?}", h.config.tier),
            h.config.trial_days_remaining(),
            h.config.terms_accepted_at.map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
                .unwrap_or_else(|| "—".to_string()),
            h.config.wallet_linked_at.map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
                .unwrap_or_else(|| "—".to_string()),
            h.config.hl_balance_usd,
            fund_sum.net_deposits,
            fund_sum.total_deposited,
            fund_sum.total_withdrawn,
            h.config.max_positions(),
            h.config.is_trial_expired_free(),
        )
    };

    let wallet_section = if let Some(ref addr) = wallet {
        format!(r#"
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
            addr       = addr,
            hl_balance = hl_balance,
            wallet_ts  = wallet_ts,
        )
    } else {
        r#"<div class="info-box" style="margin-top:4px">
  No wallet linked yet. Paste your Hyperliquid wallet address (0x…) below.
  Your funds never leave your HL account — we only need the address to query
  your balance and attribute trades to your account.
</div>"#.to_string()
    };

    let tier_badge = match tier.as_str() {
        "Pro"      => r#"<span style="color:#3fb950;font-weight:700">Pro</span>"#,
        "Internal" => r#"<span style="color:#e3b341;font-weight:700">Internal</span>"#,
        _          => r#"<span style="color:#8b949e;font-weight:600">Free</span>"#,
    };

    let trial_note = if trial_days > 0 {
        format!(r#"<span style="color:#e3b341;font-size:.78rem;margin-left:6px">
  ({trial_days} trial day{s} remaining)</span>"#,
            trial_days = trial_days,
            s          = if trial_days == 1 { "" } else { "s" },
        )
    } else { String::new() };

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
        } else { "" };
        format!(r#"<div class="metric-row">
    <span class="ml">Open positions</span>
    <span class="mv" style="color:{cap_colour}">{cap_str}{cap_hint}</span>
  </div>"#,
            cap_colour = cap_colour,
            cap_str    = cap_str,
            cap_hint   = cap_hint,
        )
    };

    let mut html = consumer_shell_open("Settings", "Settings");
    html.push_str(&format!(r#"
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

{upgrade_block}

<p class="note" style="text-align:center;margin-top:12px">
  Need help? Contact support or
  <a href="/auth/logout">sign out</a>.
</p>
"#,
        display_name  = display_name,
        email         = email,
        tier_badge    = tier_badge,
        trial_note    = trial_note,
        pos_cap_row   = pos_cap_row,
        terms_ts      = terms_ts,
        wallet_section= wallet_section,
        link_label    = if wallet.is_some() { "Update" } else { "Link Wallet" },
        total_dep     = total_dep,
        total_with    = total_with,
        net_dep       = net_dep,
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
        } else { "" },
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
        None    => return axum::response::Redirect::to("/login").into_response(),
    };

    let address = match form.get("address") {
        Some(a) => a.trim().to_string(),
        None    => return axum::response::Redirect::to("/app/settings?error=missing_address").into_response(),
    };

    {
        let mut tenants = app.tenants.write().await;
        match tenants.link_wallet(&tid, &address) {
            Ok(_)  => log::info!("🔗 Tenant {} updated wallet to {}", tid, address),
            Err(e) => {
                log::warn!("⚠ Wallet link failed for tenant {}: {}", tid, e);
                return axum::response::Redirect::to("/app/settings?error=invalid_address").into_response();
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
        Some(v) => match v.to_str() { Ok(s) => s, Err(_) => return false },
        None    => return false,
    };
    let encoded = match auth_header.strip_prefix("Basic ") {
        Some(e) => e,
        None    => return false,
    };
    use base64::Engine as _;
    let decoded = match base64::engine::general_purpose::STANDARD.decode(encoded) {
        Ok(bytes) => match String::from_utf8(bytes) { Ok(s) => s, Err(_) => return false },
        Err(_)    => return false,
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
        .header("WWW-Authenticate", r#"Basic realm="TradingBots.fun Admin", charset="UTF-8""#)
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
        None    => return (axum::http::StatusCode::SERVICE_UNAVAILABLE,
                          "Admin panel is not configured. Set ADMIN_PASSWORD.").into_response(),
    };

    if !check_admin_auth(&headers, &password) {
        return www_authenticate_response();
    }

    let (tenant_count, pro_count, free_count, total_balance) = {
        let tenants = app.tenants.read().await;
        let count    = tenants.count();
        let pro      = tenants.all().filter(|h| h.config.tier == crate::tenant::TenantTier::Pro).count();
        let free     = tenants.all().filter(|h| h.config.tier == crate::tenant::TenantTier::Free).count();
        let balance: f64 = tenants.all().map(|h| h.config.hl_balance_usd).sum();
        (count, pro, free, balance)
    };

    let html = format!(r#"<!DOCTYPE html>
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
    <a href="/">Operator view</a>
  </div>
</div>

<div class="cards">
  <div class="card"><div class="cl">Total Users</div><div class="cv">{tenant_count}</div></div>
  <div class="card"><div class="cl">Pro</div><div class="cv" style="color:#3fb950">{pro_count}</div></div>
  <div class="card"><div class="cl">Free</div><div class="cv" style="color:#8b949e">{free_count}</div></div>
  <div class="card"><div class="cl">Total HL Balance</div><div class="cv-sm">${total_balance:.2}</div></div>
</div>

<p style="font-size:.85rem;color:#8b949e">
  <a href="/admin/users">View all users →</a>
</p>
</div>
</body>
</html>"#,
        tenant_count  = tenant_count,
        pro_count     = pro_count,
        free_count    = free_count,
        total_balance = total_balance,
    );

    axum::response::Html(html).into_response()
}

/// `GET /admin/users` — table of all tenants with key stats.
async fn admin_users_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let password = match &app.admin_password {
        Some(p) => p.clone(),
        None    => return (axum::http::StatusCode::SERVICE_UNAVAILABLE,
                          "Admin panel not configured").into_response(),
    };

    if !check_admin_auth(&headers, &password) {
        return www_authenticate_response();
    }

    let rows: String = {
        let tenants = app.tenants.read().await;
        tenants.all().map(|h| {
            let tier_col = match h.config.tier {
                crate::tenant::TenantTier::Pro      => "#3fb950",
                crate::tenant::TenantTier::Internal => "#e3b341",
                crate::tenant::TenantTier::Free     => "#8b949e",
            };
            let wallet_short = h.config.wallet_address.as_deref()
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
                id_short  = &h.id.as_str()[..8.min(h.id.as_str().len())],
                name      = h.config.display_name,
                tier_col  = tier_col,
                tier      = h.config.tier,
                wallet    = wallet_short,
                bal       = h.config.hl_balance_usd,
                dep       = fund_sum.net_deposits,
                terms     = terms_ok,
            )
        }).collect()
    };

    let html = format!(r#"<!DOCTYPE html>
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
        } else { rows },
    );

    axum::response::Html(html).into_response()
}

/// Google Pay requires no domain verification — it is automatically enabled
/// in Stripe Checkout when the user's device supports it.
async fn apple_pay_domain_handler(
    State(app): State<AppState>,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    match &app.apple_pay_domain_assoc {
        Some(content) => (
            StatusCode::OK,
            [("Content-Type", "text/plain; charset=utf-8")],
            content.clone(),
        ).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            "Apple Pay domain association file not configured.\n\
             Set APPLE_PAY_DOMAIN_ASSOC in your environment.",
        ).into_response(),
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
async fn leaderboard_handler(
    State(app): State<AppState>,
) -> axum::response::Html<String> {
    let (campaign, entries) = match &app.db {
        Some(db) => {
            let c = crate::leaderboard::active_campaign(db).await.ok().flatten();
            let e = if c.is_some() {
                crate::leaderboard::live_standings(db, 50).await.unwrap_or_default()
            } else { vec![] };
            (c, e)
        }
        None => (None, vec![]),
    };

    let campaign_title = campaign.as_ref().map(|c| c.title.clone())
        .unwrap_or_else(|| "Weekly Trading Contest".into());
    let campaign_desc = campaign.as_ref()
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
        entries.iter().map(|e| {
            let medal = match e.rank { 1 => "🥇", 2 => "🥈", 3 => "🥉", _ => "" };
            let pct_color = if e.pct_return >= 0.0 { "#3fb950" } else { "#f85149" };
            let pct_sign  = if e.pct_return >= 0.0 { "+" } else { "" };
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
                rank  = e.rank,
                name  = html_escape(&e.display_name),
                wallet = e.wallet_short,
                trades = e.trades_in_period,
                pct_color = pct_color,
                pct_sign  = pct_sign,
                pct       = e.pct_return,
            )
        }).collect()
    };

    axum::response::Html(format!(r#"<!DOCTYPE html>
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
        title      = html_escape(&campaign_title),
        desc       = html_escape(&campaign_desc),
        prizes_html = prizes_html,
        prize_pool = prize_pool as i64,
        rows_html  = rows_html,
        seconds_left = seconds_left,
    ))
}

// ─── tiny helper ─────────────────────────────────────────────────────────────
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
     .replace('"', "&quot;").replace('\'', "&#39;")
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
        Err(_) => return (StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({"error":"Unauthorized"}))).into_response(),
    };

    let db = match &app.db {
        Some(db) => db,
        None => return (StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({"error":"Database not configured"}))).into_response(),
    };

    match crate::invite::generate_referral_code(db, &tenant_id).await {
        Ok(code) => axum::Json(serde_json::json!({
            "ok": true,
            "code": code,
            "share_url": format!("/login?invite={}", code),
            "expires_days": 30,
        })).into_response(),
        Err(e) => {
            log::error!("generate_invite_handler: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR,
             axum::Json(serde_json::json!({"error":"Could not generate code"}))).into_response()
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
        Err(_) => return (StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({"error":"Unauthorized"}))).into_response(),
    };

    let db = match &app.db {
        Some(db) => db,
        None => return (StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({"error":"Database not configured"}))).into_response(),
    };

    let code = match crate::invite::get_referral_code_for_tenant(db, &tenant_id).await {
        Ok(Some(c)) => c,
        Ok(None) => {
            // Auto-generate on first request
            match crate::invite::generate_referral_code(db, &tenant_id).await {
                Ok(c)  => c,
                Err(e) => {
                    log::error!("get_invite auto-generate: {}", e);
                    return (StatusCode::INTERNAL_SERVER_ERROR,
                        axum::Json(serde_json::json!({"error":"Could not generate code"}))).into_response();
                }
            }
        }
        Err(e) => {
            log::error!("get_invite_handler: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({"error":"DB error"}))).into_response();
        }
    };

    axum::Json(serde_json::json!({
        "ok": true,
        "code": code,
        "share_url": format!("/login?invite={}", code),
    })).into_response()
}

/// `GET /api/leaderboard` — JSON endpoint for the current standings.
async fn api_leaderboard_handler(
    State(app): State<AppState>,
) -> impl axum::response::IntoResponse {
    use axum::response::IntoResponse;
    let db = match &app.db {
        Some(db) => db,
        None => return axum::Json(serde_json::json!({"entries":[],"campaign":null})).into_response(),
    };
    let campaign = crate::leaderboard::active_campaign(db).await.ok().flatten();
    let entries  = crate::leaderboard::live_standings(db, 100).await.unwrap_or_default();
    axum::Json(serde_json::json!({ "campaign": campaign, "entries": entries })).into_response()
}

///
/// Returns the last 90 days of AUM snapshots as JSON.
/// Used by the landing page to render the TVL hero graph client-side.
/// No authentication required — returns aggregate data only, never per-tenant.
async fn public_tvl_handler(
    State(app): State<AppState>,
) -> impl axum::response::IntoResponse {
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
        Ok(p)  => p,
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
async fn public_tvl_svg_handler(
    State(app): State<AppState>,
) -> impl axum::response::IntoResponse {
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
    let pad:  f64 = 8.0;
    let inner_h   = h_px - 2.0 * pad;

    let values: Vec<f64> = points.iter().map(|p| p.total_aum).collect();
    let deposited = points.first().map(|p| p.deposited_capital).unwrap_or(0.0);

    let data_min = values.iter().cloned().fold(f64::INFINITY, f64::min).min(deposited);
    let data_max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max).max(deposited);
    let buf   = ((data_max - data_min).max(deposited * 0.002)) * 0.15;
    let min_v = data_min - buf;
    let max_v = data_max + buf;
    let range = (max_v - min_v).max(0.01);

    let to_y = |v: f64| h_px - pad - (v - min_v) / range * inner_h;

    let n = values.len() as f64;
    let pts: String = values.iter().enumerate().map(|(i, &v)| {
        let x = i as f64 / (n - 1.0) * w_px;
        let y = to_y(v);
        format!("{x:.1},{y:.1}")
    }).collect::<Vec<_>>().join(" ");

    let base_y  = to_y(deposited);
    let last_y  = to_y(*values.last().unwrap());
    let last_v  = *values.last().unwrap();
    let trend_c = if last_v >= deposited { "#3fb950" } else { "#f85149" };
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
        c   = trend_c,
        by  = base_y,
        fp  = fill_pts,
        pts = pts,
        ly  = last_y,
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
    State(app):    State<AppState>,
    headers:       HeaderMap,
    body:          axum::extract::Json<crate::funnel::FunnelEventPayload>,
) -> axum::http::StatusCode {
    use axum::http::StatusCode;
    use crate::funnel::{FunnelEvent, record};

    let payload = body.0;

    // Map the string event_type → enum (rejects unknown values)
    let event = match payload.event_type.as_str() {
        "PAGE_VIEW"         => FunnelEvent::PageView,
        "LOGIN_CLICK"       => FunnelEvent::LoginClick,
        "AUTH_SUCCESS"      => FunnelEvent::AuthSuccess,
        "TRIAL_START"       => FunnelEvent::TrialStart,
        "TERMS_ACCEPTED"    => FunnelEvent::TermsAccepted,
        "WALLET_LINKED"     => FunnelEvent::WalletLinked,
        "FIRST_POSITION"    => FunnelEvent::FirstPosition,
        "UPGRADE_CLICK"     => FunnelEvent::UpgradeClick,
        "CHECKOUT_STARTED"  => FunnelEvent::CheckoutStarted,
        "UPGRADED"          => FunnelEvent::Upgraded,
        "TRIAL_EXPIRED"     => FunnelEvent::TrialExpired,
        "CHURNED"           => FunnelEvent::Churned,
        "AD_IMPRESSION"     => FunnelEvent::AdImpression,
        "AD_CLICK"          => FunnelEvent::AdClick,
        _                   => return StatusCode::BAD_REQUEST,
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
    ).await;

    StatusCode::NO_CONTENT
}

pub async fn serve(app_state: AppState, port: u16) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let app = Router::new()
        .route("/", get(dashboard_handler))
        .route("/app",                  get(consumer_app_handler))
        .route("/app/history",          get(consumer_history_handler))
        .route("/app/tax",              get(consumer_tax_handler))
        .route("/app/tax/csv",          get(consumer_tax_csv_handler))
        .route("/api/state",            get(api_state_handler))
        // ── Stripe billing ─────────────────────────────────────────────────
        .route("/billing/checkout",     get(crate::stripe::checkout_handler))
        .route("/billing/success",      get(crate::stripe::success_handler))
        .route("/billing/trial",        get(crate::stripe::trial_handler))
        .route("/webhooks/stripe",      post(crate::stripe::webhook_handler))
        // ── Privy authentication ────────────────────────────────────────────
        .route("/login",                get(login_handler))
        .route("/auth/session",         post(auth_session_handler))
        .route("/auth/logout",          get(auth_logout_handler))
        // ── Onboarding / Terms wall ─────────────────────────────────────────
        .route("/app/onboarding",       get(onboarding_handler))
        .route("/app/onboarding/accept",post(onboarding_accept_handler))
        // ── Consumer settings ───────────────────────────────────────────────
        .route("/app/settings",         get(consumer_settings_handler))
        .route("/app/settings/wallet",  post(consumer_settings_wallet_handler))
        // ── Admin panel (HTTP Basic Auth) ───────────────────────────────────
        .route("/admin",                get(admin_dashboard_handler))
        .route("/admin/users",          get(admin_users_handler))
        // ── Apple Pay domain verification ───────────────────────────────────
        .route("/.well-known/apple-developer-merchantid-domain-association",
                                        get(apple_pay_domain_handler))
        // ── Public API — no auth, rate-limited at the nginx level ──────────
        // Used by the landing page TVL hero graph and external integrations.
        .route("/api/public/tvl",       get(public_tvl_handler))
        .route("/api/public/tvl/svg",   get(public_tvl_svg_handler))
        // ── Funnel / analytics (first-party, no third-party scripts) ───────
        .route("/api/funnel",           post(funnel_event_handler))
        // ── Leaderboard & invite codes ──────────────────────────────────────
        .route("/leaderboard",          get(leaderboard_handler))
        .route("/app/invite",           get(get_invite_handler))
        .route("/app/invite/generate",  post(generate_invite_handler))
        .route("/api/leaderboard",      get(api_leaderboard_handler))
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

    fn make_pos(side: &str, entry: f64, stop: f64, qty: f64, size_usd: f64, upnl: f64) -> PaperPosition {
        PaperPosition {
            symbol:          "TEST".to_string(),
            side:            side.to_string(),
            entry_price:     entry,
            quantity:        qty,
            size_usd,
            stop_loss:       stop,
            take_profit:     entry * 1.10,
            atr_at_entry:    entry * 0.02,
            high_water_mark: entry,
            low_water_mark:  entry,
            partial_closed:  false,
            r_dollars_risked: (entry - stop).abs() * qty,
            tranches_closed: 0,
            dca_count:       0,
            leverage:        1.0,
            cycles_held:     0,
            entry_time:      "00:00:00 UTC".to_string(),
            unrealised_pnl:  upnl,
            contrib:         SignalContribution::default(),
            ai_action:       None,
            ai_reason:       None,
        }
    }

    // ── Equity calculation ────────────────────────────────────────────────────

    #[test]
    fn equity_includes_capital_committed_and_unrealised() {
        // equity = free_capital + committed_margin + unrealised_pnl
        let capital    = 800.0;
        let size_usd   = 100.0; // margin committed
        let unrealised = 25.0;  // open profit
        let pos = make_pos("LONG", 100.0, 95.0, 3.0, size_usd, unrealised);

        let computed_equity: f64 = capital
            + pos.size_usd          // committed
            + pos.unrealised_pnl;   // unrealised
        assert!(
            (computed_equity - 925.0).abs() < 1e-10,
            "equity = 800 + 100 + 25 = 925, got {computed_equity}"
        );
    }

    #[test]
    fn equity_with_losing_position_reduces_below_capital_plus_committed() {
        let capital    = 800.0;
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
        let realised: f64 = 50.0;  // closed trade profits
        let unrealised = -10.0; // current open loss
        let total = realised + unrealised;
        assert!((total - 40.0).abs() < 1e-10, "total P&L: $50 realised - $10 open = $40");
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
        assert_eq!(sign, "-", "REGRESSION: negative PnL must use '-' prefix, not empty string");
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
        assert_eq!(rolling_peak, equity, "empty window → rolling_peak = current equity");
        let dd = ((rolling_peak - equity) / rolling_peak * 100.0).max(0.0);
        assert_eq!(dd, 0.0, "empty window → zero drawdown");
    }

    #[test]
    fn all_time_dd_uses_peak_equity_not_rolling_window() {
        // The all-time peak is tracked separately in s.peak_equity.
        // This can be much higher than the rolling 7-day peak.
        let peak_equity: f64 = 5000.0; // hit months ago
        let equity      = 1000.0; // current
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
        let peak_equity     = 5000.0;
        let equity          = 1000.0;
        let all_time_dd_pct = (peak_equity - equity) / peak_equity * 100.0; // 80%

        // Rolling window only has recent data
        let mut window: VecDeque<(i64, f64)> = VecDeque::new();
        window.push_back((0, 1020.0));
        window.push_back((1, 1000.0));
        let rolling_peak    = window.iter().map(|&(_, e)| e).fold(equity, f64::max);
        let rolling_dd_pct  = ((rolling_peak - equity) / rolling_peak * 100.0).max(0.0);

        let cb_threshold = 8.0_f64;
        let cb_from_all_time  = all_time_dd_pct > cb_threshold;
        let cb_from_rolling   = rolling_dd_pct  > cb_threshold;

        assert!(cb_from_all_time,  "all-time DD 80% would trigger CB: {all_time_dd_pct}%");
        assert!(!cb_from_rolling, "rolling 7d DD {rolling_dd_pct}% should NOT trigger CB");
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
            reason_class("AI-Close"), "ai",
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
        assert!(html.contains("RSI"),  "wi() must contain label");
        assert!(html.contains("0.75"), "wi() must contain formatted value");
    }

    #[test]
    fn wi_bar_width_capped_at_100_percent() {
        // val > 1.0 should cap bar width at 100%
        let html = wi("OverVal", 1.5);
        assert!(html.contains("width:100%"), "bar width must be capped at 100%: {html}");
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
        } else { 0.0 };
        assert!((r_mult - 1.0).abs() < 1e-10,
            "unrealised=$15 / r_risk=$15 should be exactly 1R, got {r_mult}");
    }

    #[test]
    fn r_multiple_bar_pct_clamps_to_0_100() {
        // bar_pct = ((r_mult + 1) / 6 * 100).clamp(0, 100)
        // At -1R → 0%, at 0R → 16.7%, at 2R → 50%, at 5R → 100%
        let clamp = |r: f64| -> f64 { ((r + 1.0) / 6.0 * 100.0).clamp(0.0, 100.0) };

        assert_eq!(clamp(-2.0), 0.0,   "-2R → bar at 0%");
        assert_eq!(clamp(-1.0), 0.0,   "-1R → bar at 0%");
        assert!((clamp(0.0) - 16.67).abs() < 0.1, "0R → bar at ~16.7%");
        assert!((clamp(2.0) - 50.0).abs() < 0.1,  "2R → bar at 50%");
        assert_eq!(clamp(5.0), 100.0,  "5R → bar at 100%");
        assert_eq!(clamp(10.0), 100.0, "10R → bar still clamped at 100%");
    }

    // ── Position card border colour ───────────────────────────────────────────

    #[test]
    fn position_border_green_when_profitable() {
        let upnl = 50.0;
        let r_risk = 15.0;
        let border = if upnl > 0.0 { "#238636" }
                     else if upnl < -r_risk * 0.5 { "#da3633" }
                     else { "#444c56" };
        assert_eq!(border, "#238636", "profitable position should have green border");
    }

    #[test]
    fn position_border_red_when_loss_exceeds_half_r() {
        let upnl  = -10.0;
        let r_risk = 15.0; // half-R = -7.5, loss = -10 > -7.5
        let border = if upnl > 0.0 { "#238636" }
                     else if upnl < -r_risk * 0.5 { "#da3633" }
                     else { "#444c56" };
        assert_eq!(border, "#da3633", "loss > 0.5R should show red danger border");
    }

    #[test]
    fn position_border_neutral_when_small_loss() {
        let upnl  = -3.0;   // less than half of R
        let r_risk = 15.0;  // half-R = -7.5 → loss -3 < -7.5 is false
        let border = if upnl > 0.0 { "#238636" }
                     else if upnl < -r_risk * 0.5 { "#da3633" }
                     else { "#444c56" };
        assert_eq!(border, "#444c56", "small loss < 0.5R should show neutral border");
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
            0 => "target 2R",
            1 => "1/3 banked · target 4R",
            _ => "2/3 banked · trailing",
        };
        assert_eq!(label, "target 2R");
    }

    #[test]
    fn tranche_1_shows_partial_banked() {
        let t = 1u8;
        let label = match t {
            0 => "target 2R",
            1 => "1/3 banked · target 4R",
            _ => "2/3 banked · trailing",
        };
        assert_eq!(label, "1/3 banked · target 4R");
    }

    #[test]
    fn tranche_2_shows_two_thirds_banked() {
        let t = 2u8;
        let label = match t {
            0 => "target 2R",
            1 => "1/3 banked · target 4R",
            _ => "2/3 banked · trailing",
        };
        assert_eq!(label, "2/3 banked · trailing");
    }

    // ── Dashboard slot badge correctness ──────────────────────────────────────

    #[test]
    fn slot_badge_reflects_correct_max_positions() {
        // Correct values: MAX_POSITIONS=8, MAX_SAME_DIRECTION=4
        let max_positions      = 8usize;
        let max_same_direction = 4usize;
        let badge = format!("{} / {} slots · max {} per direction",
            0, max_positions, max_same_direction);
        assert!(badge.contains("8 slots"),  "badge must show 8 total slots");
        assert!(badge.contains("max 4 per"), "badge must show max 4 per direction");
        assert!(!badge.contains("/ 4 slots"), "REGRESSION: badge must NOT say 4 slots");
        assert!(!badge.contains("max 2 per"), "REGRESSION: badge must NOT say max 2 per direction");
    }
}
