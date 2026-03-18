//! TradingBots.fun – Autonomous Cryptocurrency Trading System
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

// ─────────────────────────── Risk constants ───────────────────────────────────
/// Minimum signal confidence for new entries. Below this the trade is skipped.
const MIN_CONFIDENCE: f64 = 0.68;
/// Maximum fraction of equity at risk per individual trade (stop-distance based).
const MAX_TRADE_HEAT: f64 = 0.02;   // 2 %
/// Maximum total fraction of equity at risk across all open positions.
const MAX_PORTFOLIO_HEAT: f64 = 0.08;  // 8 %
/// Maximum number of concurrent open positions.
const MAX_POSITIONS: usize = 8;
/// Maximum open positions in the same direction (long OR short).
const MAX_SAME_DIRECTION: usize = 4;
/// DCA minimum confidence (slightly higher than new-entry minimum).
const DCA_MIN_CONFIDENCE: f64 = 0.72;
/// Circuit-breaker drawdown threshold.  Once peak→current drawdown exceeds
/// this fraction, all new position sizes are scaled down by `CB_SIZE_MULT`.
const CB_DRAWDOWN_THRESHOLD: f64 = 0.08;  // 8 %
/// Position-size multiplier applied when the circuit breaker is active.
const CB_SIZE_MULT: f64 = 0.35;
/// Upper bound for position size as fraction of free capital (Kelly clamp).
const MAX_POSITION_PCT: f64 = 0.18;
/// Lower bound for position size as fraction of free capital.
const MIN_POSITION_PCT: f64 = 0.01;

mod config;
mod data;
mod signal_watchlist;
mod cross_exchange;
mod indicators;
mod signals;
mod risk;
mod exchange;
mod decision;
mod db;
mod learner;
mod metrics;
mod persistence;
mod sentiment;
mod web_dashboard;
mod candlestick_patterns;
mod chart_patterns;
mod ai_reviewer;
mod coins;
mod funding;
mod trade_log;
mod daily_analyst;
mod tenant;
mod ledger;
mod stripe;
mod privy;
mod fund_tracker;
mod funnel;
mod invite;
mod leaderboard;
mod hl_wallet;
mod mailer;
mod correlation;
mod notifier;
mod onchain;

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
use notifier::SharedNotifier;
use onchain::SharedOnchain;
use signal_watchlist::{SharedWatchlist, SignalWatchlist, SkipReason};
use cross_exchange::{CrossExchangeMonitor, SharedCrossExchange};
use trade_log::{SharedTradeLogger, TradeEvent, TradeLogger, ts_now, date_today};
use db::{AumSnapshot, Database, SharedDb};

// ═══════════════════════════════════════════════════════════════════════════════
//  ERROR CLASSIFICATION – human-friendly cycle error messages
// ═══════════════════════════════════════════════════════════════════════════════

