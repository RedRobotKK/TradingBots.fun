//! RedRobot HedgeBot – Autonomous Cryptocurrency Trading System
//!
//! Professional quant trade management:
//!   • Kelly Criterion position sizing (half-Kelly, confidence-scaled)
//!   • Portfolio heat limit: max 8% total equity at risk across all positions
//!   • Per-trade risk: max 2% equity at risk (stop-distance based)
//!   • R-multiple exits: 1/3 at 2R, 1/3 at 4R, trail final 1/3 with no ceiling
//!   • Trailing stop: breakeven at 1R, trail 1.2×ATR from HWM at 1.5R
//!   • Time exit: close stale trades (<0.5R after 8 cycles)
//!   • Circuit breaker: 0.35× size multiplier when equity drawdown >8%
//!   • Pyramid: add to winners (existing >1R profit + new signal = +50% add-on)
//!   • Online learning: signal weights updated after every close/partial

mod config;
mod data;
mod indicators;
mod signals;
mod risk;
mod exchange;
mod decision;
mod db;
mod learner;
mod metrics;
mod sentiment;
mod web_dashboard;
mod candlestick_patterns;
mod chart_patterns;
mod ai_reviewer;
mod coins;
mod funding;

use anyhow::Result;
use log::{error, info, warn};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use web_dashboard::{
    BotState, CandidateInfo, ClosedTrade, DecisionInfo, PaperPosition, SharedState,
};
use learner::{SharedWeights, SignalWeights};
use metrics::PerformanceMetrics;
use sentiment::{SentimentCache, SharedSentiment};
use funding::{FundingCache, SharedFunding};

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_default_env()
        .format_timestamp_millis()
        .init();

    info!("🤖 RedRobot HedgeBot Starting — Professional Quant Mode");

    let config = config::Config::from_env()?;
    info!("✓ Config: mode={:?}  capital=${:.0}  paper={}", config.mode, config.initial_capital, config.paper_trading);

    let db     = Arc::new(db::Database::new(&config.database_url).await?);
    let market = Arc::new(data::MarketClient::new());
    let hl     = Arc::new(exchange::HyperliquidClient::new(&config)?);

    // LunarCrush sentiment client (pre-warm cache at startup)
    let sentiment_cache: SharedSentiment = SentimentCache::new(config.lunarcrush_api_key.clone());
    sentiment_cache.warm().await;

    // Binance funding rate cache (pre-warm; refreshes every 3 min automatically)
    let funding_cache: SharedFunding = FundingCache::new();
    funding_cache.warm().await;

    info!("✓ Clients ready (LunarCrush sentiment + Binance funding rates active)");

    let weights: SharedWeights = Arc::new(RwLock::new(SignalWeights::load()));

    // BTC dominance — fetched live at startup, refreshed every ~10 min in cycle
    let btc_dominance: SharedBtcDominance = {
        let dom = fetch_btc_dominance().await.unwrap_or(56.0);
        info!("✓ BTC dominance (live): {:.1}%", dom);
        Arc::new(RwLock::new(dom))
    };

    let bot_state: SharedState = Arc::new(RwLock::new(BotState {
        capital:         config.initial_capital,
        initial_capital: config.initial_capital,
        peak_equity:     config.initial_capital,
        signal_weights:  SignalWeights::load(),
        ..BotState::default()
    }));

    // Dashboard
    {
        let ds = bot_state.clone();
        tokio::spawn(async move {
            if let Err(e) = web_dashboard::serve(ds, 3000).await {
                error!("Dashboard: {}", e);
            }
        });
    }

    // Ctrl-C handler
    let running = Arc::new(RwLock::new(true));
    {
        let r = running.clone();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            info!("Shutdown signal");
            *r.write().await = false;
        });
    }

    let mut prev_mids: HashMap<String, f64> = HashMap::new();
    info!("🚀 Main loop started (30 s cycle, paper={})", config.paper_trading);

    loop {
        if !*running.read().await { break; }

        set_status(&bot_state, "📡 Fetching prices…").await;

        match run_cycle(&config, &market, &hl, &db, &bot_state, &weights,
                        &sentiment_cache, &funding_cache, &mut prev_mids, &btc_dominance).await
        {
            Ok(_) => {
                set_status(&bot_state, "⏳ Waiting for next cycle (30s)…").await;
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            }
            Err(e) => {
                error!("Cycle error: {}", e);
                set_status(&bot_state, &format!("⚠ Error: {} — retrying in 10s", e)).await;
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        }
    }

    info!("🛑 Shutdown complete");
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════════
//  BTC DOMINANCE FETCH (CoinGecko free /global endpoint)
// ═══════════════════════════════════════════════════════════════════════════════

/// Fetch live BTC market-cap dominance from CoinGecko's free /global endpoint.
/// Returns `None` if the request fails or the field is missing.
async fn fetch_btc_dominance() -> Option<f64> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(6))
        .build().ok()?;
    let resp: serde_json::Value = client
        .get("https://api.coingecko.com/api/v3/global")
        .send().await.ok()?
        .json().await.ok()?;
    resp["data"]["market_cap_percentage"]["btc"].as_f64()
}

// ═══════════════════════════════════════════════════════════════════════════════
//  MAIN CYCLE
// ═══════════════════════════════════════════════════════════════════════════════

type SharedBtcDominance = Arc<RwLock<f64>>;

