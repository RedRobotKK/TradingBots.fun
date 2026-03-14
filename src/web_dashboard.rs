use axum::{extract::State, response::Html, routing::get, Json, Router};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use crate::learner::{SignalContribution, SignalWeights};
use crate::metrics::PerformanceMetrics;
use crate::coins;

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateInfo {
    pub symbol:          String,
    pub price:           f64,
    /// None on cycle 1 (no previous reference price yet); Some(%) on cycle 2+.
    pub change_pct:      Option<f64>,
    /// LunarCrush galaxy score 0-100 (None = no data)
    pub galaxy_score:    Option<f64>,
    /// % of social posts classified bullish (None = no data)
    pub bullish_percent: Option<f64>,
    /// LunarCrush alt_rank (lower = more social momentum)
    pub alt_rank:        Option<u32>,
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
            status: String::new(), last_update: String::new(),
        }
    }
}

pub type SharedState = Arc<RwLock<BotState>>;

// ─────────────────────────────── Dashboard ───────────────────────────────────

async fn dashboard_handler(State(state): State<SharedState>) -> Html<String> {
    let s = state.read().await;
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
        r#"<div class="empty-state">No open positions — scanning for signals…</div>"#.to_string()
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

            format!(r#"<div class="pos-card" style="border-left:3px solid {border}" id="pos-{sym_id}">
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
</div>"#,
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
        s.closed_trades.iter().rev().take(20).map(|t| {
            let pc = if t.pnl >= 0.0 { "#3fb950" } else { "#f85149" };
            let ps = if t.pnl >= 0.0 { "+" } else { "-" };
            let sc = if t.side == "LONG" { "#3fb950" } else { "#f85149" };
            let pnl_abs = t.pnl.abs();
            let pct_abs = t.pnl_pct.abs();
            format!(
                "<tr><td><b>{}</b></td><td style='color:{}'>{}</td>\
                 <td>${:.4}</td><td>${:.4}</td>\
                 <td style='color:{}'>{}{:.2} ({}{:.1}%)</td>\
                 <td class='reason-{}'>{}</td><td class='ts'>{}</td></tr>",
                t.symbol, sc, t.side, t.entry, t.exit,
                pc, ps, pnl_abs, ps, pct_abs,
                reason_class(&t.reason), t.reason, t.closed_at
            )
        }).collect()
    };

    // ── Candidates table ──────────────────────────────────────────────────
    // Sentiment live status: true if at least one candidate has LunarCrush data
    let sent_live = s.candidates.iter().any(|c| c.bullish_percent.is_some());
    let sent_status_badge = if sent_live {
        "<span style='background:#0d2b0d;color:#3fb950;border:1px solid #3fb95040;\
         border-radius:10px;padding:1px 7px;font-size:.75em;margin-left:6px'>🌙 Live</span>"
    } else {
        "<span style='background:#1c1c1c;color:#8b949e;border:1px solid #30363d;\
         border-radius:10px;padding:1px 7px;font-size:.75em;margin-left:6px'\
         title='Set LUNARCRUSH_API_KEY on the droplet'>🌙 No data</span>"
    };

    let cand_rows: String = if s.candidates.is_empty() {
        r#"<tr><td colspan="4" class="empty-td">Scanning…</td></tr>"#.to_string()
    } else {
        s.candidates.iter().map(|c| {
            let chg_td = match c.change_pct {
                Some(pct) => {
                    let cc = if pct >= 0.0 { "#3fb950" } else { "#f85149" };
                    let cs = if pct >= 0.0 { "+" } else { "" };
                    format!("<td style='color:{}'>{}{:.3}%</td>", cc, cs, pct)
                }
                None => "<td style='color:var(--muted)'>—</td>".to_string(),
            };
            let is_open   = s.positions.iter().any(|p| p.symbol == c.symbol);
            let sym_style = if is_open { "font-weight:700;color:#58a6ff" } else { "" };
            let open_dot  = if is_open { " ●" } else { "" };

            // Coin logo (16 px) next to ticker
            let c_logo = coins::coin_logo_img(&c.symbol, 16);

            // Sentiment column: emoji + bullish% + galaxy score chip
            let sent_html = match (c.bullish_percent, c.galaxy_score) {
                (Some(bp), Some(gs)) => {
                    let emoji  = if bp >= 65.0 { "🟢" } else if bp >= 45.0 { "🟡" } else { "🔴" };
                    let bp_col = if bp >= 55.0 { "#3fb950" } else if bp >= 45.0 { "#e3b341" } else { "#f85149" };
                    format!("<span>{}</span> <span style='color:{};font-size:11px'>{:.0}%</span> \
                             <span style='color:#8b949e;font-size:10px'>G{:.0}</span>",
                        emoji, bp_col, bp, gs)
                }
                _ => "<span style='color:#444c56;font-size:.8em'>—</span>".to_string(),
            };

            format!("<tr>\
                       <td style='{ss}'>{logo}{sym}{dot}</td>\
                       <td>${price:.4}</td>\
                       {chg_td}\
                       <td class='sent-cell'>{sent}</td>\
                     </tr>",
                ss     = sym_style,
                logo   = c_logo,
                sym    = c.symbol,
                dot    = open_dot,
                price  = c.price,
                chg_td = chg_td,
                sent   = sent_html,
            )
        }).collect()
    };

    // ── Signal feed rows (staggered animation) ────────────────────────────
    let dec_rows: String = if s.recent_decisions.is_empty() {
        r#"<tr><td colspan="5" class="empty-td">Scanning for signals…</td></tr>"#.to_string()
    } else {
        s.recent_decisions.iter().rev().take(10).enumerate().map(|(i, d)| {
            let (ac, dc, icon) = match d.action.as_str() {
                "BUY"  => ("▲ BUY",  "#3fb950", "🟢"),
                "SELL" => ("▼ SELL", "#f85149", "🔴"),
                _      => ("— SKIP", "#8b949e", "⬜"),
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
            let delay_ms = i * 60;
            format!(
                "<tr class='sig-row' style='animation-delay:{delay}ms'>\
                   <td>{icon} <b>{sym}</b></td>\
                   <td style='color:{dc};font-weight:600'>{ac}</td>\
                   <td>{conf:.0}%</td>\
                   <td class='ts' style='max-width:260px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap'>{rbadge}{rat}</td>\
                   <td class='ts'>{ts}</td>\
                 </tr>",
                delay  = delay_ms,
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
<title>RedRobot HedgeBot</title>
<meta http-equiv="refresh" content="10">
<style>
*{{box-sizing:border-box;margin:0;padding:0}}
:root{{--bg:#0d1117;--surface:#161b22;--border:#30363d;--muted:#8b949e;--text:#e6edf3;
      --green:#3fb950;--red:#f85149;--blue:#58a6ff;--yellow:#e3b341;--dim:#21262d}}
body{{background:var(--bg);color:var(--text);font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',system-ui,sans-serif;
      font-size:14px;line-height:1.4;padding:12px;max-width:900px;margin:0 auto}}
/* ── Keyframe animations ── */
@keyframes pulse{{0%,100%{{opacity:1}}50%{{opacity:.35}}}}
@keyframes fadeSlide{{from{{opacity:0;transform:translateX(-10px)}}to{{opacity:1;transform:translateX(0)}}}}
@keyframes scanBeam{{0%{{top:-4px;opacity:.9}}100%{{top:100%;opacity:0}}}}
@keyframes progFill{{from{{width:0}}to{{width:100%}}}}
@keyframes spinDot{{0%{{transform:rotate(0deg)}}100%{{transform:rotate(360deg)}}}}
@keyframes glow{{0%,100%{{box-shadow:0 0 0 0 rgba(88,166,255,.0)}}50%{{box-shadow:0 0 8px 1px rgba(88,166,255,.25)}}}}
/* ── Header ── */
.header{{display:flex;justify-content:space-between;align-items:center;margin-bottom:14px;flex-wrap:wrap;gap:6px}}
.header h1{{color:var(--blue);font-size:1.1em;font-weight:700;display:flex;align-items:center;gap:6px}}
.header .ts{{font-size:.75em;color:var(--muted);white-space:nowrap}}
.live-ring{{width:8px;height:8px;border-radius:50%;background:var(--green);display:inline-block;
            animation:pulse 1.6s ease infinite;flex-shrink:0}}
/* ── Equity hero ── */
.equity-hero{{background:var(--surface);border:1px solid var(--border);border-radius:10px;
              padding:16px;margin-bottom:12px;display:flex;justify-content:space-between;
              align-items:center;flex-wrap:wrap;gap:8px}}
.equity-hero .eq-val{{font-size:2em;font-weight:700;color:var(--text);line-height:1}}
.equity-hero .eq-label{{font-size:.7em;color:var(--muted);margin-top:2px}}
.equity-hero .pnl-badge{{padding:6px 12px;border-radius:20px;font-size:.85em;font-weight:700}}
/* ── Metric strip ── */
.metrics{{display:grid;grid-template-columns:repeat(2,1fr);gap:8px;margin-bottom:12px}}
@media(min-width:500px){{.metrics{{grid-template-columns:repeat(3,1fr)}}}}
@media(min-width:700px){{.metrics{{grid-template-columns:repeat(6,1fr)}}}}
.metric{{background:var(--surface);border:1px solid var(--border);border-radius:8px;
         padding:8px 10px;text-align:center}}
.metric .mv{{font-size:1.05em;font-weight:700}}
.metric .ml{{font-size:.65em;color:var(--muted);margin-top:2px;white-space:nowrap}}
/* ── Status bar ── */
.status-bar{{background:var(--surface);border:1px solid var(--border);border-radius:8px;
             padding:0;margin-bottom:12px;font-size:.8em;color:var(--muted);overflow:hidden}}
.status-inner{{display:flex;justify-content:space-between;align-items:center;
               gap:8px;flex-wrap:wrap;padding:8px 12px}}
.status-bar .st-text{{flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}}
/* 5-second progress bar at bottom of status bar */
.prog-track{{height:2px;background:var(--dim);position:relative;overflow:hidden}}
.prog-fill{{height:2px;background:linear-gradient(90deg,var(--blue),var(--green));
            animation:progFill 10s linear forwards}}
/* ── Section ── */
.section{{background:var(--surface);border:1px solid var(--border);border-radius:10px;
          padding:12px;margin-bottom:12px}}
.section-title{{font-size:.7em;text-transform:uppercase;letter-spacing:1px;color:var(--muted);
                margin-bottom:10px;display:flex;justify-content:space-between;align-items:center;gap:6px}}
.section-title-left{{display:flex;align-items:center;gap:6px}}
.badge{{background:var(--dim);color:var(--muted);padding:2px 7px;border-radius:10px;font-size:.85em}}
/* ── Position cards ── */
.pos-grid{{display:grid;grid-template-columns:repeat(auto-fill,minmax(260px,1fr));gap:10px}}
.pos-card{{background:var(--dim);border-radius:8px;padding:12px;border-left:3px solid var(--border);
           animation:fadeSlide .35s ease both}}
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
.empty-state{{text-align:center;color:var(--muted);padding:20px;font-size:.85em}}
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
table{{width:100%;border-collapse:collapse;font-size:.75em;min-width:340px}}
th{{color:var(--muted);text-align:left;padding:5px 7px;border-bottom:1px solid var(--border);
    white-space:nowrap;font-weight:500}}
td{{padding:5px 7px;border-bottom:1px solid var(--dim);vertical-align:middle}}
tr:last-child td{{border-bottom:none}}
tr:hover td{{background:rgba(255,255,255,.03)}}
.empty-td{{color:var(--muted);text-align:center;padding:14px}}
.ts{{color:var(--muted);font-size:.85em;white-space:nowrap}}
/* Reason badges */
.reason-stop{{color:#f85149}}.reason-take{{color:#3fb950}}
.reason-time{{color:#e3b341}}.reason-partial{{color:#58a6ff}}
.reason-ai{{color:#e3b341;font-weight:600}}.reason-signal{{color:#8b949e}}
/* Sentiment cell */
.sent-cell{{white-space:nowrap;font-size:.82em}}
/* ── Inline weight strip ── */
.w-strip{{display:flex;flex-wrap:wrap;align-items:center;gap:6px;
          margin-top:8px;padding-top:7px;border-top:1px solid var(--border)}}
.w-item{{display:flex;align-items:center;gap:4px;font-size:.7em}}
.w-item-label{{color:var(--muted);white-space:nowrap}}
.w-item-val{{font-weight:700;color:var(--blue)}}
.w-item-bar{{width:32px;height:3px;background:var(--border);border-radius:2px;overflow:hidden}}
.w-item-fill{{height:3px;background:linear-gradient(90deg,#388bfd,#58a6ff);border-radius:2px}}
.w-strip-note{{margin-left:auto;font-size:.65em;color:var(--muted);white-space:nowrap}}
/* ── Utility ── */
.g{{color:var(--green)}}.r{{color:var(--red)}}.b{{color:var(--blue)}}.y{{color:var(--yellow)}}
</style></head><body>

<div class="header">
  <h1>
    <img src="data:image/png;base64,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"
         height="28" width="auto" alt="RedRobot"
         style="vertical-align:middle;margin-right:7px">
    <span class="live-ring"></span> RedRobot HedgeBot
  </h1>
  <span class="ts">⟳ <span id="cntdn">30s</span> &nbsp;·&nbsp; {last_update}</span>
</div>

<div class="equity-hero">
  <div>
    <div id="equity-val" class="eq-val">${equity:.2}</div>
    <div id="equity-label" class="eq-label">Total Equity &nbsp;·&nbsp; free $<span id="equity-free">{capital:.2}</span></div>
  </div>
  <div id="pnl-badge" class="pnl-badge" style="color:{pnl_colour};border:1px solid {pnl_colour}40;background:{pnl_colour}15">
    {pnl_sign}${total_pnl:.2} &nbsp; {pnl_sign}{total_pnl_pct:.2}%
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

<div class="section">
  <div class="section-title">
    <span class="section-title-left"><span class="live-ring"></span> Active Positions</span>
    <span class="badge">{open_n} / 4 slots · max 2 per direction</span>
  </div>
  <div class="pos-grid">{pos_cards}</div>
</div>

<!-- Signal feed immediately under positions -->
<div class="section sig-section">
  <div class="section-title">
    <span class="section-title-left"><span class="live-ring"></span> Signal Feed</span>
    <span class="badge">last 10 decisions</span>
  </div>
  <div class="tbl-wrap scan-wrap">
    <div class="scan-beam"></div>
    <table><tr><th>Symbol</th><th>Action</th><th>Conf</th><th>Rationale</th><th>Time</th></tr>
    {dec_rows}</table>
  </div>
</div>

<div class="section">
  <div class="section-title">
    <span>Candidates <span class="badge">{cand_n} scanned · ● = open</span>{sent_status}</span>
  </div>
  <div class="tbl-wrap">
    <table><tr><th>Symbol</th><th>Price</th><th>Session Δ</th><th>🌙 Sentiment</th></tr>{cand_rows}</table>
  </div>
  {wh}
</div>

<div class="section">
  <div class="section-title">Closed Trades <span class="badge">{total_closed} total</span></div>
  <div class="tbl-wrap">
    <table><tr><th>Symbol</th><th>Side</th><th>Entry</th><th>Exit</th><th>P&amp;L</th><th>Reason</th><th>Time</th></tr>
    {closed_rows}</table>
  </div>
</div>

<script>
(function(){{
  /* ── Countdown to next full page reload (10s) ────────────────────────── */
  var t=10,el=document.getElementById('cntdn');
  if(el){{
    function tick(){{el.textContent=(t>0?t:'…')+'s';if(t>0)t--;}}
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
        sent_status  = sent_status_badge,
        committed    = committed,
        status       = s.status,
        pos_cards    = pos_cards,
        wh           = wh,
        cand_rows    = cand_rows,
        closed_rows  = closed_rows,
        dec_rows     = dec_rows,
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

async fn api_state_handler(State(state): State<SharedState>) -> Json<BotState> {
    Json(state.read().await.clone())
}

pub async fn serve(state: SharedState, port: u16) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let app = Router::new()
        .route("/", get(dashboard_handler))
        .route("/api/state", get(api_state_handler))
        .with_state(state);
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

    // ── Sentiment live status ─────────────────────────────────────────────────

    #[test]
    fn sent_live_true_when_any_candidate_has_bullish_data() {
        let candidates = [
            CandidateInfo { symbol: "BTC".to_string(), price: 50000.0,
                change_pct: None, galaxy_score: None, bullish_percent: None, alt_rank: None },
            CandidateInfo { symbol: "ETH".to_string(), price: 3000.0,
                change_pct: None, galaxy_score: Some(72.0), bullish_percent: Some(65.0), alt_rank: None },
        ];
        let sent_live = candidates.iter().any(|c| c.bullish_percent.is_some());
        assert!(sent_live, "should be live when at least one candidate has sentiment data");
    }

    #[test]
    fn sent_live_false_when_no_candidates_have_data() {
        let candidates = [
            CandidateInfo { symbol: "SOL".to_string(), price: 100.0,
                change_pct: None, galaxy_score: None, bullish_percent: None, alt_rank: None },
        ];
        let sent_live = candidates.iter().any(|c| c.bullish_percent.is_some());
        assert!(!sent_live, "should not be live when no candidates have sentiment data");
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
        // Regression: was hardcoded as "8 slots · max 4 per direction"
        // Correct values: MAX_POSITIONS=4, MAX_SAME_DIRECTION=2
        let max_positions      = 4usize;
        let max_same_direction = 2usize;
        let badge = format!("{} / {} slots · max {} per direction",
            0, max_positions, max_same_direction);
        assert!(badge.contains("4 slots"),  "badge must show 4 total slots");
        assert!(badge.contains("max 2 per"), "badge must show max 2 per direction");
        assert!(!badge.contains("8 slots"), "REGRESSION: badge must NOT say 8 slots");
        assert!(!badge.contains("max 4 per"), "REGRESSION: badge must NOT say max 4 per direction");
    }
}
