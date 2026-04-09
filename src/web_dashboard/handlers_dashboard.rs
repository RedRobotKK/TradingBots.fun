//! `handlers_dashboard` — part of the `web_dashboard` module tree.
//!
//! Shared types and helpers available via `use super::*;`.
#![allow(unused_imports)]

use super::*;

pub(crate) async fn dashboard_handler(State(app): State<AppState>) -> Html<String> {
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
        format!("0.50× sizes · 7d DD {:.1}%", rolling_dd_pct)
    } else {
        format!("Risk Normal · 7d DD {:.1}%", rolling_dd_pct)
    };
    let pf_str = if m.profit_factor.is_infinite() {
        "∞".to_string()
    } else {
        format!("{:.2}", m.profit_factor)
    };

    // ── Portfolio heat and avg open-R ─────────────────────────────────────
    // heat_pct: what fraction of total equity is currently locked as margin.
    let heat_pct = if equity > 0.0 {
        (committed / equity * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };
    // avg_open_r: mean R-multiple across all live positions.
    //   > 0  → book is net profitable on open risk
    //   < 0  → book is net underwater
    let (avg_open_r, avg_open_r_str) = if s.positions.is_empty() {
        (0.0f64, "—".to_string())
    } else {
        let avg = s
            .positions
            .iter()
            .map(|p| {
                if p.r_dollars_risked > 1e-8 {
                    p.unrealised_pnl / p.r_dollars_risked
                } else {
                    0.0
                }
            })
            .sum::<f64>()
            / s.positions.len() as f64;
        let sign = if avg >= 0.0 { "+" } else { "" };
        (avg, format!("{sign}{avg:.2}R"))
    };
    let avg_r_colour = if avg_open_r > 0.5 {
        "#3fb950"
    } else if avg_open_r >= 0.0 {
        "#8b949e"
    } else {
        "#f85149"
    };
    // slots_colour: traffic-light for how full the position book is.
    let slots_colour = if s.positions.len() >= 18 {
        "#f85149" // nearly full — no room for new entries
    } else if s.positions.len() >= 12 {
        "#e3b341" // moderately loaded
    } else {
        "#3fb950" // plenty of capacity
    };

    // ── Macro regime pill colours ─────────────────────────────────────────
    let (macro_label, macro_bg, macro_fg, macro_border, macro_dot) =
        match s.macro_regime.as_str() {
            "BULL" => ("🐂 BULL", "#0d2318", "#3fb950", "#238636", "#3fb950"),
            "BEAR" => ("🐻 BEAR", "#2d0f0d", "#f85149", "#da3633", "#f85149"),
            _      => ("◎ NEUTRAL", "#1c1c1c", "#8b949e", "#30363d", "#8b949e"),
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

    // ── Wallet display for equity hero ────────────────────────────────────
    // Single wallet → show truncated address linking to admin users.
    // Multiple wallets → show aggregate count with link to admin users.
    let (wallet_label, wallet_href) = {
        let tenants = app.tenants.read().await;
        let all: Vec<_> = tenants.all().collect();
        // Filter to tenants that actually have an HL wallet set up
        let with_wallet: Vec<_> = all
            .iter()
            .filter(|h| h.config.hl_wallet_address.is_some())
            .collect();
        let n = with_wallet.len();
        let label = if n == 0 {
            "No wallet connected".to_string()
        } else if n == 1 {
            with_wallet[0]
                .config
                .hl_wallet_address
                .as_deref()
                .map(|w| {
                    let len = w.len();
                    format!("{}…{}", &w[..6.min(len)], &w[len.saturating_sub(4)..])
                })
                .unwrap_or_else(|| "—".to_string())
        } else {
            format!("{n} wallets (aggregate)")
        };
        (label, "/admin/users")
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

    // Tooltip explaining which scaled CB threshold tier applies to this account.
    // Mirrors the account-size tiers in execute_paper_trade / metrics.rs.
    let cb_threshold_info = if s.initial_capital <= 25.0 {
        "CB threshold: fires at 20% DD, resets at 12% (small account — scaled up from 8%)"
    } else if s.initial_capital <= 100.0 {
        "CB threshold: fires at 15% DD, resets at 9% (small account — scaled up from 8%)"
    } else if s.initial_capital <= 500.0 {
        "CB threshold: fires at 12% DD, resets at 7% (mid account — scaled up from 8%)"
    } else {
        "CB threshold: fires at 8% DD, resets at 5% (standard)"
    };

    // ── Position cards ────────────────────────────────────────────────────
    let pos_cards: String = if s.positions.is_empty() {
        r#"<div class="empty-state"><div class="radar"></div><p>No open positions — scanning for signals…</p></div>"#.to_string()
    } else {
        s.positions.iter().enumerate().map(|(pos_idx, p)| {
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

            format!(r#"<div class="pos-flip-wrap" id="pf-{sym_id}" data-pnl="{raw_pnl:.4}" data-idx="{pos_idx}"><div class="pos-flip-inner">
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
     style="display:block;max-width:100%;overflow:hidden;opacity:0.4">
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
            // Guard: when only one point exists (n-1)==0, spread it across full width.
            let x_scale = if h.len() > 1 { w_px / (n - 1.0) } else { 0.0 };
            let pts: String = h
                .iter()
                .enumerate()
                .map(|(i, &v)| {
                    let x = i as f64 * x_scale;
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
     style="display:block;max-width:100%;overflow:hidden">
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
      return['#f85149','🔴 Circuit Breaker Active — all new position sizes are scaled to 0.50× until equity recovers. This is the self-protection mechanism working exactly as designed.'];
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
    notes:['🟢 Normal mode: full Kelly × Sharpe multiplier × confidence = normal position sizes.','🔴 CB Active: ALL new position sizes × 0.50 and confidence floor raised +10%.','Auto-resets when rolling equity recovers to within 8% of the 7-day peak.','This is a hard, automatic rule — not a discretionary override.','The 7-day window prevents a single good week from permanently masking a losing streak.'],
    verdict:function(v){
      if(v>0)return['#f85149','🔴 Circuit Breaker is active. The 7-day rolling drawdown has exceeded 8%. All new position sizes are automatically 0.50× of normal and the minimum confidence required to open a trade is raised by 10 percentage points. This continues automatically until equity recovers.'];
      return['#3fb950','🟢 Normal operating mode. The 7-day equity window shows no significant drawdown from its peak. Full Kelly-based position sizing is in effect across all signals.'];
    }
  },
  openClosed:{
    name:'Position Slots Used',
    fmt:function(v){return String(v)+' / 20';},
    no_gauge:true,
    formula:'Open positions ÷ max concurrent positions (hard cap = 20)',
    notes:['Hard cap is 20 — lowered from 25 after live data showed 40 open positions averaging only 0.12R. Fewer, higher-conviction entries outperform a full book of drifting trades.','🟢 Green (< 12): plenty of capacity. The bot can take new signals freely.','🟡 Yellow (12–17): moderately loaded. New entries are still allowed but the Kelly budget is tightening.','🔴 Red (18–20): near full. Only very high-conviction signals will get through the Kelly + heat filters.','When full, the bot continues managing existing positions (trailing stops, tranches, DCA) but will not open new entries.'],
    verdict:function(v){
      if(v>=18)return['#f85149','🔴 Near capacity ('+v+'/20). The bot is managing a full book — no new entries until positions close. All open positions are still being actively managed.'];
      if(v>=12)return['#e3b341','🟡 Moderately loaded ('+v+'/20). New high-conviction entries are still possible. Monitor portfolio heat and Kelly to ensure adequate capital per position.'];
      return['#3fb950','🟢 Good capacity ('+v+'/20). The bot has room to take new entries as signals appear. Position sizing is determined by Kelly × Sharpe × confidence, not the slot count.'];
    }
  },
  avgR:{
    name:'Average Open R-Multiple',
    fmt:function(v){return v===0?'—':(v>=0?'+':'')+parseFloat(v).toFixed(2)+'R';},
    no_gauge:true,
    formula:'Mean(unrealised_pnl ÷ r_dollars_risked) across all open positions',
    notes:['R-multiple = unrealised P&L ÷ initial dollars risked on this trade. 1R = made back your original risk. -1R = at the stop-loss.','Positive avg R = the open book is net profitable on risk taken. Negative = net underwater.','This updates every poll cycle as prices move — a dip to -0.5R avg is normal in trending markets.','Individual positions showing -0.9R to -1.1R are near/at their stop and will close automatically.','Use this alongside Portfolio Heat to gauge risk quality: high heat + negative avg R = meaningful exposure to loss.'],
    verdict:function(v){
      if(v>1.0)return['#3fb950','🟢 Excellent open book. Average position is more than 1R in profit — that is strong performance. Trail stops to protect these gains.'];
      if(v>0.3)return['#3fb950','🟢 Positive open book. Average position is in profit. The bot is working as expected.'];
      if(v>-0.3)return['#8b949e','⚯ Near break-even across open positions. Normal for early-stage positions or ranging markets.'];
      if(v>-0.7)return['#e3b341','🟡 Open book underwater. Positions are average -0.3R to -0.7R. Not alarming — stops protect against further loss — but watch for DCA opportunities.'];
      return['#f85149','🔴 Open book significantly underwater (avg < -0.7R). Positions are near their stops. The circuit breaker drawdown metric will reflect this if equity drops materially.'];
    }
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
/* ── Live Wins Stock-Market Ticker ── */
.win-ticker{{display:flex;align-items:stretch;overflow:hidden;
             background:#0d1117;border:1px solid rgba(63,185,80,.30);
             border-radius:6px;height:36px;margin-bottom:12px;
             font-family:'SF Mono','Fira Code','Courier New',monospace;
             font-size:.72em;font-weight:600;position:relative;
             box-shadow:0 0 0 0 rgba(63,185,80,0);transition:box-shadow .4s ease}}
.win-ticker.wt-flash{{box-shadow:0 0 20px rgba(63,185,80,.55)!important}}
.win-ticker-label{{flex-shrink:0;padding:0 14px;color:#fff;letter-spacing:.8px;
                   font-size:.9em;font-weight:700;display:flex;align-items:center;gap:7px;
                   background:linear-gradient(135deg,#1a7f37 0%,#238636 100%);
                   border-right:2px solid rgba(63,185,80,.40);white-space:nowrap;
                   text-transform:uppercase}}
.wt-dot{{width:7px;height:7px;border-radius:50%;background:#fff;flex-shrink:0;
          animation:wt-pulse 1.4s ease-in-out infinite}}
@keyframes wt-pulse{{0%,100%{{opacity:1;transform:scale(1)}}50%{{opacity:.3;transform:scale(.75)}}}}
.win-ticker-scroll{{flex:1;overflow:hidden;position:relative;display:flex;align-items:center}}
.win-ticker-inner{{display:inline-flex;align-items:center;white-space:nowrap;
                   animation:wt-scroll 40s linear infinite;will-change:transform}}
.win-ticker-inner:hover{{animation-play-state:paused;cursor:default}}
@keyframes wt-scroll{{from{{transform:translateX(0)}}to{{transform:translateX(-50%)}}}}
.wt-item{{display:inline-flex;align-items:center;gap:5px;padding:0 6px 0 0}}
.wt-sep{{color:#30363d;padding:0 10px;font-size:1.2em;user-select:none}}
.wt-sym{{color:#58a6ff;font-weight:800;letter-spacing:.5px}}
.wt-long{{color:#3fb950}}.wt-short{{color:#f85149}}
.wt-pnl-pos{{color:#3fb950;font-weight:700}}.wt-pnl-neg{{color:#f85149;font-weight:700}}
.wt-r{{color:#e3b341;font-size:.9em}}
.wt-wlt{{color:#6e7681;font-size:.86em;font-weight:400}}
.wt-wallet-label{{color:#484f58;font-size:.82em;font-weight:400}}
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
.equity-hero .eq-left{{display:flex;flex-direction:column;gap:0;width:100%}}
.eq-top-row{{display:flex;justify-content:space-between;align-items:flex-start;gap:12px;min-width:0;overflow:hidden}}
.eq-eyebrow{{font-size:.60em;color:var(--muted);letter-spacing:.9px;text-transform:uppercase;
             margin-bottom:4px;font-weight:500}}
.equity-hero .eq-val{{font-size:2.1em;font-weight:800;line-height:1;letter-spacing:-.02em;
                       background:linear-gradient(135deg,#e6edf3 30%,#58a6ff);
                       -webkit-background-clip:text;-webkit-text-fill-color:transparent;background-clip:text}}
.equity-hero .pnl-badge{{padding:6px 13px;border-radius:22px;font-size:.82em;font-weight:700;
                          letter-spacing:.2px}}
.eq-breakdown{{display:flex;flex-direction:column;gap:4px;margin-top:13px;
               border-top:1px solid rgba(255,255,255,.07);padding-top:11px}}
.eq-row{{display:flex;align-items:center;gap:8px;font-size:.74em;padding:2px 0}}
.eq-row-icon{{font-size:.95em;line-height:1;width:18px;text-align:center;flex-shrink:0}}
.eq-row-label{{color:#8b949e;flex:1;letter-spacing:.1px}}
.eq-row-val{{font-weight:700;color:#e6edf3;font-variant-numeric:tabular-nums;
             font-family:ui-monospace,monospace}}
.eq-wallet-row{{display:flex;align-items:center;gap:7px;margin-top:12px;
                padding:6px 10px;border-radius:8px;border:1px solid rgba(255,255,255,.07);
                background:rgba(255,255,255,.02);text-decoration:none;color:#8b949e;
                font-size:.70em;letter-spacing:.2px;transition:border-color .15s,color .15s;
                font-family:ui-monospace,monospace}}
.eq-wallet-row:hover{{border-color:rgba(88,166,255,.35);color:#58a6ff}}
.eq-wallet-addr{{flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}}
.eq-right{{display:flex;align-items:flex-start;padding-top:4px;min-width:0;
           max-width:min(320px,45vw);overflow:hidden;flex-shrink:1}}
/* Sparkline SVG scales down on narrow viewports */
.eq-right svg{{max-width:100%;height:auto}}
/* ── Metric strip ── */
.metrics{{display:grid;grid-template-columns:repeat(2,1fr);gap:8px;margin-bottom:12px}}
@media(min-width:500px){{.metrics{{grid-template-columns:repeat(3,1fr)}}}}
@media(min-width:700px){{.metrics{{grid-template-columns:repeat(6,1fr)}}}}
/* ── Mobile fixes ── */
@media(max-width:480px){{
  /* Stack hero: sparkline goes full-width below the equity number */
  .eq-top-row{{flex-direction:column;gap:8px}}
  .eq-right{{max-width:100%;width:100%}}
  .eq-right svg{{width:100%!important}}
  /* Position flip-back chart: ensure canvas never overflows card */
  .pos-flip-back{{overflow:hidden}}
  .pos-flip-back canvas,.pos-flip-back>div{{max-width:100%!important;overflow:hidden}}
  /* Stat bar: allow wrapping on very narrow screens */
  .stat-bar{{flex-wrap:wrap}}
  .stat-cell{{min-width:calc(50% - 1px)}}
}}
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
    <!-- ── Macro regime indicator ── -->
    <span id="macro-pill" style="
      display:inline-flex;align-items:center;gap:5px;
      font-size:.72rem;font-weight:700;letter-spacing:.5px;
      padding:4px 10px;border-radius:20px;
      background:{macro_bg};color:{macro_fg};border:1px solid {macro_border};
      font-family:'SF Mono','Courier New',monospace;
    " title="Daily BTC+ETH MA5/MA10/MA20 consensus macro regime">
      <span style="width:7px;height:7px;border-radius:50%;background:{macro_dot};display:inline-block"></span>
      {macro_label}
    </span>
    <a href="/fleet" style="font-size:.78rem;color:#8b949e;text-decoration:none;padding:5px 10px;border:1px solid #21262d;border-radius:6px;display:inline-flex;align-items:center;gap:5px;transition:.15s" onmouseover="this.style.color='#e6edf3';this.style.borderColor='#30363d'" onmouseout="this.style.color='#8b949e';this.style.borderColor='#21262d'">⚡ Fleet (5 000)</a>
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

<!-- ── Live Wins Stock-Market Ticker ─────────────────────────────────────── -->
<div style="display:flex;align-items:center;gap:8px;margin-bottom:4px">
  <span style="font-size:.68em;color:#6e7681;font-weight:600;letter-spacing:.3px">SESSION</span>
  <span id="wt-ls-stats" style="font-size:.68em;font-family:'SF Mono','Courier New',monospace"></span>
</div>
<div class="win-ticker" id="win-ticker">
  <div class="win-ticker-label">
    <span class="wt-dot"></span>LIVE WINS
  </div>
  <div class="win-ticker-scroll">
    <div class="win-ticker-inner" id="win-inner">
      <span style="color:#484f58;padding:0 24px">Waiting for winning trades…</span>
    </div>
  </div>
</div>

<div id="equity-hero" class="{hero_class}">
  <div class="eq-left">

    <!-- Title + big number -->
    <div class="eq-top-row">
      <div>
        <div class="eq-eyebrow">Total Equity</div>
        <div id="equity-val" class="eq-val">${equity:.2}</div>
      </div>
      <div class="eq-right">
        {sparkline_svg}
      </div>
    </div>

    <!-- P&L badge -->
    <div id="pnl-badge" class="pnl-badge" style="color:{pnl_colour};border:1px solid {pnl_colour}40;background:{pnl_colour}15;margin-top:8px;display:inline-flex;align-items:center;gap:8px">
      <span>{pnl_sign}${total_pnl:.2}</span>
      <span style="opacity:.6">|</span>
      <span>{pnl_sign}{total_pnl_pct:.2}% all-time</span>
    </div>

    <!-- Three-row capital breakdown -->
    <div class="eq-breakdown">
      <div class="eq-row" title="Cash sitting idle — not in any open trade. This is what you can withdraw right now without closing anything.">
        <span class="eq-row-icon">💵</span>
        <span class="eq-row-label">Withdrawable</span>
        <span class="eq-row-val" id="equity-free">${capital:.2}</span>
      </div>
      <div class="eq-row" title="Margin locked in open positions plus unrealised P&L. This becomes withdrawable when positions close.">
        <span class="eq-row-icon">📊</span>
        <span class="eq-row-label">In Trades</span>
        <span class="eq-row-val" id="equity-in-trades">${in_trades:.2}</span>
      </div>
      <div class="eq-row" style="color:{pool_col_op}" title="Profits accumulated from closed trades. The bot uses this pool first for new entries — so it risks the market's money before your original capital.">
        <span class="eq-row-icon">🏦</span>
        <span class="eq-row-label">Profit Pool</span>
        <span class="eq-row-val" id="op-pool">${pool_bal:.2}</span>
      </div>
    </div>

    <!-- Wallet row -->
    <a href="{wallet_href}" class="eq-wallet-row" title="View all wallets in admin panel">
      <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor"
           stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" style="opacity:.5;flex-shrink:0">
        <rect x="1" y="4" width="14" height="10" rx="2"/>
        <path d="M1 7h14"/><circle cx="11.5" cy="11" r="1" fill="currentColor" stroke="none"/>
      </svg>
      <span class="eq-wallet-addr">{wallet_label}</span>
      <svg width="10" height="10" viewBox="0 0 16 16" fill="none" stroke="currentColor"
           stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" style="opacity:.35;flex-shrink:0;margin-left:auto">
        <path d="M6 3l5 5-5 5"/>
      </svg>
    </a>

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
  <div class="metric{cbcc}" onclick="showMetric('cb',{cb_int})" title="{cb_threshold_info}">
    <div class="mv" style="color:{cbc}">{cb_label}</div>
    <div class="ml">{cb_desc} <span class="ml-hint">tap to explain</span></div></div>
  <div class="metric" onclick="showMetric('openClosed',{open_n})">
    <div class="mv" style="color:{slots_colour}">{open_n}<span style="font-size:.65em;color:var(--muted)"> / {pos_cap}</span></div>
    <div class="ml">Slots Used <span class="ml-hint">tap to explain</span></div></div>
  <div class="metric" onclick="showMetric('avgR',0)" style="cursor:pointer">
    <div class="mv" style="color:{avg_r_colour}">{avg_open_r_str}</div>
    <div class="ml">Avg Open R <span class="ml-hint">tap to explain</span></div></div>
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
      {open_n}/{pos_cap} slots · {avg_open_r_str} avg R · {heat_pct:.0}% heat · Sharpe {sharpe:.2}
    </span>
  </div>
  <div class="prog-track"><div class="prog-fill"></div></div>
</div>
{ai_status_html}

<div class="section section-positions">
  <div class="section-title">
    <span class="section-title-left"><span class="live-ring"></span> Active Positions</span>
    <span style="display:flex;align-items:center;gap:8px">
      <button id="sort-pos-btn" onclick="sortPositions()" title="Cycle: Best first → Worst first → Split W/L"
        style="background:#1c2026;border:1px solid #444c56;color:#cdd9e5;border-radius:6px;
               padding:2px 10px;font-size:.75em;cursor:pointer;line-height:1.6">
        ↕ Sort P&amp;L
      </button>
      <span id="wl-badge" class="badge" style="font-size:.72em"></span>
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
/* ── Live Wins — Bloomberg-style continuous stock ticker (SSE) ────────── */
(function(){{
  var MAX_ITEMS = 60;
  var PX_PER_SEC = 80;   // scroll speed pixels/second
  var wins = [];
  var longWins = 0, shortWins = 0;

  function fmtSym(s){{ return s.replace('-PERP','').replace('USDT',''); }}

  function fmtItem(w, idx){{
    var isLong = w.side === 'LONG';
    var arrow  = isLong ? '▲' : '▼';
    var sideC  = isLong ? 'wt-long' : 'wt-short';
    var pnlC   = w.pnl >= 0 ? 'wt-pnl-pos' : 'wt-pnl-neg';
    var pnlStr = (w.pnl >= 0 ? '+$' : '-$') + Math.abs(w.pnl).toFixed(2);
    var rStr   = w.r_mult > 0 ? '+' + w.r_mult.toFixed(2) + 'R' : '';
    var sym    = fmtSym(w.symbol);
    return '<span class="wt-item">'
      + '<span class="wt-sep">◆</span>'
      + '<span class="wt-sym">' + sym + '</span>'
      + '<span class="' + sideC + '">&nbsp;' + arrow + '&nbsp;</span>'
      + '<span class="' + pnlC + '">' + pnlStr + '</span>'
      + (rStr ? '&nbsp;<span class="wt-r">' + rStr + '</span>' : '')
      + '&nbsp;<span class="wt-wlt">·&nbsp;' + w.wallet + '</span>'
      + '</span>';
  }}

  function renderTicker(){{
    var inner = document.getElementById('win-inner');
    if(!inner) return;
    if(wins.length === 0){{
      inner.style.animation = 'none';
      inner.innerHTML = '<span style="color:#484f58;padding:0 24px">Waiting for winning trades…</span>';
      return;
    }}
    // Duplicate content for seamless infinite loop
    var html = wins.map(fmtItem).join('');
    inner.innerHTML = html + html;
    // Speed: base on content width, min 8s
    inner.style.animation = 'none';
    inner.offsetWidth; // force reflow
    var halfW = inner.scrollWidth / 2;
    var dur   = Math.max(8, halfW / PX_PER_SEC);
    inner.style.animation = 'wt-scroll ' + dur.toFixed(1) + 's linear infinite';
  }}

  function updateLSStats(){{
    var el = document.getElementById('wt-ls-stats');
    if(!el) return;
    var total = longWins + shortWins;
    if(total === 0){{ el.textContent = ''; return; }}
    var lp = ((longWins/total)*100).toFixed(0);
    var sp = ((shortWins/total)*100).toFixed(0);
    el.innerHTML = '<span style="color:#3fb950">▲ ' + longWins + ' (' + lp + '%)</span>'
      + '<span style="color:#6e7681"> · </span>'
      + '<span style="color:#f85149">▼ ' + shortWins + ' (' + sp + '%)</span>';
  }}

  function addWin(w){{
    wins.unshift(w);
    if(wins.length > MAX_ITEMS) wins.pop();
    if(w.side === 'LONG') longWins++; else shortWins++;
    renderTicker();
    updateLSStats();
    // Glow flash
    var el = document.getElementById('win-ticker');
    if(el){{
      el.classList.add('wt-flash');
      setTimeout(function(){{ el.classList.remove('wt-flash'); }}, 900);
    }}
  }}

  function connectStream(){{
    if(!window.EventSource) return;
    var es = new EventSource('/api/trade-stream');
    es.addEventListener('trade_win', function(e){{
      try{{ addWin(JSON.parse(e.data)); }}catch(err){{}}
    }});
    es.onerror = function(){{ es.close(); setTimeout(connectStream, 5000); }};
  }}

  if(document.readyState === 'loading'){{
    document.addEventListener('DOMContentLoaded', connectStream);
  }} else {{
    connectStream();
  }}
}})();
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

/* ── Position P&L sort (4-mode cycle) ───────────────────────────────────
   0 = entry time order (default)
   1 = best first (biggest winners at top)
   2 = worst first (biggest losers at top — notice the pain early)
   3 = split: all winners first (sorted desc), then all losers (sorted asc)
   Mode persists in _posSortMode across calls.                           */
var _posSortMode = 0;
function sortPositions(){{
  var grid = document.getElementById('pos-grid');
  if(!grid) return;
  _posSortMode = (_posSortMode + 1) % 4;
  var cards = Array.from(grid.querySelectorAll('.pos-flip-wrap'));
  var btn = document.getElementById('sort-pos-btn');
  var badge = document.getElementById('wl-badge');

  if(_posSortMode === 0) {{
    /* restore original DOM order (time — data-idx set at render) */
    cards.sort(function(a,b){{
      return parseInt(a.getAttribute('data-idx')||'0') - parseInt(b.getAttribute('data-idx')||'0');
    }});
    cards.forEach(function(c){{ grid.appendChild(c); }});
    if(btn) btn.textContent = '↕ Sort P&L';
    if(badge) badge.textContent = '';
  }} else if(_posSortMode === 1) {{
    /* Best first (highest P&L) */
    cards.sort(function(a,b){{
      return parseFloat(b.getAttribute('data-pnl')||'0') - parseFloat(a.getAttribute('data-pnl')||'0');
    }});
    cards.forEach(function(c){{ grid.appendChild(c); }});
    if(btn) btn.textContent = '↓ Best first';
    var winners = cards.filter(function(c){{ return parseFloat(c.getAttribute('data-pnl')||'0') >= 0; }}).length;
    var losers  = cards.length - winners;
    if(badge) {{ badge.textContent = '🟢 ' + winners + ' W · 🔴 ' + losers + ' L'; badge.style.color='#cdd9e5'; }}
  }} else if(_posSortMode === 2) {{
    /* Worst first (biggest losers at top) */
    cards.sort(function(a,b){{
      return parseFloat(a.getAttribute('data-pnl')||'0') - parseFloat(b.getAttribute('data-pnl')||'0');
    }});
    cards.forEach(function(c){{ grid.appendChild(c); }});
    if(btn) btn.textContent = '↑ Worst first';
    var winners2 = cards.filter(function(c){{ return parseFloat(c.getAttribute('data-pnl')||'0') >= 0; }}).length;
    var losers2  = cards.length - winners2;
    if(badge) {{ badge.textContent = '🟢 ' + winners2 + ' W · 🔴 ' + losers2 + ' L'; badge.style.color='#cdd9e5'; }}
  }} else {{
    /* Split: winners first (sorted by P&L desc), then losers (sorted by P&L asc = worst last) */
    var winCards  = cards.filter(function(c){{ return parseFloat(c.getAttribute('data-pnl')||'0') >= 0; }});
    var loseCards = cards.filter(function(c){{ return parseFloat(c.getAttribute('data-pnl')||'0') < 0; }});
    winCards.sort(function(a,b){{  return parseFloat(b.getAttribute('data-pnl')||'0') - parseFloat(a.getAttribute('data-pnl')||'0'); }});
    loseCards.sort(function(a,b){{ return parseFloat(a.getAttribute('data-pnl')||'0') - parseFloat(b.getAttribute('data-pnl')||'0'); }});
    winCards.concat(loseCards).forEach(function(c){{ grid.appendChild(c); }});
    if(btn) btn.textContent = '⊞ W | L split';
    if(badge) {{ badge.textContent = '🟢 ' + winCards.length + ' winning · 🔴 ' + loseCards.length + ' losing'; badge.style.color='#cdd9e5'; }}
  }}
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

  /* ── Cycle-complete detection ────────────────────────────────────────────
     next_cycle_at is stamped to now+30s exactly once when analysis finishes.
     When the poller sees it change to a NEW future value, the cycle just
     ended — call loadState() for a smooth in-place DOM refresh instead of a
     hard page reload. */
  var _lastCycleAt = {next_cycle_at_ms};

  /* ── Macro-pill live updater ─────────────────────────────────────────────
     Keeps the header regime badge in sync with s.macro_regime from /api/state
     without requiring a page reload. */
  function updateMacroPill(regime) {{
    var pill = document.getElementById('macro-pill');
    if (!pill) return;
    var label, bg, fg, border, dot;
    if (regime === 'BULL') {{
      label = '🐂 BULL'; bg = '#0d2318'; fg = '#3fb950'; border = '#238636'; dot = '#3fb950';
    }} else if (regime === 'BEAR') {{
      label = '🐻 BEAR'; bg = '#2d0f0d'; fg = '#f85149'; border = '#da3633'; dot = '#f85149';
    }} else {{
      label = '◎ NEUTRAL'; bg = '#1c1c1c'; fg = '#8b949e'; border = '#30363d'; dot = '#8b949e';
    }}
    pill.style.background = bg;
    pill.style.color = fg;
    pill.style.borderColor = border;
    var dotEl = pill.querySelector('span');
    if (dotEl) dotEl.style.background = dot;
    /* Replace text node (last child) */
    var nodes = pill.childNodes;
    for (var i = nodes.length - 1; i >= 0; i--) {{
      if (nodes[i].nodeType === 3) {{ nodes[i].textContent = ' ' + label; break; }}
    }}
  }}

  /* ── Live data polling every 5s — updates key numbers without page flicker ─ */
  function $id(id){{return document.getElementById(id);}}
  function fmt2(n){{return Math.abs(n).toFixed(2);}}
  function sign(n){{return n>=0?'+':'-';}}
  function col(n){{return n>=0?'#3fb950':'#f85149';}}

  function applyPoll(s){{
    /* ── Smooth refresh when cycle timestamp advances ────────────────────
       next_cycle_at is only stamped once per cycle (end of analysis loop).
       When it changes to a strictly-newer value the full cycle just completed.
       Instead of a hard reload, update the macro pill in-place and call
       loadState() for a full DOM refresh — no flicker, no scroll-jump.
       Skip on very first poll (_lastCycleAt == 0 means page just loaded). */
    var newCycleAt = s.next_cycle_at || 0;
    if(_lastCycleAt > 0 && newCycleAt > 0 && newCycleAt !== _lastCycleAt){{
      updateMacroPill(s.macro_regime || 'NEUTRAL');
      if(typeof loadState === 'function') loadState();
    }}
    if(newCycleAt > 0) _lastCycleAt = newCycleAt;

    /* Equity hero */
    var unrealised=0,committed=0;
    (s.positions||[]).forEach(function(p){{unrealised+=p.unrealised_pnl;committed+=p.size_usd;}});
    var equity=s.capital+committed+unrealised;
    var total_pnl=s.pnl+unrealised;
    var pnl_pct=s.initial_capital>0?(total_pnl/s.initial_capital*100):0;

    var ev=$id('equity-val');
    if(ev)ev.textContent='$'+equity.toFixed(2);

    var ef=$id('equity-free');
    if(ef)ef.textContent='$'+s.capital.toFixed(2);
    var it=$id('equity-in-trades');
    if(it)it.textContent='$'+(committed+unrealised).toFixed(2);
    var op=$id('op-pool');if(op){{op.textContent='$'+(s.house_money_pool||0).toFixed(2);op.closest('.eq-row').style.color=(s.house_money_pool||0)>0?'#3fb950':'#8b949e';}}

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

  var _pollErrCount = 0;
  function poll(){{
    fetch('/api/state').then(function(r){{return r.json();}}).then(function(s){{
      _pollErrCount = 0;
      applyPoll(s);
    }}).catch(function(e){{
      _pollErrCount++;
      // After 3 consecutive failures show a subtle stale-data warning so the
      // operator knows the numbers they're looking at may be out of date.
      if (_pollErrCount === 3) {{
        // silently degrade
        var ev = document.getElementById('equity-val');
        if (ev && !ev.dataset.staleMark) {{
          ev.dataset.staleMark = '1';
          ev.title = 'Warning: dashboard data may be stale (API unreachable)';
          ev.style.opacity = '0.5';
        }}
      }}
    }});
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
        setTimeout(function(){{ if(typeof loadState==='function') loadState(); }}, 1200);
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

<!-- ── Build signature ───────────────────────────────────────────────── -->
<div style="position:fixed;bottom:6px;right:10px;font-size:.62rem;color:#444c56;
            letter-spacing:.3px;pointer-events:none;z-index:9999;font-family:monospace">
  v{pkg_version} · {git_hash}
</div>

</body></html>"#,
        last_update = s.last_update,
        equity = equity,
        capital = s.capital,
        in_trades = committed + unrealised,
        pool_bal = s.house_money_pool,
        wallet_label = wallet_label,
        wallet_href = wallet_href,
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
        // Cap Sortino at 999 for display — with 0 losing trades the denominator
        // is near-zero, producing astronomically large values that break the layout.
        sortino = m.sortino.min(999.0),
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
        cb_threshold_info = cb_threshold_info,
        open_n = s.positions.len(),
        pos_cap = 20, // updated from 25 — matches MAX_OPEN_POSITIONS in main.rs
        total_closed = s.closed_trades.len(),
        cycles = s.cycle_count,
        cand_n = s.candidates.len(),
        committed = committed,
        heat_pct = heat_pct,
        avg_open_r_str = avg_open_r_str,
        avg_r_colour = avg_r_colour,
        slots_colour = slots_colour,
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
        macro_label  = macro_label,
        macro_bg     = macro_bg,
        macro_fg     = macro_fg,
        macro_border = macro_border,
        macro_dot    = macro_dot,
        pkg_version = env!("CARGO_PKG_VERSION"),
        git_hash    = env!("GIT_COMMIT_HASH"),
    ))
}

/// Inline weight item: label · value · tiny bar  (single-line strip)
pub(crate) fn wi(label: &str, val: f64) -> String {
    format!(
        r#"<span class="w-item"><span class="w-item-label">{label}</span><span class="w-item-val">{val:.2}</span><div class="w-item-bar"><div class="w-item-fill" style="width:{pct:.0}%"></div></div></span>"#,
        label = label,
        val = val,
        pct = (val * 100.0).min(100.0),
    )
}

pub(crate) fn reason_class(r: &str) -> &'static str {
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

pub(crate) async fn api_state_handler(State(app): State<AppState>, headers: HeaderMap) -> axum::response::Response {
    use axum::response::IntoResponse;
    let has_session = get_session_tenant_id(&headers, &app.session_secret).is_some();
    let has_admin = app.admin_password.as_deref().map(|pw| check_admin_auth(&headers, pw)).unwrap_or(false);
    if !has_session && !has_admin { return axum::http::StatusCode::UNAUTHORIZED.into_response(); }
    let mut state = app.bot_state.read().await.clone();
    state.hyperliquid_stats = app.hyperliquid_stats.snapshot().await;
    // Trim closed_trades to the most recent 100 for the API response.
    // The full 500-entry ring buffer is kept in memory for metrics; the
    // frontend only renders the last 12–50 entries anyway.
    if state.closed_trades.len() > 100 {
        let drain_count = state.closed_trades.len() - 100;
        state.closed_trades.drain(..drain_count);
    }
    Json(state).into_response()
}

// ─────────────────────────── Consumer webapp ─────────────────────────────────

/// Shared CSS + HTML boilerplate for all consumer pages.