/// Classify a cycle error string and return `(sleep_secs, friendly_message)`.
///
/// The message is designed to be calm and actionable for a non-technical user
/// watching the dashboard.  The sleep duration gives the upstream service time
/// to recover before the bot retries.
fn classify_cycle_error(err: &str) -> (u64, String) {
    // Tag injected by data.rs for gateway / server errors
    if err.contains("hl_api_502") || err.contains("502") || err.contains("503") || err.contains("504") {
        return (
            60,
            "🌐 Hyperliquid servers are temporarily busy (502/503). \
             This happens occasionally during high market activity or maintenance. \
             The bot is pausing 60 s and will resume automatically — no action needed."
            .to_string(),
        );
    }
    if err.contains("hl_api_429") || err.contains("429") || err.contains("Too Many Requests") {
        return (
            30,
            "⏱ API rate limit reached. \
             The bot is backing off for 30 s and will slow its request rate. \
             No action needed."
            .to_string(),
        );
    }
    if err.contains("timeout") || err.contains("connection") || err.contains("dns") {
        return (
            20,
            "📡 Network connectivity issue detected. \
             Please check that the VPS has internet access. \
             Retrying in 20 s."
            .to_string(),
        );
    }
    // Generic fallback
    (
        10,
        format!(
            "⚠ Unexpected error — the bot will retry in 10 s. \
             If this keeps happening, restart the service with: \
             `sudo systemctl restart tradingbots-fun`\n\
             Detail: {}",
            err
        ),
    )
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_default_env()
        .format_timestamp_millis()
        .init();

    // ── CLI: --analyze [YYYY-MM-DD] ──────────────────────────────────────
    // Analyse a specific day's log and print the markdown report, then exit.
    // Usage:  cargo run -- --analyze             (yesterday)
    //         cargo run -- --analyze 2026-02-27  (specific date)
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|a| a == "--analyze") {
        // Load the API key from the environment (don't need full config for analysis)
        let api_key_owned = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
        let api_key = api_key_owned.as_str();
        if api_key.is_empty() {
            eprintln!("❌ ANTHROPIC_API_KEY not set — cannot run analysis");
            std::process::exit(1);
        }
        // Date argument is optional; default = yesterday
        let date = args.get(pos + 1)
            .filter(|d| d.len() == 10 && d.chars().nth(4) == Some('-'))
            .cloned()
            .unwrap_or_else(trade_log::date_yesterday);

        let log_path = std::path::PathBuf::from(format!("logs/trading_{}.jsonl", date));
        let out_path = std::path::PathBuf::from(format!("logs/analysis_{}.md", date));
        match daily_analyst::analyse_log_file(&log_path, &out_path, api_key).await {
            Ok(p)  => { println!("✅ Analysis written to {}", p.display()); }
            Err(e) => { eprintln!("❌ Analysis failed: {}", e); std::process::exit(1); }
        }
        return Ok(());
    }

    info!("🤖 TradingBots.fun Starting — Professional Quant Mode");

    let config = config::Config::from_env()?;
    info!("✓ Config: mode={:?}  capital=${:.0}  paper={}", config.mode, config.initial_capital, config.paper_trading);

    // ── PostgreSQL — connect and migrate, gracefully degrade if unavailable ──
    let shared_db: Option<SharedDb> = if config.database_url.starts_with("postgres") {
        match Database::connect(&config.database_url).await {
            Ok(database) => {
                info!("✅ PostgreSQL connected and migrations applied");
                // Spawn hourly maintenance task (prune old snapshots, ANALYZE).
                let db_maint = Arc::new(database);
                let db_for_maint = db_maint.clone();
                tokio::spawn(async move {
                    let mut interval = tokio::time::interval(
                        std::time::Duration::from_secs(3600)
                    );
                    interval.tick().await; // skip the first immediate tick
                    loop {
                        interval.tick().await;
                        if let Err(e) = db_for_maint.run_maintenance().await {
                            log::warn!("DB maintenance failed: {e}");
                        }
                    }
                });
                Some(db_maint)
            }
            Err(e) => {
                warn!("⚠ PostgreSQL unavailable — running without persistence: {e}");
                warn!("  Set DATABASE_URL=postgresql://tradingbots:<pass>@localhost/tradingbots to enable");
                None
            }
        }
    } else {
        info!("ℹ DATABASE_URL is not a PostgreSQL URL — persistence disabled");
        None
    };

    let market = Arc::new(data::MarketClient::new());
    let hl     = Arc::new(exchange::HyperliquidClient::new(&config)?);

    // Daily structured JSONL log (LLM-ingestible, rotates at midnight UTC)
    let trade_logger: SharedTradeLogger = TradeLogger::shared("logs")
        .map_err(|e| anyhow::anyhow!("Failed to create trade logger: {}", e))?;
    info!("✓ Trade logger ready (logs/trading_{}.jsonl)", date_today());

    // LunarCrush sentiment client (pre-warm cache at startup)
    let sentiment_cache: SharedSentiment = SentimentCache::new(config.lunarcrush_api_key.clone());
    sentiment_cache.warm().await;

    // HL funding rate cache via metaAndAssetCtxs (all perps, native symbols, refreshes every 3 min)
    let funding_cache: SharedFunding = FundingCache::new();
    funding_cache.warm().await;

    // On-chain exchange netflow — Coinglass API (graceful no-op if key absent)
    // COINGLASS_API_KEY read directly from env inside OnchainCache::new().
    let onchain_cache: SharedOnchain = onchain::OnchainCache::new();
    onchain_cache.warm().await;

    // Webhook / Telegram notifier — fires on position open/close/CB/AI events.
    // Auto-detected from WEBHOOK_URL and/or TELEGRAM_BOT_TOKEN + TELEGRAM_CHAT_ID.
    let notifier: Option<SharedNotifier> = notifier::Notifier::from_env()
        .map(std::sync::Arc::new);
    if notifier.is_some() {
        info!("✓ Notifier ready (webhook/Telegram configured)");
    } else {
        info!("ℹ️  No WEBHOOK_URL or TELEGRAM_* env set — notifications disabled");
    }

    info!("✓ Clients ready (LunarCrush sentiment + HL funding rates + on-chain active)");

    // Cross-exchange monitor — Binance/ByBit/OKX vs HL price divergence (every 5 min)
    // Fires a tiny bull/bear signal when HL diverges from CEX peers by ≥0.25% for ≥3 cycles.
    // No-op in dev when CEX APIs are unreachable; never blocks trade decisions.
    let cex_monitor: SharedCrossExchange = CrossExchangeMonitor::new();
    cex_monitor.warm().await;

    // Signal watchlist — tracks near-miss SKIPs across cycles for re-evaluation
    let signal_watchlist: SharedWatchlist = SignalWatchlist::new();

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

    // ── Apply config values that are never persisted ──────────────────────
    bot_state.write().await.referral_code = config.referral_code.clone();

    // ── Restore persisted state (positions, P&L, metrics, equity window) ──
    if let Some(snapshot) = persistence::PersistedState::load() {
        snapshot.apply_to(&mut *bot_state.write().await);
        // Always keep initial_capital + referral_code from current config
        bot_state.write().await.initial_capital = config.initial_capital;
        bot_state.write().await.referral_code   = config.referral_code.clone();
        info!("✓ State restored: {} open positions · {} closed trades · capital=${:.2}",
            bot_state.read().await.positions.len(),
            bot_state.read().await.closed_trades.len(),
            bot_state.read().await.capital,
        );
    }

    // Dashboard
    // Shared tenant manager: used by both the dashboard and the midnight
    // leaderboard snapshot task — keep one instance so registrations are
    // visible in both places.
    let tenant_manager = tenant::new_tenant_manager();
    // Transactional mailer — None when RESEND_API_KEY is not set.
    let mailer = mailer::Mailer::new(
        config.email_api_key.as_deref(),
        config.email_from.as_deref(),
    ).map(std::sync::Arc::new);
    {
        let app_state = web_dashboard::AppState {
            bot_state:             bot_state.clone(),
            tenants:               tenant_manager.clone(),
            db:                    shared_db.clone(),
            stripe_api_key:        config.stripe_secret_key.clone(),
            stripe_webhook_secret: config.stripe_webhook_secret.clone(),
            stripe_price_id:       config.stripe_price_id.clone(),
            privy_app_id:             config.privy_app_id.clone(),
            walletconnect_project_id: config.walletconnect_project_id.clone(),
            session_secret:         config.session_secret.clone(),
            jwks_cache:             privy::new_jwks_cache(),
            apple_pay_domain_assoc: config.apple_pay_domain_assoc.clone(),
            admin_password:         config.admin_password.clone(),
            coinzilla_zone_id:      config.coinzilla_zone_id.clone(),
            mailer:                 mailer.clone(),
            stripe_promo_price_id:  config.stripe_promo_price_id.clone(),
        };
        tokio::spawn(async move {
            if let Err(e) = web_dashboard::serve(app_state, 3000).await {
                error!("Dashboard: {}", e);
            }
        });
    }

    // ── Trial-expiry promo email task (hourly) ────────────────────────────────
    // Scans for Free tenants whose 14-day trial has just elapsed and who
    // haven't received the $9.95 intro-offer email yet, then sends one.
    // Runs every hour; each tenant is emailed exactly once (DB idempotency).
    if let (Some(ref db), Some(ref ml)) = (&shared_db, &mailer) {
        let db_promo   = db.clone();
        let ml_promo   = ml.clone();
        let promo_pid  = config.stripe_promo_price_id.clone();
        let site_url   = std::env::var("SITE_URL")
            .unwrap_or_else(|_| "https://tradingbots.fun".to_string());
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(
                std::time::Duration::from_secs(3600)
            );
            interval.tick().await; // skip the first immediate tick (just started)
            loop {
                interval.tick().await;
                match db_promo.fetch_expired_trial_tenants().await {
                    Ok(tenants) => {
                        for (tenant_id, email, name) in tenants {
                            // Build the checkout URL — use the promo price when
                            // configured, otherwise fall back to the standard price.
                            let promo_flag = if promo_pid.is_some() { "&promo=1" } else { "" };
                            let checkout_url = format!(
                                "{}/billing/checkout?tenant_id={}{}",
                                site_url, tenant_id, promo_flag
                            );
                            let html = mailer::Mailer::trial_expiry_html(&name, &checkout_url);
                            let subject = "Your trial ended — try Pro for $9.95 this month";
                            match ml_promo.send(&email, subject, &html).await {
                                Ok(_) => {
                                    info!("📧 Promo email sent → {}", email);
                                    if let Err(e) = db_promo.mark_promo_sent(&tenant_id).await {
                                        log::warn!("mark_promo_sent failed for {tenant_id}: {e}");
                                    }
                                }
                                Err(e) => {
                                    log::warn!("Promo email failed for {email}: {e}");
                                }
                            }
                        }
                    }
                    Err(e) => log::warn!("fetch_expired_trial_tenants: {e}"),
                }
            }
        });
    } else if mailer.is_none() {
        info!("ℹ️  RESEND_API_KEY not set — trial promo emails disabled");
    }

    // Ctrl-C handler — saves state before exiting
    let running = Arc::new(RwLock::new(true));
    {
        let r = running.clone();
        let bs_shutdown = bot_state.clone();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            info!("Shutdown signal — saving state…");
            persistence::save_snapshot(&bs_shutdown).await;
            info!("💾 State saved on shutdown");
            *r.write().await = false;
        });
    }

    let mut prev_mids: HashMap<String, f64> = HashMap::new();
    // Track last analysis date to trigger exactly once per midnight
    let mut last_analysis_date = date_today();
    // Set start capital once at boot
    { trade_logger.lock().await.day_stats.start_capital = config.initial_capital; }

    info!("🚀 Main loop started (30 s cycle, paper={})", config.paper_trading);

    loop {
        if !*running.read().await { break; }

        // ── Midnight analysis trigger ─────────────────────────────────────
        // Check once per cycle if the date has rolled over.  If so, run the
        // daily analysis for yesterday's log in the background, and write
        // leaderboard snapshots for every active tenant.
        let today_now = date_today();
        if today_now != last_analysis_date {
            last_analysis_date = today_now.clone();
            if let Some(api_key) = config.anthropic_api_key.clone() {
                let logger_clone = trade_logger.clone();
                tokio::spawn(async move {
                    info!("🌙 Midnight — running daily analysis for yesterday…");
                    daily_analyst::analyse_day(&logger_clone, &api_key).await;
                });
            }
            // Write leaderboard snapshots for the active campaign
            if let Some(ref db) = shared_db {
                let db_snap   = db.clone();
                let tm_snap   = tenant_manager.clone();
                tokio::spawn(async move {
                    match leaderboard::snapshot_daily(&db_snap, &tm_snap).await {
                        Ok(n)  => info!("📊 Leaderboard: wrote {} daily snapshots", n),
                        Err(e) => log::warn!("Leaderboard snapshot failed: {e}"),
                    }
                });
            }
        }

        set_status(&bot_state, "📡 Fetching prices…").await;

        match run_cycle(&config, &market, &hl, &shared_db, &bot_state, &weights,
                        &sentiment_cache, &funding_cache, &mut prev_mids, &btc_dominance,
                        &trade_logger, config.builder_fee_bps,
                        &notifier, &onchain_cache, &signal_watchlist, &cex_monitor).await
        {
            Ok(_) => {
                // Persist state every N cycles (cheap; atomic rename)
                let cycle_n = bot_state.read().await.cycle_count;
                if cycle_n % persistence::SAVE_EVERY_N_CYCLES == 0 {
                    persistence::save_snapshot(&bot_state).await;
                }
                set_status(&bot_state, "⏳ Waiting for next cycle (30s)…").await;
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            }
            Err(e) => {
                let err_str = e.to_string();
                let (sleep_secs, friendly_msg) = classify_cycle_error(&err_str);
                error!("Cycle error: {err_str}");
                warn!("{friendly_msg}");
                set_status(&bot_state, &friendly_msg).await;
                tokio::time::sleep(std::time::Duration::from_secs(sleep_secs)).await;
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

/// Execute one 30-second trading cycle.
///
/// Sequence of operations:
///   1. Tier-1 price fetch — single Hyperliquid `allMids` call (all perps).
///   2. Update peak equity; increment `cycles_held` on open positions.
///   3. Position management — trailing stops, R-multiple partials, time exits.
///   4. Session price snapshot update; candidate list pushed to dashboard.
///   5. Refresh BTC dominance every 20 cycles.
///   6. Compute BTC 24h / 4h returns for cross-asset context.
///   7. Tier-2 analysis — fetch candles + order book for each candidate and
///      call `analyse_symbol()` which emits entry or skip decisions.
///   8. Optional Claude AI position review every 10 cycles.
#[allow(clippy::too_many_arguments)]
async fn run_cycle(
    config:          &config::Config,
    market:          &Arc<data::MarketClient>,
    hl:              &Arc<exchange::HyperliquidClient>,
    db:              &Option<SharedDb>,
    bot_state:       &SharedState,
    weights:         &SharedWeights,
    sent_cache:      &SharedSentiment,
    fund_cache:      &SharedFunding,
    prev_mids:       &mut HashMap<String, f64>,
    btc_dominance:   &SharedBtcDominance,
    trade_logger:    &SharedTradeLogger,
    fee_bps:         u32,                       // builder fee bps
    notifier:         &Option<SharedNotifier>,   // webhook / Telegram alerts
    onchain_cache:    &SharedOnchain,            // exchange netflow signal
    signal_watchlist: &SharedWatchlist,          // near-miss SKIP re-evaluator
    cex_monitor:      &SharedCrossExchange,      // cross-exchange price divergence
) -> Result<()> {
    { bot_state.write().await.cycle_count += 1; }

    // ── Tier 1: all prices in one Hyperliquid call ────────────────────────
    set_status(bot_state, "📡 Hyperliquid allMids…").await;
    let current_mids = market.fetch_all_mids().await
        .map_err(|e| { warn!("allMids: {}", e); e })?;
    info!("📊 {} prices", current_mids.len());

    // Log cycle start (used by daily analyst for context)
    {
        let s = bot_state.read().await;
        let _equity = s.capital + s.positions.iter().map(|p| p.size_usd + p.unrealised_pnl).sum::<f64>();
        trade_logger.lock().await.log(&TradeEvent::CycleStart {
            ts:             ts_now(),
            cycle_number:   s.cycle_count,
            open_positions: s.positions.len(),
            free_capital:   s.capital,
            peak_equity:    s.peak_equity,
            btc_dom_pct:    0.0, // updated later in the cycle
            btc_ret_24h:    0.0,
            btc_ret_4h:     0.0,
            candidate_count: 0,
        });
    }

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

        // All-time peak (for display)
        if equity > s.peak_equity { s.peak_equity = equity; }

        // Rolling 7-day window for circuit breaker (prevents one lucky spike
        // from permanently throttling position sizes months later)
        const SEVEN_DAYS_SECS: i64 = 7 * 24 * 3600;
        let now_ts = chrono::Utc::now().timestamp();
        s.equity_window.push_back((now_ts, equity));
        // Trim entries older than 7 days
        while s.equity_window.front().map(|&(ts, _)| now_ts - ts > SEVEN_DAYS_SECS).unwrap_or(false) {
            s.equity_window.pop_front();
        }

        // Sparkline history — one point per cycle, capped at ~2.4 h (288 × 30 s)
        s.equity_history.push(equity);
        if s.equity_history.len() > 288 { s.equity_history.remove(0); }

        for pos in s.positions.iter_mut() { pos.cycles_held += 1; }

        // ── PostgreSQL: equity snapshot ───────────────────────────────────
        // Persist one equity data point per cycle. Tenant ID is "operator"
        // in single-op mode; will be per-tenant in multi-tenant phase.
        // Fire-and-forget — a DB write failure must never crash the cycle.
        if let Some(db) = db {
            let equity_copy = equity;
            let db_clone    = db.clone();
            tokio::spawn(async move {
                if let Err(e) = db_clone.insert_equity_snapshot("00000000-0000-0000-0000-000000000001", equity_copy).await {
                    log::debug!("equity_snapshot write skipped: {e}");
                }
            });
        }

        // ── PostgreSQL: AUM snapshot (pre-aggregated for admin + landing page) ──
        // In single-operator mode total_aum == operator equity.
        // In multi-tenant mode this will sum across all tenants.
        let initial_capital = s.initial_capital;
        let open_positions  = s.positions.len() as i32;
        // Today's trade stats come from the trade_logger day_stats (not ClosedTrade
        // strings which only carry HH:MM:SS without a date).  For now we emit
        // zeroes; the DB can compute accurate daily stats via a SQL query.
        let closed_today: i32    = 0;
        let win_rate_today: Option<f64> = None;

        if let Some(db) = db {
            let total_pnl = equity - initial_capital;
            let pnl_pct   = if initial_capital > 0.0 { total_pnl / initial_capital * 100.0 } else { 0.0 };
            let snap = AumSnapshot {
                total_aum:            equity,
                deposited_capital:    initial_capital,
                total_pnl,
                pnl_pct,
                active_tenant_count:  if open_positions > 0 { 1 } else { 0 },
                total_tenant_count:   1,
                open_position_count:  open_positions,
                total_trades_today:   closed_today,
                win_rate_today,
            };
            let db_clone = db.clone();
            tokio::spawn(async move {
                if let Err(e) = db_clone.insert_aum_snapshot(&snap).await {
                    log::debug!("aum_snapshot write skipped: {e}");
                }
            });
        }
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
        take_partial(sym, price, 1, bot_state, weights, trade_logger).await;
    }
    for (sym, price) in to_partial2 {
        if to_close.iter().any(|(s, _, _)| s == &sym) { continue; }
        take_partial(sym, price, 2, bot_state, weights, trade_logger).await;
    }

    // Execute full closes
    for (sym, price, reason) in to_close {
        close_paper_position(&sym, price, &reason, bot_state, weights, trade_logger, notifier).await;
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
    // Build CandidateInfo and push to state under a single lock.
    // rsi/regime/atr_pct are filled in later by the per-symbol analysis loop.
    {
        let session_snap = bot_state.read().await.session_prices.clone();
        let cand_infos: Vec<CandidateInfo> = candidates.iter().filter_map(|sym| {
            let &price = current_mids.get(sym.as_str())?;
            // cycle 1: prev_snapshot is empty (no previous reference price yet) → show "—"
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
            Some(CandidateInfo {
                symbol:     sym.clone(),
                price,
                change_pct,
                rsi:        None,
                regime:     None,
                atr_pct:    None,
                confidence: None,
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

    // BTC 24h return: compare first vs last candle across the 1h window (50 × 1h ≈ 50h).
    // BTC 4h return: first vs last candle in the 4h window (used for relative perf signal).
    // Both fetched same-cycle so always fresh.
    let (btc_ret_24h, btc_ret_4h): (f64, f64) = {
        let candles_1h = market.fetch_market_data("BTC").await;
        let candles_4h = market.fetch_market_data_4h("BTC").await;
        let ret_24h = match &candles_1h {
            Ok(c) if c.len() >= 2 => {
                let f = c.first().unwrap().close;
                let l = c.last().unwrap().close;
                if f > 0.0 { (l - f) / f * 100.0 } else { 0.0 }
            }
            _ => 0.0,
        };
        let ret_4h = match &candles_4h {
            Ok(c) if c.len() >= 2 => {
                let f = c.first().unwrap().close;
                let l = c.last().unwrap().close;
                if f > 0.0 { (l - f) / f * 100.0 } else { 0.0 }
            }
            _ => 0.0,
        };
        (ret_24h, ret_4h)
    };

    let btc_dom = *btc_dominance.read().await;
    if btc_ret_24h != 0.0 || btc_ret_4h != 0.0 {
        info!("🟠 BTC ctx: dom={:.1}%  24h={:+.2}%  4h={:+.2}%", btc_dom, btc_ret_24h, btc_ret_4h);
    } else {
        warn!("BTC candles unavailable — dominance filter disabled this cycle");
    }

    // ── Tier 2: analyse candidates ────────────────────────────────────────
    let total = candidates.len();
    let mut new_decisions: Vec<DecisionInfo> = Vec::new();
    // Collect per-symbol indicator snapshots to batch-update CandidateInfo at end of cycle.
    let mut cand_indicators: Vec<(String, f64, &'static str, f64, f64)> = Vec::new(); // (sym, rsi, regime, atr_pct, confidence)

    for (i, sym) in candidates.iter().enumerate() {
        set_status(bot_state,
            &format!("🔬 Analysing {}/{}: {}…", i + 1, total, sym)).await;

        match analyse_symbol(sym, market, hl, db, config, bot_state, weights, sent_cache, fund_cache, btc_dom, btc_ret_24h, btc_ret_4h, trade_logger, fee_bps, notifier, onchain_cache, cex_monitor).await {
            Ok(Some((dec, ind))) => {
                let price = ind.close_price;

                if dec.action != "SKIP" {
                    info!("💡 {} → {} conf={:.0}%", sym, dec.action, dec.confidence * 100.0);
                    // Trade fired — remove from watchlist (entry taken or opportunity resolved).
                    signal_watchlist.remove(sym).await;
                } else {
                    // ── Watchlist: re-evaluate if symbol was already being watched ──
                    // Use skipped_direction (the lean even on SKIP) so "SKIP with BUY lean"
                    // is correctly evaluated as "still pointing BUY".
                    let reeval_action = if dec.skipped_direction != "NONE" {
                        &dec.skipped_direction
                    } else {
                        &dec.action
                    };
                    signal_watchlist.re_evaluate(
                        sym, reeval_action, dec.confidence, price,
                    ).await;

                    // ── Watchlist: enqueue near-miss SKIPs ────────────────────────
                    // Distinguish between a gated skip and a low-confidence skip.
                    let skip_reason = if dec.rationale.contains("Funding gate")
                        || dec.rationale.contains("circuit breaker")
                    {
                        SkipReason::Gated(
                            dec.rationale.chars().take(60).collect()
                        )
                    } else {
                        SkipReason::LowConfidence
                    };
                    signal_watchlist.maybe_watch(
                        sym, &dec.skipped_direction, dec.confidence, price, skip_reason,
                    ).await;
                }

                cand_indicators.push((sym.clone(), ind.rsi, ind.regime, ind.atr_pct, dec.confidence));
                // Push ALL decisions (including SKIPs) so the signal feed always shows activity.
                // SKIPs are rendered dimmed in the dashboard; BUY/SELL get the coloured treatment.
                new_decisions.push(DecisionInfo {
                    symbol:      sym.clone(),
                    action:      dec.action.clone(),
                    confidence:  dec.confidence,
                    entry_price: dec.entry_price,
                    rationale:   dec.rationale.clone(),
                    timestamp:   now_str(),
                });
            }
            Ok(None) => {}
            Err(e)   => warn!("  {} error: {}", sym, e),
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }

    // Evict expired watchlist entries once per cycle (keeps the map tidy).
    signal_watchlist.gc().await;

    {
        let mut s = bot_state.write().await;
        // Write RSI / regime / ATR% back into the candidate list for the dashboard.
        for (sym, rsi, regime, atr_pct, conf) in &cand_indicators {
            if let Some(c) = s.candidates.iter_mut().find(|c| c.symbol == *sym) {
                c.rsi        = Some(*rsi);
                c.regime     = Some(regime.to_string());
                c.atr_pct    = Some(*atr_pct);
                c.confidence = Some(*conf);
            }
        }
        s.recent_decisions.extend(new_decisions);
        // Keep at most 100 entries (20 decisions × 5 cycles of history).
        // The dashboard shows the 20 most recent, so older ones are just trimmed.
        let len = s.recent_decisions.len();
        if len > 100 { s.recent_decisions.drain(0..len - 100); }
        // Record when the next cycle will fire so the dashboard can show a real countdown.
        s.next_cycle_at = chrono::Utc::now().timestamp_millis() + 30_000;
    }

    { let mut s = bot_state.write().await; s.last_update = now_str(); }

    // ── Metrics snapshot (every 10 cycles) ───────────────────────────────
    let (cycle_count, open_count) = {
        let s = bot_state.read().await;
        (s.cycle_count, s.positions.len())
    };
    if cycle_count % 10 == 0 {
        let s = bot_state.read().await;
        let m = &s.metrics;
        trade_logger.lock().await.log(&TradeEvent::MetricsSnapshot {
            ts:               ts_now(),
            cycle_number:     s.cycle_count,
            total_trades:     s.closed_trades.len(),
            win_rate:         m.win_rate,
            expectancy_pct:   m.expectancy,
            sharpe:           m.sharpe,
            sortino:          m.sortino,
            max_drawdown_pct: m.max_drawdown * 100.0,
            profit_factor:    m.profit_factor,
            kelly_fraction:   m.kelly_fraction(),
            total_pnl_usd:    s.pnl,
            capital:          s.capital,
            open_positions:   s.positions.len(),
        });
    }

    // ── AI position review (every 10 cycles ≈ 5 minutes) ─────────────────
    if cycle_count % 10 == 0 && open_count > 0 {
        if let Some(api_key) = &config.anthropic_api_key {
            set_status(bot_state, "🤖 Claude AI reviewing positions…").await;
            // Surface AI activity in the dashboard before the (slow) API call.
            {
                let mut s = bot_state.write().await;
                s.ai_status = format!("🤖 Querying Claude for {} open position(s)…", open_count);
            }
            let (positions_snap, metrics_snap, capital_snap) = {
                let s = bot_state.read().await;
                (s.positions.clone(), s.metrics.clone(), s.capital)
            };
            let review = ai_reviewer::review_positions(
                &positions_snap, &metrics_snap, capital_snap, api_key,
            ).await;
            // Update ai_status with a summary of what Claude recommended.
            {
                let now = chrono::Utc::now().format("%H:%M UTC").to_string();
                let summary = if review.recommendations.is_empty() {
                    format!("🤖 AI reviewed {} position(s) — all HOLD · {}", open_count, now)
                } else {
                    let actions: Vec<String> = review.recommendations.iter()
                        .map(|r| format!("{} → {}", r.symbol, r.action.replace('_', " ").to_uppercase()))
                        .collect();
                    format!("🤖 {} · {} · {}", open_count, actions.join(" · "), now)
                };
                let mut s = bot_state.write().await;
                s.ai_status = summary;
            }
            apply_ai_review(&review, bot_state, weights, &current_mids, trade_logger, notifier).await;
        }
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════════
//  AI REVIEW APPLICATION
// ═══════════════════════════════════════════════════════════════════════════════

async fn apply_ai_review(
    review:       &ai_reviewer::AiReview,
    bot_state:    &SharedState,
    weights:      &SharedWeights,
    current_mids: &HashMap<String, f64>,
    trade_logger: &SharedTradeLogger,
    notifier:     &Option<SharedNotifier>,
) {
    for rec in &review.recommendations {
        let cur_price = match current_mids.get(rec.symbol.as_str()) {
            Some(&p) => p,
            None     => continue,
        };

        match rec.action.as_str() {
            "close_now" => {
                // Hard guardrails before trusting AI close_now.  The AI is
                // given a single snapshot every 5 min with no trajectory context,
                // so it can fire prematurely on normal early drawdown or noise.
                //
                // All three conditions must hold:
                //   1. Min hold 45 min (90 cycles) — crypto regularly pulls -0.4R
                //      in the first 15–30 min before reversing; don't let AI cut
                //      those trades short.
                //   2. Position is genuinely losing (R < -0.25) — prevents AI from
                //      closing breakeven or mildly profitable trades on "signal
                //      failed" noise.
                //   3. Either DCA slots are exhausted (dca_count ≥ 2, so there is
                //      no remaining rescue mechanism) OR loss is deep (R < -0.50).
                //      If DCA is still available the strategy should try it first.
                let should_close = {
                    let s = bot_state.read().await;
                    s.positions.iter().find(|p| p.symbol == rec.symbol)
                        .map(|p| {
                            let r = if p.r_dollars_risked > 1e-8 {
                                p.unrealised_pnl / p.r_dollars_risked
                            } else { 0.0 };
                            let min_hold_met     = p.cycles_held >= 90; // 45 min
                            let genuinely_losing = r < -0.25;
                            let dca_exhausted    = p.dca_count >= 2;    // no more adds available
                            let deep_loss        = r < -0.50;           // beyond DCA rescue zone
                            min_hold_met && genuinely_losing && (dca_exhausted || deep_loss)
                        })
                        .unwrap_or(false)
                };
                if should_close {
                    info!("🤖 AI close: {} — {}", rec.symbol, rec.reason);
                    // Snapshot r_mult before the close for the ai_action notification
                    let r_for_notify = {
                        let s = bot_state.read().await;
                        s.positions.iter().find(|p| p.symbol == rec.symbol)
                            .map(|p| if p.r_dollars_risked > 1e-8 {
                                p.unrealised_pnl / p.r_dollars_risked
                            } else { 0.0 })
                            .unwrap_or(0.0)
                    };
                    close_paper_position(&rec.symbol, cur_price, "AI-Close", bot_state, weights, trade_logger, notifier).await;
                    // Fire ai_action notification
                    if let Some(n) = notifier {
                        let n = n.clone();
                        let sym = rec.symbol.clone();
                        let reason = rec.reason.chars().take(120).collect::<String>();
                        tokio::spawn(async move {
                            n.ai_action(&sym, "close_now", &reason, r_for_notify).await;
                        });
                    }
                } else {
                    info!("🤖 AI close {} SKIPPED — guardrail (need 45min hold + R<-0.25 + DCA exhausted/deep loss)", rec.symbol);
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
                            let add_qty   = add_usd * lev / cur_price;
                            let old_qty   = s.positions[idx].quantity;
                            let old_entry = s.positions[idx].entry_price;
                            let old_stop  = s.positions[idx].stop_loss;
                            let new_qty   = old_qty + add_qty;
                            // Update weighted average entry so unrealised_pnl stays correct
                            let avg_entry = (old_entry * old_qty + cur_price * add_qty) / new_qty;
                            s.capital                        -= add_usd;
                            s.positions[idx].quantity         = new_qty;
                            s.positions[idx].size_usd        += add_usd;
                            s.positions[idx].entry_price      = avg_entry;
                            // FIX: update r_dollars_risked to reflect larger position.
                            // Without this the AI sees an inflated (falsely negative) R-multiple
                            // on the next review cycle, increasing the risk of a premature close.
                            s.positions[idx].r_dollars_risked =
                                (avg_entry - old_stop).abs() * new_qty;
                            info!("🤖 AI scale-up {} ×{:.2}  +${:.2} — {}", rec.symbol, factor, add_usd, rec.reason);
                        }
                    }
                } else {
                    info!("🤖 AI scale-up {} REJECTED — R < 0.5 (guardrail)", rec.symbol);
                }
            }

            "scale_down" => {
                // Guardrail: keep at least 25% AND enforce minimum hold time.
                // Early scale_down on a young position sabotages both the DCA
                // rescue path and the R-multiple exit tranches.
                let keep_frac = rec.factor.clamp(0.25, 0.99);
                let close_frac = 1.0 - keep_frac;
                let (close_usd, close_qty) = {
                    let s = bot_state.read().await;
                    s.positions.iter().find(|p| p.symbol == rec.symbol)
                        .filter(|p| p.cycles_held >= 60) // 30 min minimum before any scale_down
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

            _ => {
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

/// Analyse a single symbol and optionally execute a paper (or live) trade.
///
/// Steps:
///   1. Fetch 50 × 1h candles from Binance; bail early if < 26.
///   2. Fetch 50 × 4h candles for multi-timeframe confirmation (non-fatal if
///      unavailable — HTF scaling is skipped).
///   3. Compute `TechnicalIndicators`, `OrderFlow`, sentiment and funding data.
///   4. Build `BtcMarketContext` (skipped for BTC itself).
///   5. Call `decision::make_decision()` to get a `Decision`.
///   6. If paper mode and action ≠ SKIP → `execute_paper_trade()`.
///      If live mode → `risk::should_trade()` gate → `hl.place_order()`.
///
/// Indicator snapshot returned alongside a Decision so the candidates table
/// can display live RSI / regime / ATR% without a second lock cycle.
struct SymbolIndicators {
    rsi:         f64,
    regime:      &'static str,   // "Trending" | "Neutral" | "Ranging"
    atr_pct:     f64,            // ATR(14) as % of price
    close_price: f64,            // last candle close — used by signal watchlist
}

/// Returns `Ok(Some((Decision, SymbolIndicators)))` even for SKIP decisions so
/// the dashboard can show them; returns `Ok(None)` when candle data is insufficient.
#[allow(clippy::too_many_arguments)]
async fn analyse_symbol(
    symbol:       &str,
    market:       &Arc<data::MarketClient>,
    hl:           &Arc<exchange::HyperliquidClient>,
    _db:          &Option<SharedDb>,
    config:       &config::Config,
    bot_state:    &SharedState,
    weights:      &SharedWeights,
    sent_cache:   &SharedSentiment,
    fund_cache:   &SharedFunding,
    btc_dom:      f64,   // BTC dominance %
    btc_ret_24h:  f64,   // BTC 24h return %
    btc_ret_4h:   f64,   // BTC 4h return % (for relative performance signal)
    trade_logger: &SharedTradeLogger,
    fee_bps:      u32,   // builder fee bps for this tenant (1 = Pro, 3 = Free)
    notifier:     &Option<SharedNotifier>,   // webhook / Telegram notifier
    onchain_cache: &SharedOnchain,           // exchange netflow signal
    cex_monitor:   &SharedCrossExchange,     // cross-exchange price divergence
) -> Result<Option<(decision::Decision, SymbolIndicators)>> {
    let candles = market.fetch_market_data(symbol).await?;
    if candles.len() < 26 { return Ok(None); }

    // Fetch 4h candles for multi-timeframe confirmation and relative performance.
    // Non-fatal: if unavailable, HTF filter is skipped (scale = 1.0).
    let (htf, asset_return_4h) = match market.fetch_market_data_4h(symbol).await {
        Ok(c4h) if c4h.len() >= 26 => {
            let htf_ind = indicators::calculate_htf(&c4h);
            let ret = if c4h.len() >= 2 {
                let f = c4h.first().unwrap().close;
                let l = c4h.last().unwrap().close;
                if f > 0.0 { (l - f) / f * 100.0 } else { 0.0 }
            } else { 0.0 };
            (Some(htf_ind), ret)
        }
        _ => (None, 0.0),
    };

    let ind  = indicators::calculate_all(&candles)?;
    let ob   = market.fetch_order_book(symbol).await?;
    let of   = signals::detect_order_flow(&ob)?;
    let w    = weights.read().await.clone();
    let sent = sent_cache.get(symbol).await;
    let fund = fund_cache.get(symbol).await;

    // Cross-exchange divergence signal: compare HL mid vs Binance/ByBit/OKX.
    // `current_price` is the HL allMids price already fetched in the outer loop.
    let current_price = candles.last().map(|c| c.close).unwrap_or(0.0);
    let cex_sig = cex_monitor.evaluate(symbol, current_price).await;
    // Advance persistence counter regardless of whether signal is active.
    if let Some(ref s) = cex_sig {
        cex_monitor.tick_persistence(symbol, s.hl_premium_pct).await;
    }

    // BTC dominance context not applied to BTC itself (no self-reference).
    let ctx = if symbol == "BTC" {
        None
    } else {
        Some(decision::BtcMarketContext {
            dominance:       btc_dom,
            btc_return_24h:  btc_ret_24h,
            btc_return_4h:   btc_ret_4h,
            asset_return_4h,
        })
    };

    let mut dec = decision::make_decision(
        &candles, &ind, &of, &w,
        sent.as_ref(), fund.as_ref(),
        ctx.as_ref(), htf.as_ref(),
        cex_sig.as_ref(),
    )?;

    // ── On-chain exchange netflow signal ──────────────────────────────────
    // Coinglass netflow: net USD flowing INTO exchanges = selling pressure (bearish).
    // Flowing OUT = accumulation (bullish).  signal_strength() returns [-1, +1].
    // We apply up to ±4% confidence adjustment on active (non-SKIP) decisions.
    // Max impact is intentionally small — this is a supplementary signal, not primary.
    if dec.action != "SKIP" {
        // get() always returns OnchainData (neutral 0.0 if key absent or symbol unknown).
        let oc_strength = onchain_cache.get(symbol).await.signal_strength();
        if oc_strength.abs() > 0.05 {
            // Aligned = netflow confirms trade direction → boost; opposed → penalty
            let aligned = (dec.action == "BUY"  && oc_strength > 0.0)
                       || (dec.action == "SELL" && oc_strength < 0.0);
            let adj = oc_strength.abs() * 0.04 * if aligned { 1.0 } else { -1.0 };
            dec.confidence = (dec.confidence + adj).clamp(0.0, 1.0);
            log::debug!("{}: on-chain adj {:+.3} ({}) → conf={:.0}%",
                symbol, adj,
                if aligned { "aligned" } else { "opposed" },
                dec.confidence * 100.0);
        }
    }

    // ── Funding-staleness gate (new entries only) ─────────────────────────
    // If the funding cache hasn't refreshed in >10 minutes we have no reliable
    // crowd-positioning data.  Block new entries (BUY/SELL) so we don't open
    // positions blind.  Position management (exits, trailing stops) already
    // ran earlier in run_cycle and is unaffected — this only gates new orders.
    if dec.action != "SKIP" && fund_cache.is_stale().await {
        let age = fund_cache.age_secs().await
            .map(|s| format!("{}s ago", s))
            .unwrap_or_else(|| "never fetched".to_string());
        log::warn!(
            "⚠️  {} → {} gated: funding data stale ({}) — skipping new entry",
            symbol, dec.action, age
        );
        dec.action     = "SKIP".to_string();
        dec.rationale  = format!(
            "Funding gate: crowd-positioning data is stale (last refresh: {}). \
             New entries blocked until HL funding cache refreshes.",
            age
        );
        dec.confidence = 0.0;
    }

    log::debug!("{}: RSI={:.1} trend={:.2}% MACD={:.5} ATR={:.4} → {}",
        symbol, ind.rsi, ind.trend, ind.macd, ind.atr, dec.action);

    // ── Log every decision (including SKIP) for daily analysis ───────────
    let htf_ref = htf.as_ref();
    let regime_str = if ind.adx > 27.0 { "Trending" }
                     else if ind.adx >= 19.0 { "Neutral" }
                     else { "Ranging" };
    {
        let skip_reason = if dec.action == "SKIP" {
            Some(dec.rationale.chars().take(120).collect::<String>())
        } else { None };
        trade_logger.lock().await.log(&TradeEvent::Decision {
            ts:             ts_now(),
            symbol:         symbol.to_string(),
            action:         dec.action.clone(),
            confidence:     dec.confidence,
            rationale:      dec.rationale.chars().take(200).collect(),
            rsi:            ind.rsi,
            rsi_4h:         htf_ref.map_or(50.0, |h| h.rsi_4h),
            adx:            ind.adx,
            regime:         regime_str.to_string(),
            macd:           ind.macd,
            macd_hist:      ind.macd_histogram,
            z_score:        ind.z_score,
            z_score_4h:     htf_ref.map_or(0.0, |h| h.z_score_4h),
            ema_cross_pct:  ind.ema_cross_pct,
            atr:            ind.atr,
            atr_expansion:  ind.atr_expansion_ratio,
            bb_width_pct:   ind.bb_width_pct,
            volume_ratio:   ind.volume_ratio,
            vwap_pct:       ind.vwap_pct,
            sentiment_galaxy: sent.as_ref().map(|s| s.galaxy_score),
            sentiment_bull:   sent.as_ref().map(|s| s.bullish_percent),
            funding_rate:     fund.as_ref().map(|f| f.funding_rate),
            funding_delta:    fund.as_ref().map(|f| f.funding_delta),
            btc_dom_pct:    btc_dom,
            asset_ret_4h:   asset_return_4h,
            entry_price:    dec.entry_price,
            stop_loss:      dec.stop_loss,
            take_profit:    dec.take_profit,
            leverage:       dec.leverage,
            skip_reason,
        });
    }

    if config.paper_trading && dec.action != "SKIP" {
        execute_paper_trade(symbol, &dec, &ind, bot_state, weights, trade_logger, notifier).await;
    } else if !config.paper_trading && dec.action != "SKIP" {
        let account = hl.get_account().await?;
        if risk::should_trade(&dec, &account)? {
            let capital = bot_state.read().await.capital;
            match hl.place_order(symbol, &dec, capital, fee_bps).await {
                Ok(id) => { info!("✅ {} {} @ ${:.4} [{}]", dec.action, symbol, dec.entry_price, id); }
                Err(e) => error!("❌ Order failed {}: {}", symbol, e),
            }
        }
    }

    let current_price = candles.last().map_or(1.0, |c| c.close);
    let ind_snapshot = SymbolIndicators {
        rsi:         ind.rsi,
        regime:      regime_str,
        atr_pct:     if ind.atr > 0.0 && current_price > 0.0 { ind.atr / current_price * 100.0 } else { 0.0 },
        close_price: current_price,
    };

    Ok(Some((dec, ind_snapshot)))
}

// ═══════════════════════════════════════════════════════════════════════════════
//  POSITION SIZING  (Kelly + Sharpe multiplier + confidence)
// ═══════════════════════════════════════════════════════════════════════════════

/// Returns target fraction of free capital for this trade.
///
/// Priority order:
///   1. If half-Kelly available (≥5 trades): Kelly × confidence_scale × Sharpe_mult
///   2. Fallback confidence tiers × Sharpe_mult
///
/// The circuit-breaker multiplier (`CB_SIZE_MULT = 0.35`) is applied here when
/// the peak→current equity drawdown exceeds `CB_DRAWDOWN_THRESHOLD` (8%).
///
/// Result is clamped to [`MIN_POSITION_PCT`, `MAX_POSITION_PCT`].
fn position_size_pct(confidence: f64, metrics: &PerformanceMetrics, in_circuit_breaker: bool) -> f64 {
    let sharpe_mult = metrics.size_multiplier();
    let kelly       = metrics.kelly_fraction();

    let base = if kelly > 0.0 {
        // Linear scale: conf=MIN_CONFIDENCE → 60% of Kelly, conf=1.0 → 100% of Kelly.
        let slope = 0.4 / (1.0 - MIN_CONFIDENCE);
        let conf_scale = (0.6 + (confidence - MIN_CONFIDENCE).max(0.0) * slope).min(1.0);
        kelly * conf_scale
    } else {
        // Pre-Kelly fallback tiers (first ~5 trades)
        match confidence {
            c if c >= 0.85 => 0.08,
            c if c >= 0.75 => 0.06,
            c if c >= MIN_CONFIDENCE => 0.04,
            _              => 0.03,
        }
    };

    let cb_mult = if in_circuit_breaker { CB_SIZE_MULT } else { 1.0 };
    (base * sharpe_mult * cb_mult).clamp(MIN_POSITION_PCT, MAX_POSITION_PCT)
}

/// Fraction of equity at risk for this specific trade (stop-loss based).
///
/// Formula: stop_distance% × notional_size / equity
///   = stop_dist_pct × (size_usd × leverage) / equity
///
/// `size_usd` is the MARGIN committed, not the notional.  Without the
/// leverage factor the heat check underestimates actual risk by the full
/// leverage multiple (e.g. 3× leverage → 3× more equity at risk than the
/// naive formula implies).
fn trade_heat(entry: f64, stop: f64, size_usd: f64, equity: f64, leverage: f64) -> f64 {
    if equity < 1.0 { return 1.0; }
    let stop_dist_pct = (entry - stop).abs() / entry.max(1e-8);
    stop_dist_pct * size_usd * leverage / equity
}

/// Total % of equity at risk across all open positions.
///
/// Risk per position is computed from the **current** stop-loss, not the
/// stale `r_dollars_risked` field (which never updates when the trailing
/// stop advances).  Once a trailing stop has moved to or beyond the entry
/// price the position has a locked-in profit floor — its remaining
/// downside is zero, so it contributes 0% heat.
///
/// Formula per position: max(0, |entry − current_stop| × quantity) / equity
fn portfolio_heat(positions: &[PaperPosition], equity: f64) -> f64 {
    if equity < 1.0 { return 1.0; }
    positions.iter()
        .map(|p| {
            // Dollars currently at risk if stop is hit.
            // Positive  = stop is below entry (LONG) or above entry (SHORT).
            // Zero/neg  = stop has crossed entry → position is protected.
            let current_risk = if p.side == "LONG" {
                (p.entry_price - p.stop_loss) * p.quantity
            } else {
                (p.stop_loss - p.entry_price) * p.quantity
            };
            current_risk.max(0.0) / equity
        })
        .sum::<f64>()
}

// ═══════════════════════════════════════════════════════════════════════════════
//  ENTRY EXECUTION
// ═══════════════════════════════════════════════════════════════════════════════

/// Evaluate and execute a new paper-trade entry (or pyramid / DCA on existing).
///
/// Decision tree for an existing position on `symbol`:
///   - **Same side, profitable (≥ 1R)**: pyramid (+50% of current size).
///   - **Same side, mild loss (-0.15R to -0.85R) + confidence ≥ DCA_MIN_CONFIDENCE**:
///     DCA (add 50%, recompute average entry and stop).
///   - **Same side, other**: skip (hold position).
///   - **Opposite side, confidence < MIN_CONFIDENCE**: ignore (no flip).
///   - **Opposite side, confidence ≥ MIN_CONFIDENCE**: close current, open new.
///
/// For a new position, all six guards must pass:
///   1. `confidence ≥ MIN_CONFIDENCE` (0.68)
///   2. Open positions < `MAX_POSITIONS` (8)
///   3. Same-direction positions < `MAX_SAME_DIRECTION` (4)
///   4. Sufficient free capital (`size_usd ≥ $2`)
///   5. Per-trade heat ≤ `MAX_TRADE_HEAT` (2% of equity) — scales down if over
///   6. Portfolio heat < `MAX_PORTFOLIO_HEAT` (8% of equity)
///
/// Circuit breaker: if peak→current drawdown > `CB_DRAWDOWN_THRESHOLD` (8%),
/// position size is multiplied by `CB_SIZE_MULT` (0.35).
async fn execute_paper_trade(
    symbol:       &str,
    dec:          &decision::Decision,
    ind:          &indicators::TechnicalIndicators,
    bot_state:    &SharedState,
    weights:      &SharedWeights,
    trade_logger: &SharedTradeLogger,
    notifier:     &Option<SharedNotifier>,
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
                // Conditions: between -0.15R and -0.85R, ≤2 DCA add-ons, signal confidence ≥ DCA_MIN_CONFIDENCE
                // Upper bound extended to -0.85R (-0.75 was too tight; minor overshoots
                // permanently blocked DCA even when slots remained.)
                if r_mult < -0.15 && r_mult > -0.85 && pos.dca_count < 2 && dec.confidence >= DCA_MIN_CONFIDENCE {
                    drop(s);
                    dca_position(symbol, dec, ind, bot_state).await;
                    return;
                }

                // Same side but conditions not met — hold without action
                return;
            }

            // ── Opposite side: ONLY reverse on high-confidence signal ──
            // Require ≥MIN_CONFIDENCE to avoid noise flipping existing positions.
            // Use dynamic floor based on performance metrics.
            let effective_floor = s.metrics.confidence_floor(MIN_CONFIDENCE);
            if dec.confidence < effective_floor {
                info!("⏸  {} opposing signal ignored (conf {:.0}% < {:.0}%) — holding {} position",
                      symbol, dec.confidence * 100.0, effective_floor * 100.0, pos.side);
                return;
            }
            // Minimum hold guard — do not flip a position that opened < 30 min ago.
            // Early noise can easily generate a 1-bar counter-signal; the original
            // signal needs time to play out before we admit it has reversed.
            if pos.cycles_held < 60 {
                info!("⏸  {} reversal blocked — position only {} cycles old (need 60 / 30 min)",
                      symbol, pos.cycles_held);
                return;
            }
            // Clone values before dropping the read guard to satisfy the borrow checker
            let pos_side    = pos.side.clone();
            let r_mult_snap = r_mult;
            let cycles_snap = pos.cycles_held;
            drop(s);
            info!("🔄 {} signal reversal: {} at {:.2}R  held={}  conf={:.0}%",
                  symbol, pos_side, r_mult_snap, cycles_snap, dec.confidence * 100.0);
            close_paper_position(symbol, dec.entry_price, "SignalExit", bot_state, weights, trade_logger, notifier).await;
        }
        // No existing: fall through to new entry
    }

    // ── Correlation filter ────────────────────────────────────────────────
    // Prevents stacking highly-correlated same-direction positions (e.g. BTC+ETH LONG
    // when corr=0.85 doubles macro exposure, not diversification).
    // Uses 30-day rolling Pearson correlation from correlation.rs.
    {
        let s = bot_state.read().await;
        match correlation::correlation_block(symbol, target_side, dec.confidence, &s.positions) {
            correlation::CorrBlock::Blocked { existing, corr, existing_conf } => {
                info!("⚡ {} {} blocked by correlation — {:.2} corr with {} \
                       (conf {:.0}% ≤ existing {:.0}% + {:.0}% edge required)",
                      symbol, target_side, corr, existing,
                      dec.confidence * 100.0, existing_conf * 100.0,
                      correlation::CONF_EDGE * 100.0);
                return;
            }
            correlation::CorrBlock::Override { existing, corr } => {
                info!("⚡ {} {} corr-override approved — {:.2} corr with {} \
                       (confidence edge sufficient)",
                      symbol, target_side, corr, existing);
            }
            correlation::CorrBlock::Clear => {}
        }
    }

    // ── Open new position ─────────────────────────────────────────────────
    let atr = ind.atr.max(dec.entry_price * 0.001);

    let mut s     = bot_state.write().await;
    let metrics   = s.metrics.clone();
    let equity    = s.capital + s.positions.iter().map(|p| p.size_usd + p.unrealised_pnl).sum::<f64>();

    // ── Circuit breaker ───────────────────────────────────────────────────
    // Uses rolling 7-day peak (not all-time) so a single lucky spike long
    // ago doesn't permanently throttle sizing.  When drawdown from the
    // 7-day high exceeds CB_DRAWDOWN_THRESHOLD (8%), new position sizes are
    // scaled to CB_SIZE_MULT (0.35×).
    let rolling_peak = s.equity_window.iter()
        .map(|&(_, e)| e)
        .fold(equity, f64::max); // fallback to current equity if window empty
    let drawdown = if rolling_peak > 0.0 {
        (rolling_peak - equity) / rolling_peak
    } else {
        0.0
    };
    let in_cb = drawdown > CB_DRAWDOWN_THRESHOLD;

    // ── Minimum confidence gate ────────────────────────────────────────────
    // Only enter trades where the signal is genuinely strong.
    // Signals below MIN_CONFIDENCE generated 0W/14L in choppy markets.
    // Use dynamic floor based on performance metrics; add extra 0.10 if circuit breaker active.
    let mut effective_floor = metrics.confidence_floor(MIN_CONFIDENCE);
    if in_cb {
        effective_floor = (effective_floor + 0.10).min(0.92);
    }
    if dec.confidence < effective_floor {
        info!("⚠ {} skipped — confidence {:.0}% below {:.0}% minimum",
              symbol, dec.confidence * 100.0, effective_floor * 100.0);
        return;
    }
    s.cb_active = in_cb; // keep dashboard in sync with actual sizing CB
    if in_cb {
        info!("🔴 CB ACTIVE — 7d drawdown {:.1}% (>{:.0}%), sizing ×{:.2}",
              drawdown * 100.0, CB_DRAWDOWN_THRESHOLD * 100.0, CB_SIZE_MULT);
        trade_logger.lock().await.log(&TradeEvent::CircuitBreaker {
            ts:             ts_now(),
            activated:      true,
            drawdown_pct:   drawdown * 100.0,
            threshold_pct:  CB_DRAWDOWN_THRESHOLD * 100.0,
            size_mult:      CB_SIZE_MULT,
            peak_equity:    rolling_peak,
            current_equity: equity,
        });
        // Fire circuit-breaker webhook notification
        if let Some(n) = notifier {
            let n = n.clone();
            let dd = drawdown * 100.0;
            tokio::spawn(async move {
                n.circuit_breaker(dd, CB_SIZE_MULT).await;
            });
        }
    }

    let pct          = position_size_pct(dec.confidence, &metrics, in_cb);
    let mut size_usd = s.capital * pct;

    // Guard: max MAX_POSITIONS concurrent positions (quality over quantity)
    if s.positions.len() >= MAX_POSITIONS {
        info!("⚠ {} skipped — max {} positions open", symbol, MAX_POSITIONS);
        return;
    }
    // Guard: max MAX_SAME_DIRECTION positions per side (prevent directional overexposure)
    let same_dir = s.positions.iter().filter(|p| p.side == target_side).count();
    if same_dir >= MAX_SAME_DIRECTION {
        info!("⚠ {} skipped — already {} {} positions (max {} per direction)",
              symbol, same_dir, target_side, MAX_SAME_DIRECTION);
        return;
    }
    // Guard: min position size
    if size_usd < 2.0 || s.capital < size_usd {
        info!("⚠ {} skipped — insufficient capital (${:.2})", symbol, s.capital);
        return;
    }
    // Guard: per-trade heat ≤ MAX_TRADE_HEAT (2%) of equity.
    // If the default Kelly/confidence size would exceed the heat limit, scale it
    // down to the maximum allowed size rather than skipping the trade entirely.
    //
    // Note: leverage is factored into both heat and the allowed-size calculation.
    // Without it the check underestimates actual risk by the full leverage multiple.
    let stop_dist_pct = (dec.entry_price - dec.stop_loss).abs() / dec.entry_price.max(1e-8);
    let t_heat = trade_heat(dec.entry_price, dec.stop_loss, size_usd, equity, dec.leverage);
    if t_heat > MAX_TRADE_HEAT {
        // Max MARGIN that keeps notional R-risk at exactly MAX_TRADE_HEAT of equity:
        //   MAX_TRADE_HEAT = stop_dist_pct × margin × leverage / equity
        //   → margin = MAX_TRADE_HEAT × equity / (stop_dist_pct × leverage)
        let allowed = MAX_TRADE_HEAT * equity / (stop_dist_pct * dec.leverage);
        if allowed < 2.0 {
            info!("⚠ {} skipped — stop too tight, min heat size ${:.2} < $2", symbol, allowed);
            return;
        }
        info!("🌡 {} heat-scaled: ${:.2} → ${:.2} (stop_dist={:.2}% × {:.1}×lev)",
              symbol, size_usd, allowed, stop_dist_pct * 100.0, dec.leverage);
        size_usd = allowed;  // apply the reduction — enforce the heat limit
    }
    // Guard: total portfolio heat ≤ MAX_PORTFOLIO_HEAT
    let p_heat = portfolio_heat(&s.positions, equity);
    if p_heat >= MAX_PORTFOLIO_HEAT {
        info!("🔥 {} skipped — portfolio heat {:.1}% (max {:.0}%)",
              symbol, p_heat * 100.0, MAX_PORTFOLIO_HEAT * 100.0);
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
        entry_confidence: dec.confidence,
    });

    let kelly_str = if metrics.kelly_fraction() > 0.0 {
        format!("Kelly={:.1}%", metrics.kelly_fraction() * 100.0)
    } else { "pre-Kelly".to_string() };

    info!("📝 {} {} @ ${:.4}  margin=${:.2}  {:.1}×lev  notional=${:.2}  R=${:.2}  heat={:.1}%  [{}]",
        target_side, symbol, dec.entry_price,
        size_usd, leverage, notional,
        r_risk, p_heat * 100.0, kelly_str);

    // Log the trade entry
    drop(s);
    trade_logger.lock().await.log(&TradeEvent::TradeEntry {
        ts:                 ts_now(),
        symbol:             symbol.to_string(),
        side:               target_side.to_string(),
        entry_price:        dec.entry_price,
        size_usd,
        leverage,
        notional_usd:       notional,
        stop_loss:          dec.stop_loss,
        take_profit:        dec.take_profit,
        r_risk_usd:         r_risk,
        confidence:         dec.confidence,
        rationale:          dec.rationale.chars().take(200).collect(),
        in_circuit_breaker: in_cb,
        portfolio_heat_pct: p_heat * 100.0,
        kelly_pct:          metrics.kelly_fraction() * 100.0,
    });

    // ── Fire position-opened webhook / Telegram notification ──────────────
    if let Some(n) = notifier {
        let n       = n.clone();
        let sym     = symbol.to_string();
        let side    = target_side.to_string();
        let entry   = dec.entry_price;
        let sz      = size_usd;
        let conf    = dec.confidence;
        let sl      = dec.stop_loss;
        let lev     = leverage;
        tokio::spawn(async move {
            n.position_opened(&sym, &side, entry, sz, conf, sl, lev).await;
        });
    }
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

        // Apply the same leverage as the original position so the new shares
        // carry the same notional exposure per dollar of margin.
        let lev     = s.positions[idx].leverage;
        let add_qty = add_size * lev / dec.entry_price;

        // Weighted average entry — pyramid price is higher than original,
        // so avg_entry rises. Needed for correct unrealised_pnl tracking.
        let old_qty   = s.positions[idx].quantity;
        let old_entry = s.positions[idx].entry_price;
        let new_qty   = old_qty + add_qty;
        let avg_entry = (old_entry * old_qty + dec.entry_price * add_qty) / new_qty;

        s.capital -= add_size;

        s.positions[idx].quantity         = new_qty;
        s.positions[idx].size_usd         += add_size;
        s.positions[idx].entry_price      = avg_entry;
        s.positions[idx].r_dollars_risked += (dec.entry_price - s.positions[idx].stop_loss).abs() * add_qty;
        // Tighten stop to pyramided entry's stop if it's better
        if (dec.stop_loss > s.positions[idx].stop_loss && s.positions[idx].side == "LONG")
            || (dec.stop_loss < s.positions[idx].stop_loss && s.positions[idx].side == "SHORT") {
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

        // Apply leverage so DCA shares have the same notional-per-dollar as the original.
        let lev       = s.positions[idx].leverage;
        let add_qty   = add_size * lev / dec.entry_price;
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
    symbol:       String,
    exit_price:   f64,
    tranche:      u8,
    bot_state:    &SharedState,
    weights:      &SharedWeights,
    trade_logger: &SharedTradeLogger,
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

        let partial_breakdown = Some(format!(
            "<div style='font-size:.78em;padding:4px 0;line-height:1.7'>\
             <div>Partial close ⅓ at <b>{lbl}</b> target &nbsp;·&nbsp; \
             entry <b>${entry:.4}</b> → <b>${exit:.4}</b></div>\
             <div style='color:#8b949e'>Locked in <b style='color:#3fb950'>{pnl:+.2}</b> \
             ({pct:+.1}%) on this tranche</div></div>",
            lbl   = r_label,
            entry = entry,
            exit  = exit_price,
            pnl   = trade_pnl,
            pct   = pnl_pct,
        ));
        let entry_time_partial = s.positions.get(idx)
            .map(|p| p.entry_time.clone()).unwrap_or_default();
        let leverage_partial = s.positions.get(idx)
            .map(|p| p.leverage).unwrap_or(1.0);
        let fees_partial = ledger::estimate_fees(close_size, leverage_partial);
        let partial_trade = ClosedTrade {
            symbol:     symbol.clone(),
            side,
            entry,
            exit:       exit_price,
            pnl:        trade_pnl,
            pnl_pct,
            reason:     format!("Partial{}R", r_label),
            closed_at:  now_str(),
            entry_time: entry_time_partial,
            quantity:   close_qty,
            size_usd:   close_size,
            leverage:   leverage_partial,
            fees_est:   fees_partial,
            breakdown:  partial_breakdown,
            note:       None,
        };
        ledger::append(&partial_trade);
        s.closed_trades.push(partial_trade);
        let len = s.closed_trades.len();
        if len > 100 { s.closed_trades.drain(0..len - 100); }

        s.metrics = PerformanceMetrics::calculate(&s.closed_trades);
        let m = &s.metrics;
        info!("📈 Metrics → Sharpe:{:.2} Kelly:{:.1}% WinRate:{:.0}%",
            m.sharpe,
            if m.kelly_fraction() > 0.0 { m.kelly_fraction() * 100.0 } else { 0.0 },
            m.win_rate * 100.0);

        // Log partial close
        let r_at_partial = if close_size > 0.0 { trade_pnl / close_size } else { 0.0 };
        trade_logger.lock().await.log(&TradeEvent::TradePartial {
            ts:              ts_now(),
            symbol:          symbol.clone(),
            side:            if was_long { "LONG".to_string() } else { "SHORT".to_string() },
            exit_price,
            size_closed_usd: close_size,
            pnl_usd:         trade_pnl,
            r_milestone:     if tranche == 1 { 2 } else { 4 },
            r_at_close:      r_at_partial,
        });

        drop(s);
        let mut w = weights.write().await;
        w.update(&contrib, was_long, trade_pnl > 0.0);
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  FULL CLOSE
// ═══════════════════════════════════════════════════════════════════════════════

async fn close_paper_position(
    symbol:       &str,
    exit_price:   f64,
    reason:       &str,
    bot_state:    &SharedState,
    weights:      &SharedWeights,
    trade_logger: &SharedTradeLogger,
    notifier:     &Option<SharedNotifier>,
) {
    let mut s = bot_state.write().await;

    let idx = s.positions.iter().position(|p| p.symbol == symbol);
    if let Some(idx) = idx {
        let pos = s.positions.remove(idx);

        // P&L = price move × quantity  (same formula as unrealised_pnl and partial close)
        // Using (exit - entry) * qty rather than (qty*exit - size_usd) because size_usd
        // is margin only; with leverage>1 the latter inflates P&L dramatically.
        let trade_pnl = if pos.side == "LONG" {
            (exit_price - pos.entry_price) * pos.quantity
        } else {
            (pos.entry_price - exit_price) * pos.quantity
        };

        s.capital += pos.size_usd + trade_pnl;
        s.pnl     += trade_pnl;

        let pnl_pct    = trade_pnl / pos.size_usd * 100.0;
        let profitable = trade_pnl > 0.0;
        let was_long   = pos.side == "LONG";
        let r_at_close = if pos.r_dollars_risked > 1e-8 { trade_pnl / pos.r_dollars_risked } else { 0.0 };

        info!("📝 CLOSE {} {} @ ${:.4} → {:+.2} ({:+.1}% / {:.2}R) [{}]",
            pos.side, symbol, exit_price, trade_pnl, pnl_pct, r_at_close, reason);

        // ── Build verbose breakdown for the click-to-expand dashboard row ────────
        let hold_mins = pos.cycles_held / 2;
        let hold_str  = if hold_mins < 60 { format!("{}m", hold_mins) }
                        else { format!("{:.1}h", hold_mins as f64 / 60.0) };
        let c = &pos.contrib;
        let sig_flags: String = [
            ("RSI",   c.rsi_bullish),
            ("BB",    c.bb_bullish),
            ("MACD",  c.macd_bullish),
            ("Trend", c.trend_bullish),
            ("OF",    c.of_bullish),
        ].iter().map(|(name, bull)| {
            let col = if *bull { "#3fb950" } else { "#f85149" };
            let arrow = if *bull { "↑" } else { "↓" };
            format!("<span style='color:{col}'>{name}{arrow}</span>")
        }).collect::<Vec<_>>().join(" ");
        let ai_line = match (&pos.ai_action, &pos.ai_reason) {
            (Some(act), Some(rsn)) => format!(
                "<div style='margin-top:4px'>🤖 <b style='color:#e3b341'>AI {}</b>: <span style='color:#cdd9e5'>{rsn}</span></div>",
                act.replace('_', " ").to_uppercase()
            ),
            _ => String::new(),
        };
        let breakdown = Some(format!(
            "<div style='font-size:.78em;padding:4px 0;line-height:1.7'>\
             <div><b>{side}</b> entry <b>${entry:.4}</b> → exit <b>${exit:.4}</b> \
             &nbsp;·&nbsp; held <b>{hold}</b> \
             &nbsp;·&nbsp; result <b style='color:{rc}'>{r:+.2}R</b></div>\
             <div style='color:#8b949e'>Stop <span style='color:#f85149'>${stop:.4}</span> \
             &nbsp;·&nbsp; TP <span style='color:#3fb950'>${tp:.4}</span> \
             &nbsp;·&nbsp; margin <b>${size:.2}</b> · {lev:.1}× lev</div>\
             <div style='margin-top:2px'>Signals at entry: {sigs}</div>\
             {ai_ln}</div>",
            side  = pos.side,
            entry = pos.entry_price,
            exit  = exit_price,
            hold  = hold_str,
            r     = r_at_close,
            rc    = if r_at_close >= 0.0 { "#3fb950" } else { "#f85149" },
            stop  = pos.stop_loss,
            tp    = pos.take_profit,
            size  = pos.size_usd,
            lev   = pos.leverage,
            sigs  = sig_flags,
            ai_ln = ai_line,
        ));

        let fees_full = ledger::estimate_fees(pos.size_usd, pos.leverage);
        let full_trade = ClosedTrade {
            symbol:     symbol.to_string(),
            side:       pos.side.clone(),
            entry:      pos.entry_price,
            exit:       exit_price,
            pnl:        trade_pnl,
            pnl_pct,
            reason:     reason.to_string(),
            closed_at:  now_str(),
            entry_time: pos.entry_time.clone(),
            quantity:   pos.quantity,
            size_usd:   pos.size_usd,
            leverage:   pos.leverage,
            fees_est:   fees_full,
            breakdown,
            note:       None,   // operator can add note via POST /api/trade-note
        };
        ledger::append(&full_trade);
        s.closed_trades.push(full_trade);
        let len = s.closed_trades.len();
        if len > 100 { s.closed_trades.drain(0..len - 100); }

        // ── Recalculate all metrics from updated history ──────────────────
        s.metrics = PerformanceMetrics::calculate(&s.closed_trades);
        let m = &s.metrics;
        info!("📈 Metrics → Sharpe:{:.2} Sortino:{:.2} Expect:{:+.1}% PF:{:.2} Kelly:{:.1}% CB:{}",
            m.sharpe, m.sortino, m.expectancy, m.profit_factor,
            if m.kelly_fraction() > 0.0 { m.kelly_fraction() * 100.0 } else { 0.0 },
            if m.in_circuit_breaker() { "ON" } else { "off" });

        // ── Log the exit ──────────────────────────────────────────────────
        trade_logger.lock().await.log(&TradeEvent::TradeExit {
            ts:               ts_now(),
            symbol:           symbol.to_string(),
            side:             pos.side.clone(),
            entry_price:      pos.entry_price,
            exit_price,
            size_usd:         pos.size_usd,
            pnl_usd:          trade_pnl,
            pnl_pct,
            r_multiple:       r_at_close,
            reason:           reason.to_string(),
            cycles_held:      pos.cycles_held as u32,
            minutes_held:     (pos.cycles_held / 2) as u32,
            dca_count:        pos.dca_count,
            tranches_closed:  pos.tranches_closed,
        });

        // ── Fire position-closed webhook / Telegram notification ─────────────
        if let Some(n) = notifier {
            let n        = n.clone();
            let sym      = symbol.to_string();
            let side     = pos.side.clone();
            let pnl      = trade_pnl;
            let pct      = pnl_pct;
            let why      = reason.to_string();
            let r        = r_at_close;
            tokio::spawn(async move {
                n.position_closed(&sym, &side, pnl, pct, &why, r).await;
            });
        }

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

// ═══════════════════════════════════════════════════════════════════════════════
//  UNIT TESTS — position management & heat calculations
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use learner::SignalContribution;

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Minimal PaperPosition for heat tests.
    fn make_pos(side: &str, entry: f64, stop: f64, qty: f64, size_usd: f64) -> PaperPosition {
        PaperPosition {
            symbol:          "TEST".to_string(),
            side:            side.to_string(),
            entry_price:     entry,
            quantity:        qty,
            size_usd,
            stop_loss:       stop,
            take_profit:     if side == "LONG" { entry * 1.10 } else { entry * 0.90 },
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
            unrealised_pnl:  0.0,
            contrib:         SignalContribution::default(),
            ai_action:        None,
            ai_reason:        None,
            entry_confidence: 0.68,
        }
    }

    // ── trade_heat ────────────────────────────────────────────────────────────

    #[test]
    fn trade_heat_1x_leverage_matches_margin_based_formula() {
        // With 1× leverage: heat = stop_dist_pct × size_usd / equity
        // entry=$100, stop=$98 → stop_dist=2%,  size_usd=$100, equity=$1000
        // heat = 0.02 × 100 / 1000 = 0.002  (0.2 %)
        let heat = trade_heat(100.0, 98.0, 100.0, 1000.0, 1.0);
        assert!((heat - 0.002).abs() < 1e-10, "1× heat should be 0.002, got {heat}");
    }

    #[test]
    fn trade_heat_3x_leverage_triples_heat() {
        // 3× leverage triples actual notional risk vs 1× baseline
        let heat_1x = trade_heat(100.0, 98.0, 100.0, 1000.0, 1.0);
        let heat_3x = trade_heat(100.0, 98.0, 100.0, 1000.0, 3.0);
        assert!(
            (heat_3x - heat_1x * 3.0).abs() < 1e-10,
            "3× leverage should triple heat: {heat_1x} × 3 = {} ≠ {heat_3x}",
            heat_1x * 3.0
        );
    }

    #[test]
    fn trade_heat_zero_equity_guard() {
        // Equity below $1 is a divide-by-near-zero guard — always returns 1.0
        let heat = trade_heat(100.0, 98.0, 100.0, 0.5, 3.0);
        assert_eq!(heat, 1.0, "near-zero equity must return 1.0 sentinel");
    }

    #[test]
    fn trade_heat_exceeds_max_at_3x_where_1x_would_not() {
        // Setup: entry=$100, stop=$98 (2% stop), $100 margin, $1000 equity.
        // 1× heat = 0.002 — well below MAX_TRADE_HEAT (0.02).
        // 3× heat = 0.006 — still below, but 10× leverage = 0.02 = exactly at limit.
        // At 10× leverage, heat should equal exactly MAX_TRADE_HEAT (0.02).
        let heat_10x = trade_heat(100.0, 98.0, 100.0, 1000.0, 10.0);
        assert!(
            (heat_10x - MAX_TRADE_HEAT).abs() < 1e-10,
            "10× lev with 2% stop on 10% position should reach MAX_TRADE_HEAT exactly: {heat_10x}"
        );
    }

    #[test]
    fn trade_heat_short_position_uses_abs_distance() {
        // SHORT: entry < stop (stop is above entry)
        // entry=$100, stop=$103 → |100-103|/100 = 3% stop distance
        // With 1× leverage, $50 margin, $1000 equity:
        // heat = 0.03 × 50 / 1000 = 0.0015
        let heat = trade_heat(100.0, 103.0, 50.0, 1000.0, 1.0);
        assert!(
            (heat - 0.0015).abs() < 1e-10,
            "SHORT heat: expected 0.0015, got {heat}"
        );
    }

    #[test]
    fn trade_heat_allowed_margin_formula_is_consistent() {
        // If t_heat > MAX_TRADE_HEAT, the allowed-margin formula must produce
        // a size whose heat is exactly MAX_TRADE_HEAT.
        // allowed = MAX_TRADE_HEAT × equity / (stop_dist_pct × leverage)
        let entry: f64 = 100.0;
        let stop      = 95.0;   // 5% stop distance
        let equity    = 1000.0;
        let leverage  = 3.0;
        let stop_dist = (entry - stop).abs() / entry;  // 0.05

        // Deliberately oversized: $500 margin → heat = 0.05 × 500 × 3 / 1000 = 0.075
        let oversized_heat = trade_heat(entry, stop, 500.0, equity, leverage);
        assert!(oversized_heat > MAX_TRADE_HEAT, "test setup: heat {oversized_heat} must exceed limit");

        // Compute allowed margin
        let allowed = MAX_TRADE_HEAT * equity / (stop_dist * leverage);
        let heat_at_allowed = trade_heat(entry, stop, allowed, equity, leverage);
        assert!(
            (heat_at_allowed - MAX_TRADE_HEAT).abs() < 1e-10,
            "allowed-margin heat should equal MAX_TRADE_HEAT, got {heat_at_allowed}"
        );
    }

    // ── REGRESSION: leverage bug ──────────────────────────────────────────────

    #[test]
    fn regression_trade_heat_old_formula_underestimated_by_leverage() {
        // Before the fix: trade_heat() = stop_dist_pct × size_usd / equity
        // This ignored leverage, so at 3× it accepted trades 3× riskier than intended.
        //
        // Concrete example: entry=$100, stop=$98 (2%), $100 margin, 3× leverage, $1000 equity.
        // Old formula:  0.02 × 100 / 1000 = 0.002  (looks like only 0.2% at risk)
        // True formula: 0.02 × 100 × 3 / 1000 = 0.006 (actually 0.6% at risk — correct)
        //
        // The actual dollar loss if stop is hit:
        //   qty = 100 × 3 / 100 = 3 shares
        //   loss = (100 - 98) × 3 = $6  → 6/1000 = 0.6%  ✓ matches new formula
        let entry: f64 = 100.0; let stop = 98.0; let size_usd = 100.0; let equity = 1000.0;
        let leverage = 3.0;
        let qty = size_usd * leverage / entry;
        let actual_dollar_loss = (entry - stop).abs() * qty;
        let actual_heat = actual_dollar_loss / equity;

        let formula_heat = trade_heat(entry, stop, size_usd, equity, leverage);
        assert!(
            (formula_heat - actual_heat).abs() < 1e-10,
            "trade_heat() must equal actual dollar-loss / equity: {formula_heat} ≠ {actual_heat}"
        );
    }

    // ── portfolio_heat ────────────────────────────────────────────────────────

    #[test]
    fn portfolio_heat_empty_positions_returns_zero() {
        let heat = portfolio_heat(&[], 1000.0);
        assert_eq!(heat, 0.0, "no open positions → 0 heat");
    }

    #[test]
    fn portfolio_heat_zero_equity_guard() {
        let pos = make_pos("LONG", 100.0, 95.0, 1.0, 100.0);
        let heat = portfolio_heat(&[pos], 0.5);
        assert_eq!(heat, 1.0, "near-zero equity must return 1.0 sentinel");
    }

    #[test]
    fn portfolio_heat_long_stop_below_entry_has_positive_heat() {
        // entry=$100, stop=$95, qty=10 → risk = (100-95)×10 = $50 on $1000 equity = 5%
        let pos = make_pos("LONG", 100.0, 95.0, 10.0, 100.0);
        let heat = portfolio_heat(&[pos], 1000.0);
        let expected = 50.0 / 1000.0;  // 5%
        assert!(
            (heat - expected).abs() < 1e-10,
            "LONG stop below entry: expected {expected}, got {heat}"
        );
    }

    #[test]
    fn portfolio_heat_long_stop_at_breakeven_is_zero() {
        // Once trailing stop reaches entry price, there's no remaining downside risk.
        // stop = entry_price → (entry - stop) × qty = 0 → heat = 0.
        let pos = make_pos("LONG", 100.0, 100.0, 10.0, 100.0);  // stop AT entry
        let heat = portfolio_heat(&[pos], 1000.0);
        assert_eq!(heat, 0.0, "LONG stop at breakeven → zero heat");
    }

    #[test]
    fn portfolio_heat_long_stop_above_entry_is_zero() {
        // Trailing stop has advanced ABOVE entry (locked in profit, no remaining risk).
        // stop > entry for LONG → (entry - stop) < 0 → .max(0) → 0.
        let pos = make_pos("LONG", 100.0, 105.0, 10.0, 100.0);  // stop ABOVE entry
        let heat = portfolio_heat(&[pos], 1000.0);
        assert_eq!(heat, 0.0, "LONG stop above entry (trailing past BE) → zero heat");
    }

    #[test]
    fn portfolio_heat_short_stop_above_entry_has_positive_heat() {
        // SHORT: stop above entry = risk zone.
        // entry=$100, stop=$104, qty=10 → risk = (104-100)×10 = $40 on $1000 = 4%
        let pos = make_pos("SHORT", 100.0, 104.0, 10.0, 100.0);
        let heat = portfolio_heat(&[pos], 1000.0);
        let expected = 40.0 / 1000.0;  // 4%
        assert!(
            (heat - expected).abs() < 1e-10,
            "SHORT stop above entry: expected {expected}, got {heat}"
        );
    }

    #[test]
    fn portfolio_heat_short_stop_at_breakeven_is_zero() {
        let pos = make_pos("SHORT", 100.0, 100.0, 10.0, 100.0); // stop AT entry
        let heat = portfolio_heat(&[pos], 1000.0);
        assert_eq!(heat, 0.0, "SHORT stop at breakeven → zero heat");
    }

    #[test]
    fn portfolio_heat_short_stop_below_entry_is_zero() {
        // Trailing stop has advanced BELOW entry for a SHORT (locked in profit).
        let pos = make_pos("SHORT", 100.0, 96.0, 10.0, 100.0); // stop BELOW entry
        let heat = portfolio_heat(&[pos], 1000.0);
        assert_eq!(heat, 0.0, "SHORT stop below entry (trailing past BE) → zero heat");
    }

    #[test]
    fn portfolio_heat_multiple_positions_sum_correctly() {
        // Two positions: LONG $30 risk + SHORT $20 risk = $50 on $1000 equity = 5%
        let long_pos  = make_pos("LONG",  100.0, 97.0, 10.0, 100.0); // (100-97)×10 = $30
        let short_pos = make_pos("SHORT", 200.0, 202.0, 10.0, 100.0); // (202-200)×10 = $20
        let heat = portfolio_heat(&[long_pos, short_pos], 1000.0);
        let expected = 50.0 / 1000.0;  // 5%
        assert!(
            (heat - expected).abs() < 1e-10,
            "multi-position heat: expected {expected}, got {heat}"
        );
    }

    #[test]
    fn portfolio_heat_trailing_stop_position_contributes_zero() {
        // One live-risk position + one position where trailing stop is above entry.
        // Only the live-risk position should contribute to heat.
        let at_risk   = make_pos("LONG", 100.0, 95.0, 10.0, 100.0); // (100-95)×10 = $50
        let protected = make_pos("LONG", 100.0, 103.0, 10.0, 100.0); // stop above entry → 0
        let heat = portfolio_heat(&[at_risk, protected], 1000.0);
        let expected = 50.0 / 1000.0;  // only $50 risk from the at-risk position
        assert!(
            (heat - expected).abs() < 1e-10,
            "protected position must not add heat: expected {expected}, got {heat}"
        );
    }

    // ── REGRESSION: portfolio_heat stale r_dollars_risked ─────────────────────

    #[test]
    fn regression_portfolio_heat_old_formula_would_overstate_after_stop_advance() {
        // Pre-fix: portfolio_heat used p.r_dollars_risked / equity.
        // After trailing stop advances past entry, r_dollars_risked is still set to
        // the original entry risk.  The new formula correctly returns 0.
        //
        // Scenario: LONG entry=$100, original stop=$95, qty=10, equity=$1000.
        //   Original r_dollars_risked = (100-95)×10 = $50.
        //   Stop has since moved to $103 (trailing past breakeven).
        //   Current risk = 0 (no downside to stop).
        //   OLD formula: $50/$1000 = 5% ← WRONG, 3% phantom risk
        //   NEW formula: max(0, (100-103)×10) / 1000 = 0% ← CORRECT
        let mut pos = make_pos("LONG", 100.0, 103.0, 10.0, 100.0);
        pos.r_dollars_risked = 50.0; // stale from original entry (before stop advanced)

        let heat = portfolio_heat(&[pos], 1000.0);
        // Old formula would give 50/1000 = 0.05; new formula gives 0.
        assert_eq!(
            heat, 0.0,
            "REGRESSION: portfolio_heat must return 0 when trailing stop is above entry, \
             regardless of stale r_dollars_risked (old formula returned 0.05)"
        );
    }

    // ── position_size_pct ─────────────────────────────────────────────────────

    #[test]
    fn position_size_pct_pre_kelly_high_confidence() {
        // With <5 trades (no Kelly), conf=0.85 → base 8%, no Sharpe adjustment (1.0 mult)
        let metrics = PerformanceMetrics::default(); // total_trades=0 → no Kelly, size_mult=1.0
        let pct = position_size_pct(0.85, &metrics, false);
        // base=0.08 × sharpe_mult(1.0 for <3 trades) × no-CB → 0.08
        assert_eq!(pct, 0.08, "pre-Kelly high confidence: expected 8%");
    }

    #[test]
    fn position_size_pct_pre_kelly_mid_confidence() {
        let metrics = PerformanceMetrics::default();
        let pct = position_size_pct(0.75, &metrics, false);
        assert_eq!(pct, 0.06, "pre-Kelly mid confidence: expected 6%");
    }

    #[test]
    fn position_size_pct_pre_kelly_min_confidence() {
        let metrics = PerformanceMetrics::default();
        let pct = position_size_pct(MIN_CONFIDENCE, &metrics, false);
        assert_eq!(pct, 0.04, "pre-Kelly min confidence: expected 4%");
    }

    #[test]
    fn position_size_pct_circuit_breaker_reduces_size() {
        // CB active → CB_SIZE_MULT (0.35) applied on top of everything else.
        let metrics = PerformanceMetrics::default();
        let normal = position_size_pct(0.85, &metrics, false);
        let cb     = position_size_pct(0.85, &metrics, true);
        let expected_cb = normal * CB_SIZE_MULT;
        assert!(
            (cb - expected_cb).abs() < 1e-10,
            "CB should multiply by {CB_SIZE_MULT}: {normal} × {CB_SIZE_MULT} = {expected_cb}, got {cb}"
        );
    }

    #[test]
    fn position_size_pct_never_below_min() {
        // Even with CB + negative Sharpe, result can't go below MIN_POSITION_PCT
        let metrics = PerformanceMetrics { sharpe: -2.0, total_trades: 10, ..Default::default() };
        let pct = position_size_pct(MIN_CONFIDENCE, &metrics, true);
        assert!(
            pct >= MIN_POSITION_PCT,
            "position_size_pct must never go below MIN_POSITION_PCT ({MIN_POSITION_PCT}), got {pct}"
        );
    }

    #[test]
    fn position_size_pct_never_above_max() {
        // Very high Kelly with great Sharpe still can't exceed MAX_POSITION_PCT
        let metrics = PerformanceMetrics {
            sharpe:        3.0,
            win_rate:      0.80,
            avg_win_pct:   25.0,
            avg_loss_pct:  5.0,
            total_trades:  50,
            ..Default::default()
        };
        let pct = position_size_pct(1.0, &metrics, false);
        assert!(
            pct <= MAX_POSITION_PCT,
            "position_size_pct must never exceed MAX_POSITION_PCT ({MAX_POSITION_PCT}), got {pct}"
        );
    }

    // ── P&L formula correctness ───────────────────────────────────────────────

    #[test]
    fn pnl_formula_long_close_matches_price_move_times_qty() {
        // close_paper_position uses: (exit - entry) * quantity for LONG
        // This is the leveraged return: qty = margin × leverage / entry
        let entry: f64 = 100.0;
        let exit     = 110.0; // +10%
        let margin   = 100.0;
        let leverage = 3.0;
        let qty      = margin * leverage / entry; // 3.0 shares
        let expected_pnl = (exit - entry) * qty;   // $30

        assert!(
            (expected_pnl - 30.0).abs() < 1e-10,
            "LONG P&L: 3 shares × $10 move = $30, got {expected_pnl}"
        );

        // Verify pnl_pct (relative to margin, not notional): 30/100 = 30%
        let pnl_pct = expected_pnl / margin * 100.0;
        assert!(
            (pnl_pct - 30.0).abs() < 1e-10,
            "pnl_pct should be 30%% on margin (3× leveraged 10% move), got {pnl_pct}"
        );
    }

    #[test]
    fn pnl_formula_short_close_matches_price_move_times_qty() {
        // close_paper_position uses: (entry - exit) * quantity for SHORT
        let entry: f64 = 100.0;
        let exit     = 90.0;  // price falls 10% → SHORT wins
        let margin   = 100.0;
        let leverage = 3.0;
        let qty      = margin * leverage / entry; // 3.0 shares
        let expected_pnl = (entry - exit) * qty;  // $30

        assert!(
            (expected_pnl - 30.0).abs() < 1e-10,
            "SHORT P&L: 3 shares × $10 favourable move = $30, got {expected_pnl}"
        );
    }

    #[test]
    fn r_multiple_formula_is_consistent_with_pnl_and_entry_risk() {
        // r_mult = unrealised_pnl / r_dollars_risked
        // At 1R: pnl should equal r_dollars_risked
        let entry: f64 = 100.0;
        let stop     = 95.0;  // 5% distance
        let margin   = 100.0;
        let leverage = 3.0;
        let qty      = margin * leverage / entry;                   // 3.0 shares
        let r_risk   = (entry - stop).abs() * qty;                 // $15

        // Price moves to 1R target: entry + (entry-stop) = $105
        let price_at_1r = entry + (entry - stop);                  // $105
        let pnl_at_1r   = (price_at_1r - entry) * qty;            // $15

        let r_mult = pnl_at_1r / r_risk;
        assert!(
            (r_mult - 1.0).abs() < 1e-10,
            "at price = entry + 1×stop_dist, R-multiple should be 1.0, got {r_mult}"
        );
    }

    #[test]
    fn dca_weighted_avg_entry_formula() {
        // After DCA add-on: avg_entry = (old_qty × old_entry + add_qty × dca_price) / new_qty
        let old_entry: f64 = 100.0;
        let dca_price = 95.0;  // price fell, we DCA lower
        let old_qty   = 3.0;
        let add_qty   = 1.5;   // 50% of original size
        let new_qty   = old_qty + add_qty; // 4.5

        let avg_entry = (old_entry * old_qty + dca_price * add_qty) / new_qty;
        // = (300 + 142.5) / 4.5 = 442.5 / 4.5 = 98.333...
        let expected = 442.5 / 4.5;
        assert!(
            (avg_entry - expected).abs() < 1e-10,
            "DCA avg_entry should be {expected}, got {avg_entry}"
        );
        // avg_entry must be BETWEEN original entry and DCA price
        assert!(avg_entry < old_entry && avg_entry > dca_price,
            "DCA avg_entry {avg_entry} must be between {dca_price} and {old_entry}");
    }

    #[test]
    fn pyramid_weighted_avg_entry_formula() {
        // After pyramid: avg_entry = (old_qty × old_entry + add_qty × pyramid_price) / new_qty
        let old_entry: f64 = 100.0;
        let pyramid_price = 106.0; // price rose 6%, we pyramid
        let old_qty      = 3.0;
        let add_qty      = 1.5;   // 50% add-on
        let new_qty      = old_qty + add_qty; // 4.5

        let avg_entry = (old_entry * old_qty + pyramid_price * add_qty) / new_qty;
        // = (300 + 159) / 4.5 = 459 / 4.5 = 102.0
        let expected = 459.0 / 4.5;
        assert!(
            (avg_entry - expected).abs() < 1e-10,
            "pyramid avg_entry should be {expected}, got {avg_entry}"
        );
        // avg_entry must be ABOVE original entry (pyramid is into profit)
        assert!(avg_entry > old_entry && avg_entry < pyramid_price,
            "pyramid avg_entry {avg_entry} must be between {old_entry} and {pyramid_price}");
    }

    #[test]
    fn partial_close_r_dollars_risked_scales_proportionally() {
        // After 1/3 close (tranche 1), r_dollars_risked *= 2/3.
        // Simulates take_partial() logic.
        let original_r: f64 = 150.0;
        let after_partial = original_r * (2.0 / 3.0);
        let expected = 100.0;
        assert!(
            (after_partial - expected).abs() < 1e-10,
            "after 1/3 close, r_dollars_risked should scale to 2/3: {after_partial} ≠ {expected}"
        );
    }

    #[test]
    fn trailing_stop_breakeven_move_for_long() {
        // At 1R profit: stop moves to entry (breakeven).
        // pos.stop_loss < pos.entry_price AND r_mult >= 1.0 → stop = entry
        let entry: f64 = 100.0;
        let stop    = 95.0;
        let qty     = 3.0;
        let r_risk  = (entry - stop) * qty; // $15

        // Current price at exactly 1R: entry + (entry - stop) = $105
        let cur     = 105.0;
        let unrealised = (cur - entry) * qty; // $15
        let r_mult  = unrealised / r_risk;    // 1.0

        assert!((r_mult - 1.0).abs() < 1e-10, "should be exactly 1R at $105");

        // Trailing stop rule: if r_mult >= 1.0 && stop < entry → set stop = entry
        let new_stop = if r_mult >= 1.0 && stop < entry { entry } else { stop };
        assert_eq!(new_stop, entry, "stop should move to breakeven at 1R");
    }

    #[test]
    fn trailing_stop_trails_hwm_at_1_5r_for_long() {
        // At ≥ 1.5R: trail 1.2×ATR below HWM.
        let entry: f64 = 100.0;
        let stop    = 95.0;
        let qty     = 3.0;
        let atr     = 2.0;
        let r_risk  = (entry - stop) * qty; // $15

        // Price at 1.5R: entry + 1.5 × (entry-stop) = $107.50
        let cur    = 107.5;
        let unr    = (cur - entry) * qty;
        let r_mult = unr / r_risk; // 1.5

        assert!((r_mult - 1.5).abs() < 1e-10, "should be exactly 1.5R at $107.50");

        let hwm    = cur;
        let trail  = hwm - atr * 1.2; // 107.5 - 2.4 = 105.1
        // Only advance stop if trail > current stop
        let new_stop = if r_mult >= 1.5 && trail > stop { trail } else { stop };
        assert!(
            (new_stop - 105.1).abs() < 1e-10,
            "trailing stop at 1.5R should be HWM - 1.2×ATR = 105.1, got {new_stop}"
        );
    }
}