async fn run_cycle(
    config:          &config::Config,
    market:          &Arc<data::MarketClient>,
    hl:              &Arc<exchange::HyperliquidClient>,
    db:              &Arc<db::Database>,
    bot_state:       &SharedState,
    weights:         &SharedWeights,
    sent_cache:      &SharedSentiment,
    fund_cache:      &SharedFunding,
    prev_mids:       &mut HashMap<String, f64>,
    btc_dominance:   &SharedBtcDominance,
) -> Result<()> {
    { bot_state.write().await.cycle_count += 1; }

    // ── Tier 1: all prices in one Hyperliquid call ────────────────────────
    set_status(bot_state, "📡 Hyperliquid allMids…").await;
    let current_mids = market.fetch_all_mids().await
        .map_err(|e| { warn!("allMids: {}", e); e })?;
    info!("📊 {} prices", current_mids.len());

    // Save old prices BEFORE overwriting — needed for change_pct display
    let prev_snapshot = prev_mids.clone();
    let candidates_raw = market.filter_candidates(&current_mids, prev_mids);
    *prev_mids = current_mids.clone();

    // Priority coins always analysed FIRST so they get slot priority
    const PRIORITY: &[&str] = &["SOL", "BTC", "ETH", "BNB", "AVAX"];
    let mut candidates: Vec<String> = PRIORITY.iter()
        .filter_map(|&p| candidates_raw.iter().find(|s| s.as_str() == p).cloned())
        .collect();
    for sym in &candidates_raw {
        if !candidates.contains(sym) { candidates.push(sym.clone()); }
    }

    // ── Update peak equity & increment cycles_held ────────────────────────
    {
        let mut s = bot_state.write().await;
        let committed: f64 = s.positions.iter().map(|p| p.size_usd).sum();
        let unrealised: f64 = s.positions.iter().map(|p| p.unrealised_pnl).sum();
        let equity = s.capital + committed + unrealised;
        if equity > s.peak_equity { s.peak_equity = equity; }
        for pos in s.positions.iter_mut() { pos.cycles_held += 1; }
    }

    // ── Position management: P&L, trailing stops, exit signals ───────────
    let open_count = bot_state.read().await.positions.len();
    if open_count > 0 {
        set_status(bot_state,
            &format!("🔍 Managing {} open position(s)…", open_count)).await;
    }

    let mut to_close:    Vec<(String, f64, String)> = Vec::new(); // (sym, exit, reason)
    let mut to_partial1: Vec<(String, f64)>         = Vec::new(); // 1/3 out at 2R
    let mut to_partial2: Vec<(String, f64)>         = Vec::new(); // 1/3 out at 4R

    {
        let mut s = bot_state.write().await;
        for pos in s.positions.iter_mut() {
            let cur = match current_mids.get(pos.symbol.as_str()) {
                Some(&p) => p,
                None     => continue,
            };

            // Water marks
            if cur > pos.high_water_mark { pos.high_water_mark = cur; }
            if cur < pos.low_water_mark  { pos.low_water_mark  = cur; }

            // Unrealised P&L
            pos.unrealised_pnl = if pos.side == "LONG" {
                (cur - pos.entry_price) * pos.quantity
            } else {
                (pos.entry_price - cur) * pos.quantity
            };

            // R-multiple (uses original dollars risked, not current stop)
            let r_mult = if pos.r_dollars_risked > 1e-8 {
                pos.unrealised_pnl / pos.r_dollars_risked
            } else { 0.0 };

            let atr = pos.atr_at_entry.max(pos.entry_price * 0.001);

            // ── Trailing stop logic ───────────────────────────────────────
            if pos.side == "LONG" {
                // Breakeven at 1R profit
                if r_mult >= 1.0 && pos.stop_loss < pos.entry_price {
                    pos.stop_loss = pos.entry_price;
                    info!("📌 {} LONG stop → breakeven ${:.4}", pos.symbol, pos.entry_price);
                }
                // Trail 1.2×ATR below HWM once 1.5R ahead
                if r_mult >= 1.5 {
                    let trail = pos.high_water_mark - atr * 1.2;
                    if trail > pos.stop_loss {
                        pos.stop_loss = trail;
                    }
                }
            } else { // SHORT
                if r_mult >= 1.0 && pos.stop_loss > pos.entry_price {
                    pos.stop_loss = pos.entry_price;
                    info!("📌 {} SHORT stop → breakeven ${:.4}", pos.symbol, pos.entry_price);
                }
                if r_mult >= 1.5 {
                    let trail = pos.low_water_mark + atr * 1.2;
                    if trail < pos.stop_loss {
                        pos.stop_loss = trail;
                    }
                }
            }

            // ── Exit checks ───────────────────────────────────────────────
            let hit_stop = (pos.side == "LONG"  && cur <= pos.stop_loss)
                        || (pos.side == "SHORT" && cur >= pos.stop_loss);
            let hit_tp   = (pos.side == "LONG"  && cur >= pos.take_profit)
                        || (pos.side == "SHORT" && cur <= pos.take_profit);

            // ── Time exit: only genuinely dead money after HOURS, not minutes ──
            //
            // Rules (all must hold):
            //   • Never time-exit a winning position (let winners run)
            //   • "Truly flat": no meaningful progress either way after 2 hrs
            //     → cycles_held ≥ 240 (120 min) AND |r_mult| < 0.20
            //   • "Chronic bleeder": consistently losing after 3 hrs with NO DCA taken
            //     → cycles_held ≥ 360 (180 min) AND r_mult < -0.45 AND dca_count == 0
            //   • Post-DCA bleeder: DCA didn't rescue it after another 2 hrs
            //     → cycles_held ≥ 480 (240 min) AND r_mult < -0.60 AND dca_count > 0
            //   • Never time-exit within 0.10R of stop (let stop-loss handle)
            let truly_flat     = pos.cycles_held >= 240 && r_mult.abs() < 0.20;
            let chronic_loss   = pos.cycles_held >= 360 && r_mult < -0.45 && r_mult > -0.90 && pos.dca_count == 0;
            let post_dca_loss  = pos.cycles_held >= 480 && r_mult < -0.60 && r_mult > -0.90 && pos.dca_count > 0;
            let stale = (truly_flat || chronic_loss || post_dca_loss) && r_mult <= 0.0;

            if hit_stop {
                to_close.push((pos.symbol.clone(), cur, "StopLoss".to_string()));
            } else if hit_tp {
                to_close.push((pos.symbol.clone(), cur, "TakeProfit".to_string()));
            } else if stale {
                let reason_detail = if truly_flat { "flat" } else if chronic_loss { "chronic loss" } else { "post-DCA loss" };
                to_close.push((pos.symbol.clone(), cur, "TimeExit".to_string()));
                info!("⏰ {} time-exit ({}) after {} cycles at {:.2}R", pos.symbol, reason_detail, pos.cycles_held, r_mult);
            } else {
                // R-multiple partial profit tranches
                if r_mult >= 2.0 && pos.tranches_closed == 0 {
                    to_partial1.push((pos.symbol.clone(), cur));
                } else if r_mult >= 4.0 && pos.tranches_closed == 1 {
                    to_partial2.push((pos.symbol.clone(), cur));
                }
            }
        }
    }

    // Execute partials first (they don't remove positions)
    for (sym, price) in to_partial1 {
        if to_close.iter().any(|(s, _, _)| s == &sym) { continue; }
        take_partial(sym, price, 1, bot_state, weights).await;
    }
    for (sym, price) in to_partial2 {
        if to_close.iter().any(|(s, _, _)| s == &sym) { continue; }
        take_partial(sym, price, 2, bot_state, weights).await;
    }

    // Execute full closes
    for (sym, price, reason) in to_close {
        close_paper_position(&sym, price, &reason, bot_state, weights).await;
        info!("🚨 {} closed → {} @ ${:.4}", sym, reason, price);
    }

    // ── Update session prices (first time we see each symbol) ────────────
    {
        let mut s = bot_state.write().await;
        for (sym, &price) in &current_mids {
            s.session_prices.entry(sym.clone()).or_insert(price);
        }
    }

    // ── Update dashboard candidate list & weights mirror ─────────────────
    // Step 1: fetch sentiment for all candidates WITHOUT holding the state lock
    // (cache is in-memory so this is fast; network calls only happen on refresh)
    let mut cand_sentiment: HashMap<String, Option<sentiment::SentimentData>> = HashMap::new();
    for sym in &candidates {
        let sent = sent_cache.get(sym).await;
        cand_sentiment.insert(sym.clone(), sent);
    }
    // Step 2: build CandidateInfo and push to state under a single lock
    {
        let session_snap = bot_state.read().await.session_prices.clone();
        let cand_infos: Vec<CandidateInfo> = candidates.iter().filter_map(|sym| {
            let &price = current_mids.get(sym.as_str())?;
            // cycle 1: prev_snapshot is empty (no previous prices yet) → show "—"
            let change_pct: Option<f64> = if prev_snapshot.is_empty() {
                None
            } else {
                let session_chg = session_snap.get(sym.as_str()).map(|&base| {
                    if base > 0.0 { (price - base) / base * 100.0 } else { 0.0 }
                }).unwrap_or(0.0);
                let cycle_chg = prev_snapshot.get(sym.as_str()).map(|&prev| {
                    if prev > 0.0 { (price - prev) / prev * 100.0 } else { 0.0 }
                }).unwrap_or(0.0);
                Some(if session_chg.abs() > 0.01 { session_chg } else { cycle_chg })
            };
            let sent = cand_sentiment.get(sym).and_then(|s| s.as_ref());
            Some(CandidateInfo {
                symbol:          sym.clone(),
                price,
                change_pct,
                galaxy_score:    sent.map(|s| s.galaxy_score),
                bullish_percent: sent.map(|s| s.bullish_percent),
                alt_rank:        sent.map(|s| s.alt_rank),
            })
        }).collect();
        let mut s = bot_state.write().await;
        s.candidates     = cand_infos;
        s.last_update    = now_str();
        s.signal_weights = weights.read().await.clone();
    }

    // ── BTC market context (dominance + direction) ───────────────────────
    // Refresh live dominance every 20 cycles (~10 min).  Between refreshes
    // the cached value is reused so we don't hammer the CoinGecko free tier.
    let cycle_count_now = bot_state.read().await.cycle_count;
    if cycle_count_now % 20 == 1 {
        if let Some(dom) = fetch_btc_dominance().await {
            *btc_dominance.write().await = dom;
            info!("🔄 BTC dominance refreshed: {:.1}%", dom);
        } else {
            warn!("BTC dominance refresh failed — using cached {:.1}%",
                  *btc_dominance.read().await);
        }
    }

    // BTC 24h return: compare first vs last candle across the fetched window
    // (~25 h of 15-min candles).  Used same-cycle so it's always fresh.
    let btc_ctx: Option<decision::BtcMarketContext> = match market.fetch_market_data("BTC").await {
        Ok(btc_candles) if btc_candles.len() >= 2 => {
            let first = btc_candles.first().unwrap().close;
            let last  = btc_candles.last().unwrap().close;
            let ret   = (last - first) / first * 100.0;
            let dom   = *btc_dominance.read().await;
            info!("🟠 BTC ctx: dom={:.1}%  24h_ret={:+.2}%", dom, ret);
            Some(decision::BtcMarketContext { dominance: dom, btc_return_24h: ret })
        }
        _ => {
            warn!("BTC candles unavailable — dominance filter disabled this cycle");
            None
        }
    };

    // ── Tier 2: analyse candidates ────────────────────────────────────────
    let total = candidates.len();
    let mut new_decisions: Vec<DecisionInfo> = Vec::new();

    for (i, sym) in candidates.iter().enumerate() {
        set_status(bot_state,
            &format!("🔬 Analysing {}/{}: {}…", i + 1, total, sym)).await;

        match analyse_symbol(sym, market, hl, db, config, bot_state, weights, sent_cache, fund_cache, btc_ctx.as_ref()).await {
            Ok(Some(dec)) if dec.action != "SKIP" => {
                info!("💡 {} → {} conf={:.0}%", sym, dec.action, dec.confidence * 100.0);
                new_decisions.push(DecisionInfo {
                    symbol:      sym.clone(),
                    action:      dec.action.clone(),
                    confidence:  dec.confidence,
                    entry_price: dec.entry_price,
                    rationale:   dec.rationale.clone(),
                    timestamp:   now_str(),
                });
            }
            Ok(_)  => {}
            Err(e) => warn!("  {} error: {}", sym, e),
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }

    if !new_decisions.is_empty() {
        let mut s = bot_state.write().await;
        s.recent_decisions.extend(new_decisions);
        let len = s.recent_decisions.len();
        if len > 50 { s.recent_decisions.drain(0..len - 50); }
    }

    { let mut s = bot_state.write().await; s.last_update = now_str(); }

    // ── AI position review (every 10 cycles ≈ 5 minutes) ─────────────────
    let (cycle_count, open_count) = {
        let s = bot_state.read().await;
        (s.cycle_count, s.positions.len())
    };
    if cycle_count % 10 == 0 && open_count > 0 {
        if let Some(api_key) = &config.anthropic_api_key {
            set_status(bot_state, "🤖 Claude AI reviewing positions…").await;
            let (positions_snap, metrics_snap, capital_snap) = {
                let s = bot_state.read().await;
                (s.positions.clone(), s.metrics.clone(), s.capital)
            };
            let review = ai_reviewer::review_positions(
                &positions_snap, &metrics_snap, capital_snap, api_key,
            ).await;
            apply_ai_review(&review, bot_state, weights, &current_mids).await;
        }
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════════
//  AI REVIEW APPLICATION
// ═══════════════════════════════════════════════════════════════════════════════

async fn apply_ai_review(
    review:      &ai_reviewer::AiReview,
    bot_state:   &SharedState,
    weights:     &SharedWeights,
    current_mids: &HashMap<String, f64>,
) {
    for rec in &review.recommendations {
        let cur_price = match current_mids.get(rec.symbol.as_str()) {
            Some(&p) => p,
            None     => continue,
        };

        match rec.action.as_str() {
            "close_now" => {
                // Guardrail: only close if position is losing (don't let AI cut winners)
                let should_close = {
                    let s = bot_state.read().await;
                    s.positions.iter().find(|p| p.symbol == rec.symbol)
                        .map(|p| {
                            let r = if p.r_dollars_risked > 1e-8 {
                                p.unrealised_pnl / p.r_dollars_risked
                            } else { 0.0 };
                            r < -0.4  // only close when genuinely losing
                        })
                        .unwrap_or(false)
                };
                if should_close {
                    info!("🤖 AI close: {} — {}", rec.symbol, rec.reason);
                    close_paper_position(&rec.symbol, cur_price, "AI-Close", bot_state, weights).await;
                } else {
                    info!("🤖 AI close {} REJECTED — position not sufficiently in loss (guardrail)", rec.symbol);
                }
            }

            "scale_up" => {
                // Guardrail: factor capped at 2.0, position must be profitable
                let factor = rec.factor.clamp(1.0, 2.0);
                let can_scale = {
                    let s = bot_state.read().await;
                    s.positions.iter().find(|p| p.symbol == rec.symbol)
                        .map(|p| {
                            let r = if p.r_dollars_risked > 1e-8 {
                                p.unrealised_pnl / p.r_dollars_risked
                            } else { 0.0 };
                            r > 0.5  // only add to winners
                        })
                        .unwrap_or(false)
                };
                if can_scale {
                    let mut s = bot_state.write().await;
                    // Use index to avoid simultaneous mutable + immutable borrow of `s`
                    if let Some(idx) = s.positions.iter().position(|p| p.symbol == rec.symbol) {
                        let add_usd = s.positions[idx].size_usd * (factor - 1.0);
                        let lev     = s.positions[idx].leverage;
                        if s.capital >= add_usd && add_usd >= 1.0 {
                            let add_qty = add_usd * lev / cur_price;
                            s.capital                  -= add_usd;
                            s.positions[idx].quantity  += add_qty;
                            s.positions[idx].size_usd  += add_usd;
                            info!("🤖 AI scale-up {} ×{:.2}  +${:.2} — {}", rec.symbol, factor, add_usd, rec.reason);
                        }
                    }
                } else {
                    info!("🤖 AI scale-up {} REJECTED — R < 0.5 (guardrail)", rec.symbol);
                }
            }

            "scale_down" => {
                // Guardrail: keep at least 25% of position
                let keep_frac = rec.factor.clamp(0.25, 0.99);
                let close_frac = 1.0 - keep_frac;
                let (close_usd, close_qty) = {
                    let s = bot_state.read().await;
                    s.positions.iter().find(|p| p.symbol == rec.symbol)
                        .map(|p| (p.size_usd * close_frac, p.quantity * close_frac))
                        .unwrap_or((0.0, 0.0))
                };
                if close_usd > 1.0 {
                    let pnl_portion = {
                        let s = bot_state.read().await;
                        s.positions.iter().find(|p| p.symbol == rec.symbol)
                            .map(|p| p.unrealised_pnl * close_frac)
                            .unwrap_or(0.0)
                    };
                    let mut s = bot_state.write().await;
                    if let Some(pos) = s.positions.iter_mut().find(|p| p.symbol == rec.symbol) {
                        pos.quantity          -= close_qty;
                        pos.size_usd          -= close_usd;
                        pos.unrealised_pnl    -= pnl_portion;
                        pos.r_dollars_risked  *= keep_frac;
                        s.capital             += close_usd + pnl_portion;
                        s.pnl                 += pnl_portion;
                        info!("🤖 AI scale-down {} keep {:.0}%  realised ${:.2} — {}",
                            rec.symbol, keep_frac * 100.0, pnl_portion, rec.reason);
                    }
                }
            }

            "hold" | _ => {
                // Nothing to do — existing strategy manages the position
            }
        }

        // ── Always persist AI annotation on the position card ─────────────
        {
            let mut s = bot_state.write().await;
            if let Some(pos) = s.positions.iter_mut().find(|p| p.symbol == rec.symbol) {
                pos.ai_action = Some(rec.action.clone());
                pos.ai_reason = Some(rec.reason.chars().take(80).collect()); // cap at 80 chars
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  PER-SYMBOL ANALYSIS
// ═══════════════════════════════════════════════════════════════════════════════

async fn analyse_symbol(
    symbol:     &str,
    market:     &Arc<data::MarketClient>,
    hl:         &Arc<exchange::HyperliquidClient>,
    db:         &Arc<db::Database>,
    config:     &config::Config,
    bot_state:  &SharedState,
    weights:    &SharedWeights,
    sent_cache: &SharedSentiment,
    fund_cache: &SharedFunding,
    btc_ctx:    Option<&decision::BtcMarketContext>,
) -> Result<Option<decision::Decision>> {
    let candles = market.fetch_market_data(symbol).await?;
    if candles.len() < 26 { return Ok(None); }

    let ind    = indicators::calculate_all(&candles)?;
    let ob     = market.fetch_order_book(symbol).await?;
    let of     = signals::detect_order_flow(&ob)?;
    let w      = weights.read().await.clone();
    let sent   = sent_cache.get(symbol).await;
    let fund   = fund_cache.get(symbol).await;
    // BTC dominance context is not applied to BTC's own signal (no self-reference)
    let ctx    = if symbol == "BTC" { None } else { btc_ctx };
    let dec    = decision::make_decision(&candles, &ind, &of, &w, sent.as_ref(), fund.as_ref(), ctx)?;

    log::debug!("{}: RSI={:.1} trend={:.2}% MACD={:.5} ATR={:.4} → {}",
        symbol, ind.rsi, ind.trend, ind.macd, ind.atr, dec.action);

    if config.paper_trading && dec.action != "SKIP" {
        execute_paper_trade(symbol, &dec, &ind, bot_state, weights).await;
    } else if !config.paper_trading && dec.action != "SKIP" {
        let account = hl.get_account().await?;
        if risk::should_trade(&dec, &account)? {
            match hl.place_order(&dec).await {
                Ok(id) => { info!("✅ {} {} @ ${:.4} [{}]", dec.action, symbol, dec.entry_price, id); db.log_trade(&dec, &id).await.ok(); }
                Err(e) => error!("❌ Order failed {}: {}", symbol, e),
            }
        }
    }

    Ok(Some(dec))
}

// ═══════════════════════════════════════════════════════════════════════════════
//  POSITION SIZING  (Kelly + Sharpe multiplier + confidence)
// ═══════════════════════════════════════════════════════════════════════════════

/// Returns target fraction of free capital for this trade.
///
/// Priority order:
///   1. If half-Kelly available (≥5 trades): Kelly × confidence_scale × Sharpe_mult
///   2. Fallback confidence tiers × Sharpe_mult
///   Clamped to [1%, 18%].
fn position_size_pct(confidence: f64, metrics: &PerformanceMetrics) -> f64 {
    let sharpe_mult = metrics.size_multiplier();
    let kelly       = metrics.kelly_fraction();

    let base = if kelly > 0.0 {
        // Linear scale: conf=0.65 → 60% of Kelly, conf=1.0 → 100% of Kelly.
        // (1.0 - 0.6) / (1.0 - 0.65) = 0.4 / 0.35 ≈ 1.143 — the slope.
        let conf_scale = (0.6 + (confidence - 0.65).max(0.0) * (0.4 / 0.35)).min(1.0);
        kelly * conf_scale
    } else {
        // Pre-Kelly fallback tiers (first ~5 trades)
        match confidence {
            c if c >= 0.85 => 0.08,
            c if c >= 0.75 => 0.06,
            c if c >= 0.65 => 0.04,
            _              => 0.03,
        }
    };

    (base * sharpe_mult).clamp(0.01, 0.18)
}

/// Fraction of equity at risk for this specific trade (stop-loss based).
/// = stop_distance% × position_size_pct
fn trade_heat(entry: f64, stop: f64, size_usd: f64, equity: f64) -> f64 {
    if equity < 1.0 { return 1.0; }
    let stop_dist_pct = (entry - stop).abs() / entry.max(1e-8);
    stop_dist_pct * size_usd / equity
}

/// Total % of equity at risk across all open positions.
fn portfolio_heat(positions: &[PaperPosition], equity: f64) -> f64 {
    if equity < 1.0 { return 1.0; }
    positions.iter()
        .map(|p| p.r_dollars_risked / equity)
        .sum::<f64>()
}

// ═══════════════════════════════════════════════════════════════════════════════
//  ENTRY EXECUTION
// ═══════════════════════════════════════════════════════════════════════════════

async fn execute_paper_trade(
    symbol:    &str,
    dec:       &decision::Decision,
    ind:       &indicators::TechnicalIndicators,
    bot_state: &SharedState,
    weights:   &SharedWeights,
) {
    let target_side = if dec.action == "BUY" { "LONG" } else { "SHORT" };

    // Check for existing position on this symbol
    {
        let s = bot_state.read().await;
        let existing = s.positions.iter().find(|p| p.symbol == symbol);
        if let Some(pos) = existing {
            let r_mult = if pos.r_dollars_risked > 1e-8 {
                pos.unrealised_pnl / pos.r_dollars_risked
            } else { 0.0 };

            if pos.side == target_side {
                // ── Same direction: pyramid UP on winner ──
                if r_mult >= 1.0 && pos.tranches_closed == 0 {
                    drop(s);
                    pyramid_position(symbol, dec, ind, bot_state).await;
                    return;
                }

                // ── Same direction: DCA DOWN on moderate loser with conviction ──
                // Conditions: between -0.15R and -0.75R, ≤2 DCA add-ons, signal confidence ≥ 0.72
                if r_mult < -0.15 && r_mult > -0.75 && pos.dca_count < 2 && dec.confidence >= 0.72 {
                    drop(s);
                    dca_position(symbol, dec, ind, bot_state).await;
                    return;
                }

                // Same side but conditions not met — hold without action
                return;
            }

            // ── Opposite side: ONLY reverse on high-confidence signal ──
            // Require ≥0.68 confidence to avoid noise flipping existing positions.
            if dec.confidence < 0.68 {
                info!("⏸  {} opposing signal ignored (conf {:.0}% < 68%) — holding {} position",
                      symbol, dec.confidence * 100.0, pos.side);
                return;
            }
            // Clone values before dropping the read guard to satisfy the borrow checker
            let pos_side    = pos.side.clone();
            let r_mult_snap = r_mult;
            drop(s);
            info!("🔄 {} signal reversal: {} at {:.2}R  conf={:.0}%",
                  symbol, pos_side, r_mult_snap, dec.confidence * 100.0);
            close_paper_position(symbol, dec.entry_price, "SignalExit", bot_state, weights).await;
        }
        // No existing: fall through to new entry
    }

    // ── Minimum confidence gate ────────────────────────────────────────────
    // Only enter trades where the signal is genuinely strong. Weak-confidence
    // entries (< 0.68) generated 0W/14L in choppy markets — not worth the risk.
    if dec.confidence < 0.68 {
        info!("⚠ {} skipped — confidence {:.0}% below 68% minimum", symbol, dec.confidence * 100.0);
        return;
    }

    // ── Open new position ─────────────────────────────────────────────────
    let atr = ind.atr.max(dec.entry_price * 0.001);

    let mut s     = bot_state.write().await;
    let metrics   = s.metrics.clone();
    let pct       = position_size_pct(dec.confidence, &metrics);
    let mut size_usd  = s.capital * pct;
    let equity    = s.capital + s.positions.iter().map(|p| p.size_usd + p.unrealised_pnl).sum::<f64>();

    // Guard: max 4 concurrent positions (reduced from 8 — quality over quantity)
    if s.positions.len() >= 4 {
        info!("⚠ {} skipped — max 4 positions open", symbol);
        return;
    }
    // Guard: max 2 positions in the same direction (prevent directional overexposure)
    let same_dir = s.positions.iter().filter(|p| p.side == target_side).count();
    if same_dir >= 2 {
        info!("⚠ {} skipped — already {} {} positions (max 2 per direction)", symbol, same_dir, target_side);
        return;
    }
    // Guard: min position size
    if size_usd < 2.0 || s.capital < size_usd {
        info!("⚠ {} skipped — insufficient capital (${:.2})", symbol, s.capital);
        return;
    }
    // Guard: per-trade heat ≤ 2% of equity.
    // If the default Kelly/confidence size would exceed the heat limit, scale it
    // down to the maximum allowed size rather than skipping the trade entirely.
    let stop_dist_pct = (dec.entry_price - dec.stop_loss).abs() / dec.entry_price.max(1e-8);
    let t_heat = trade_heat(dec.entry_price, dec.stop_loss, size_usd, equity);
    if t_heat > 0.02 {
        // Max size that keeps R-risk at exactly 2% of equity
        let allowed = 0.02 * equity / stop_dist_pct;
        if allowed < 2.0 {
            info!("⚠ {} skipped — stop too tight, min heat size ${:.2} < $2", symbol, allowed);
            return;
        }
        info!("🌡 {} heat-scaled: ${:.2} → ${:.2} (stop_dist={:.2}%)",
              symbol, size_usd, allowed, stop_dist_pct * 100.0);
        size_usd = allowed;  // apply the reduction — enforce the heat limit
    }
    // Guard: total portfolio heat ≤ 8%
    let p_heat = portfolio_heat(&s.positions, equity);
    if p_heat >= 0.08 {
        info!("🔥 {} skipped — portfolio heat {:.1}% (max 8%)", symbol, p_heat * 100.0);
        return;
    }

    // Apply confidence-scaled leverage — quantity based on notional, capital deducted at margin
    let leverage  = dec.leverage;
    let notional  = size_usd * leverage;
    let qty       = notional / dec.entry_price;
    let r_risk    = (dec.entry_price - dec.stop_loss).abs() * qty; // dollars risked on notional

    s.capital -= size_usd;  // only deduct margin, not notional
    s.positions.push(PaperPosition {
        symbol:           symbol.to_string(),
        side:             target_side.to_string(),
        entry_price:      dec.entry_price,
        quantity:         qty,
        size_usd,
        stop_loss:        dec.stop_loss,
        take_profit:      dec.take_profit,
        atr_at_entry:     atr,
        high_water_mark:  dec.entry_price,
        low_water_mark:   dec.entry_price,
        partial_closed:   false,
        r_dollars_risked: r_risk,
        tranches_closed:  0,
        dca_count:        0,
        leverage,
        cycles_held:      0,
        entry_time:       now_str(),
        unrealised_pnl:   0.0,
        contrib:          dec.signal_contribution.clone(),
        ai_action:        None,
        ai_reason:        None,
    });

    let kelly_str = if metrics.kelly_fraction() > 0.0 {
        format!("Kelly={:.1}%", metrics.kelly_fraction() * 100.0)
    } else { "pre-Kelly".to_string() };

    info!("📝 {} {} @ ${:.4}  margin=${:.2}  {:.1}×lev  notional=${:.2}  R=${:.2}  heat={:.1}%  [{}]",
        target_side, symbol, dec.entry_price,
        size_usd, leverage, notional,
        r_risk, p_heat * 100.0, kelly_str);
}

/// Add 50% of original entry size to an existing winning position (pyramid).
async fn pyramid_position(
    symbol:    &str,
    dec:       &decision::Decision,
    ind:       &indicators::TechnicalIndicators,
    bot_state: &SharedState,
) {
    let atr = ind.atr.max(dec.entry_price * 0.001);
    let mut s = bot_state.write().await;

    let idx = s.positions.iter().position(|p| p.symbol == symbol);
    if let Some(idx) = idx {
        // Add-on size = 50% of the current remaining position size
        let add_size = s.positions[idx].size_usd * 0.5;
        if s.capital < add_size || add_size < 1.0 { return; }

        let add_qty = add_size / dec.entry_price;
        s.capital -= add_size;

        s.positions[idx].quantity         += add_qty;
        s.positions[idx].size_usd         += add_size;
        s.positions[idx].r_dollars_risked += (dec.entry_price - s.positions[idx].stop_loss).abs() * add_qty;
        // Tighten stop to pyramided entry's stop if it's better
        if dec.stop_loss > s.positions[idx].stop_loss && s.positions[idx].side == "LONG" {
            s.positions[idx].stop_loss = dec.stop_loss;
        } else if dec.stop_loss < s.positions[idx].stop_loss && s.positions[idx].side == "SHORT" {
            s.positions[idx].stop_loss = dec.stop_loss;
        }
        // Update HWM to current price
        if dec.entry_price > s.positions[idx].high_water_mark {
            s.positions[idx].high_water_mark = dec.entry_price;
        }
        let _ = atr;

        info!("📈 PYRAMID {} @ ${:.4} +${:.2} (total ${:.2})",
            symbol, dec.entry_price, add_size, s.positions[idx].size_usd);
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  DCA — AVERAGE DOWN INTO A LOSING POSITION WITH STRONG CONVICTION
// ═══════════════════════════════════════════════════════════════════════════════

/// Average down into an existing losing position (DCA add-on).
///
/// Called when:
///   • Position is between −0.15R and −0.75R (mild loss, not near stop)
///   • Signal still confirms the original direction with ≥ 0.72 confidence
///   • dca_count < 2 (max 2 adds)
///
/// What it does:
///   • Adds 50% of the CURRENT position size as new capital
///   • Recomputes weighted average entry price (lowers it for LONG, raises for SHORT)
///   • Updates stop-loss to 2×ATR from the new average entry (never worsens stop)
///   • Increments dca_count
async fn dca_position(
    symbol:    &str,
    dec:       &decision::Decision,
    ind:       &indicators::TechnicalIndicators,
    bot_state: &SharedState,
) {
    let atr       = ind.atr.max(dec.entry_price * 0.001);
    let mut s     = bot_state.write().await;
    let idx       = s.positions.iter().position(|p| p.symbol == symbol);

    if let Some(idx) = idx {
        let add_size = s.positions[idx].size_usd * 0.50;
        if s.capital < add_size || add_size < 1.0 {
            info!("⚠ DCA {} skipped — insufficient capital (${:.2})", symbol, s.capital);
            return;
        }

        let add_qty   = add_size / dec.entry_price;
        let old_qty   = s.positions[idx].quantity;
        let old_entry = s.positions[idx].entry_price;
        let new_qty   = old_qty + add_qty;

        // Weighted average entry
        let avg_entry = (old_entry * old_qty + dec.entry_price * add_qty) / new_qty;

        // New ATR-based stop from the average entry (2×ATR)
        let new_stop = if s.positions[idx].side == "LONG" {
            avg_entry - atr * 2.0
        } else {
            avg_entry + atr * 2.0
        };

        // Only tighten the stop (move it closer to entry for LONG = higher value)
        let improved_stop = if s.positions[idx].side == "LONG" {
            new_stop.max(s.positions[idx].stop_loss) // higher stop = better for LONG
        } else {
            new_stop.min(s.positions[idx].stop_loss) // lower stop = better for SHORT
        };

        s.capital -= add_size;
        s.positions[idx].quantity     = new_qty;
        s.positions[idx].size_usd    += add_size;
        s.positions[idx].entry_price  = avg_entry;
        s.positions[idx].stop_loss    = improved_stop;
        s.positions[idx].dca_count   += 1;

        // Recalculate dollars risked from new avg entry and stop
        s.positions[idx].r_dollars_risked =
            (avg_entry - improved_stop).abs() * new_qty;

        info!("📉 DCA×{} {} @ ${:.4}  avg_entry=${:.4}  stop=${:.4}  +${:.2}  total=${:.2}",
            s.positions[idx].dca_count, symbol,
            dec.entry_price, avg_entry, improved_stop,
            add_size, s.positions[idx].size_usd);
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  PARTIAL PROFIT TAKING  (1/3 at 2R, 1/3 at 4R)
// ═══════════════════════════════════════════════════════════════════════════════

/// Close one tranche (1/3 of remaining position) at R-multiple milestone.
/// tranche=1 → 2R milestone, tranche=2 → 4R milestone.
async fn take_partial(
    symbol:     String,
    exit_price: f64,
    tranche:    u8,
    bot_state:  &SharedState,
    weights:    &SharedWeights,
) {
    let mut s = bot_state.write().await;
    let idx = s.positions.iter().position(|p| p.symbol == symbol && p.tranches_closed < tranche);
    if let Some(idx) = idx {
        // Take 1/3 of remaining position
        let close_qty  = s.positions[idx].quantity / 3.0;
        let close_size = s.positions[idx].size_usd / 3.0;
        let entry      = s.positions[idx].entry_price;
        let side       = s.positions[idx].side.clone();
        let contrib    = s.positions[idx].contrib.clone();
        let was_long   = side == "LONG";

        let trade_pnl = if was_long {
            (exit_price - entry) * close_qty
        } else {
            (entry - exit_price) * close_qty
        };

        s.positions[idx].quantity        -= close_qty;
        s.positions[idx].size_usd        -= close_size;
        s.positions[idx].r_dollars_risked *= 2.0 / 3.0; // scale down risked dollars
        s.positions[idx].tranches_closed  = tranche;
        s.positions[idx].partial_closed   = true;

        s.capital += close_size + trade_pnl;
        s.pnl     += trade_pnl;

        let pnl_pct    = trade_pnl / close_size * 100.0;
        let r_label    = if tranche == 1 { "2R" } else { "4R" };

        info!("💰 {}R PARTIAL {} {} @ ${:.4}  P&L {:+.2} ({:+.1}%)  [⅓ closed]",
            if tranche == 1 { 2 } else { 4 }, side, symbol, exit_price, trade_pnl, pnl_pct);

        s.closed_trades.push(ClosedTrade {
            symbol:    symbol.clone(),
            side,
            entry,
            exit:      exit_price,
            pnl:       trade_pnl,
            pnl_pct,
            reason:    format!("Partial{}R", r_label),
            closed_at: now_str(),
        });
        let len = s.closed_trades.len();
        if len > 100 { s.closed_trades.drain(0..len - 100); }

        s.metrics = PerformanceMetrics::calculate(&s.closed_trades);
        let m = &s.metrics;
        info!("📈 Metrics → Sharpe:{:.2} Kelly:{:.1}% WinRate:{:.0}%",
            m.sharpe,
            if m.kelly_fraction() > 0.0 { m.kelly_fraction() * 100.0 } else { 0.0 },
            m.win_rate * 100.0);

        drop(s);
        let mut w = weights.write().await;
        w.update(&contrib, was_long, trade_pnl > 0.0);
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  FULL CLOSE
// ═══════════════════════════════════════════════════════════════════════════════

async fn close_paper_position(
    symbol:     &str,
    exit_price: f64,
    reason:     &str,
    bot_state:  &SharedState,
    weights:    &SharedWeights,
) {
    let mut s = bot_state.write().await;

    let idx = s.positions.iter().position(|p| p.symbol == symbol);
    if let Some(idx) = idx {
        let pos = s.positions.remove(idx);

        let current_value = pos.quantity * exit_price;
        let trade_pnl = if pos.side == "LONG" {
            current_value - pos.size_usd
        } else {
            pos.size_usd - current_value
        };

        s.capital += pos.size_usd + trade_pnl;
        s.pnl     += trade_pnl;

        let pnl_pct    = trade_pnl / pos.size_usd * 100.0;
        let profitable = trade_pnl > 0.0;
        let was_long   = pos.side == "LONG";
        let r_at_close = if pos.r_dollars_risked > 1e-8 { trade_pnl / pos.r_dollars_risked } else { 0.0 };

        info!("📝 CLOSE {} {} @ ${:.4} → {:+.2} ({:+.1}% / {:.2}R) [{}]",
            pos.side, symbol, exit_price, trade_pnl, pnl_pct, r_at_close, reason);

        s.closed_trades.push(ClosedTrade {
            symbol:    symbol.to_string(),
            side:      pos.side.clone(),
            entry:     pos.entry_price,
            exit:      exit_price,
            pnl:       trade_pnl,
            pnl_pct,
            reason:    reason.to_string(),
            closed_at: now_str(),
        });
        let len = s.closed_trades.len();
        if len > 100 { s.closed_trades.drain(0..len - 100); }

        // ── Recalculate all metrics from updated history ──────────────────
        s.metrics = PerformanceMetrics::calculate(&s.closed_trades);
        let m = &s.metrics;
        info!("📈 Metrics → Sharpe:{:.2} Sortino:{:.2} Expect:{:+.1}% PF:{:.2} Kelly:{:.1}% CB:{}",
            m.sharpe, m.sortino, m.expectancy, m.profit_factor,
            if m.kelly_fraction() > 0.0 { m.kelly_fraction() * 100.0 } else { 0.0 },
            if m.in_circuit_breaker() { "ON" } else { "off" });

        // ── Online signal weight learning ─────────────────────────────────
        drop(s);
        let mut w = weights.write().await;
        w.update(&pos.contrib, was_long, profitable);
        info!("🧠 Weights → RSI:{:.2} BB:{:.2} MACD:{:.2} Trend:{:.2} OF:{:.2}",
            w.rsi, w.bollinger, w.macd, w.trend, w.order_flow);
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  HELPERS
// ═══════════════════════════════════════════════════════════════════════════════

async fn set_status(bot_state: &SharedState, msg: &str) {
    let mut s = bot_state.write().await;
    s.status      = msg.to_string();
    s.last_update = now_str();
}

fn now_str() -> String {
    chrono::Utc::now().format("%H:%M:%S UTC").to_string()
}
