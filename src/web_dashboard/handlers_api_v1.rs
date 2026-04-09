//! `handlers_api_v1` — part of the `web_dashboard` module tree.
//!
//! Shared types and helpers available via `use super::*;`.
#![allow(unused_imports)]

use super::*;

pub(crate) async fn api_v1_status_handler(State(app): State<AppState>) -> axum::response::Response {
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

pub(crate) fn default_duration() -> String {
    "30d".to_string()
}

/// x402 payment requirements object (embedded in 402 responses).
pub(crate) fn x402_payment_requirements(resource: &str, duration: &str) -> serde_json::Value {
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
pub(crate) async fn verify_base_usdc_payment(
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
#[allow(dead_code)]
pub(crate) async fn dispatch_session_webhooks(
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
pub(crate) async fn api_v1_session_handler(
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

    let tx_hash = match payment_header { Some(h) => h, None => return axum::http::StatusCode::BAD_REQUEST.into_response() };
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
        name: None,           // x402 sessions are anonymous
        balance_usd: 0.0,     // x402 sessions share the global pool, not a personal allocation
        session_pnl: 0.0,
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
pub(crate) async fn validate_bot_session(
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
pub(crate) async fn api_v1_session_status_handler(
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
pub(crate) async fn api_v1_session_command_handler(
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

    // ── Immediate read-only commands (no queueing needed) ─────────────────
    if cmd_str == "get_positions" {
        let s = app.bot_state.read().await;
        let positions: Vec<serde_json::Value> = s.positions.iter().map(|p| serde_json::json!({
            "symbol":          p.symbol,
            "side":            p.side,
            "entry_price":     p.entry_price,
            "quantity":        p.quantity,
            "size_usd":        p.size_usd,
            "leverage":        p.leverage,
            "unrealised_pnl":  p.unrealised_pnl,
            "stop_loss":       p.stop_loss,
            "take_profit":     p.take_profit,
            "cycles_held":     p.cycles_held,
            "venue":           p.venue,
            "funding_rate":    p.funding_rate,
            "ai_action":       p.ai_action,
        })).collect();
        let count = positions.len();
        return axum::response::Json(serde_json::json!({
            "ok":        true,
            "positions": positions,
            "count":     count,
        })).into_response();
    }

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
        "set_leverage" => {
            let lev = body.get("leverage").and_then(|v| v.as_i64()).unwrap_or(10) as i32;
            if symbol.is_empty() {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::response::Json(serde_json::json!({"error":"symbol required for set_leverage"})),
                ).into_response();
            }
            BotCommand::SetLeverage { symbol, leverage: lev.clamp(1, 50) }
        }
        "pause_trading" => BotCommand::PauseTrading,
        "resume_trading" => BotCommand::ResumeTrading,
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
                        "valid": ["close_position","take_profit","close_all","close_profitable","open_long","open_short","set_leverage","pause_trading","resume_trading","get_positions"]
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
pub(crate) async fn api_v1_query_handler(
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
pub(crate) async fn api_v1_hl_account_handler(
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

/// `GET /api/v1/session/{id}/positions` — real-time positions with full detail.
///
/// Returns every open position with entry, unrealized P&L, venue, funding rate,
/// and AI action. Protected by `Authorization: Bearer {token}`.
pub(crate) async fn api_v1_session_positions_handler(
    State(app): State<AppState>,
    headers: axum::http::HeaderMap,
    axum::extract::Path(session_id): axum::extract::Path<String>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if let Err(e) = validate_bot_session(&app, &headers, &session_id).await {
        return e;
    }
    let s = app.bot_state.read().await;
    let positions: Vec<serde_json::Value> = s.positions.iter().map(|p| serde_json::json!({
        "symbol":          p.symbol,
        "side":            p.side,
        "entry_price":     p.entry_price,
        "quantity":        p.quantity,
        "size_usd":        p.size_usd,
        "leverage":        p.leverage,
        "unrealised_pnl":  p.unrealised_pnl,
        "pnl_pct":         if p.size_usd > 0.0 { (p.unrealised_pnl / p.size_usd) * 100.0 } else { 0.0 },
        "stop_loss":       p.stop_loss,
        "take_profit":     p.take_profit,
        "cycles_held":     p.cycles_held,
        "entry_time":      p.entry_time,
        "venue":           p.venue,
        "funding_rate":    p.funding_rate,
        "funding_delta":   p.funding_delta,
        "ai_action":       p.ai_action,
        "ai_reason":       p.ai_reason,
        "ob_sentiment":    p.ob_sentiment,
    })).collect();
    let count = positions.len();
    let unrealised_total: f64 = s.positions.iter().map(|p| p.unrealised_pnl).sum();
    axum::response::Json(serde_json::json!({
        "ok":              true,
        "session_id":      session_id,
        "positions":       positions,
        "count":           count,
        "unrealised_total_usd": unrealised_total,
        "venue":           "Hyperliquid Perps (paper)",
    }))
    .into_response()
}

/// `GET /api/v1/session/{id}/trades` — closed trade history with venue filter.
///
/// Query param: `?venue=hyperliquid` (optional). Without it returns all trades.
/// Returns last 100 trades in reverse-chronological order.
/// Protected by `Authorization: Bearer {token}`.
pub(crate) async fn api_v1_session_trades_handler(
    State(app): State<AppState>,
    headers: axum::http::HeaderMap,
    axum::extract::Path(session_id): axum::extract::Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if let Err(e) = validate_bot_session(&app, &headers, &session_id).await {
        return e;
    }
    let venue_filter = params.get("venue").map(|v| v.to_lowercase());
    let s = app.bot_state.read().await;
    let trades: Vec<serde_json::Value> = s.closed_trades.iter().rev().take(100)
        .filter(|t| {
            if let Some(ref vf) = venue_filter {
                t.venue.to_lowercase().contains(vf.as_str())
            } else {
                true
            }
        })
        .map(|t| serde_json::json!({
            "symbol":     t.symbol,
            "side":       t.side,
            "entry":      t.entry,
            "exit":       t.exit,
            "pnl":        t.pnl,
            "pnl_pct":    t.pnl_pct,
            "reason":     t.reason,
            "venue":      t.venue,
            "entry_time": t.entry_time,
            "closed_at":  t.closed_at,
            "quantity":   t.quantity,
            "size_usd":   t.size_usd,
            "leverage":   t.leverage,
            "fees_est":   t.fees_est,
            "note":       t.note,
        }))
        .collect();
    let count = trades.len();
    let total_pnl: f64 = s.closed_trades.iter().map(|t| t.pnl).sum();
    axum::response::Json(serde_json::json!({
        "ok":            true,
        "session_id":    session_id,
        "trades":        trades,
        "count":         count,
        "total_pnl_usd": total_pnl,
        "venue_filter":  venue_filter,
    }))
    .into_response()
}

/// `GET /api/v1/venues/hyperliquid/markets` — live Hyperliquid market metadata.
///
/// Returns current mid prices, 24h funding rates, and mark prices for all
/// available perps. **No authentication required** — public endpoint for
/// agents doing pre-trade research.
pub(crate) async fn api_v1_venues_hyperliquid_markets_handler(
    State(_app): State<AppState>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    // Fetch from Hyperliquid info API
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(6))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    // Get meta (asset names, leverage limits)
    let meta_result = client
        .post("https://api.hyperliquid.xyz/info")
        .json(&serde_json::json!({"type": "meta"}))
        .send()
        .await;

    // Get all mids (current prices)
    let mids_result = client
        .post("https://api.hyperliquid.xyz/info")
        .json(&serde_json::json!({"type": "allMids"}))
        .send()
        .await;

    // Get funding rates
    let _funding_result = client
        .post("https://api.hyperliquid.xyz/info")
        .json(&serde_json::json!({"type": "fundingHistory", "coin": "BTC", "startTime": chrono::Utc::now().timestamp_millis() - 3_600_000}))
        .send()
        .await;

    let mids: serde_json::Value = match mids_result {
        Ok(r) => r.json().await.unwrap_or(serde_json::Value::Null),
        Err(_) => serde_json::Value::Null,
    };

    let meta: serde_json::Value = match meta_result {
        Ok(r) => r.json().await.unwrap_or(serde_json::Value::Null),
        Err(_) => serde_json::Value::Null,
    };

    // Build market list from mids (it's a map of coin -> price string)
    let markets: Vec<serde_json::Value> = if let Some(obj) = mids.as_object() {
        obj.iter().take(100).map(|(coin, price)| {
            let px: f64 = price.as_str().unwrap_or("0").parse().unwrap_or(0.0);
            serde_json::json!({
                "coin":     coin,
                "mid_px":   px,
                "venue":    "Hyperliquid Perps",
                "leverage_max": 50,
            })
        }).collect()
    } else {
        vec![]
    };

    let market_count = markets.len();
    axum::response::Json(serde_json::json!({
        "ok":          true,
        "venue":       "Hyperliquid Perps",
        "network":     "mainnet",
        "markets":     markets,
        "market_count": market_count,
        "meta":        meta,
        "fetched_at":  chrono::Utc::now().to_rfc3339(),
        "x402_info": {
            "session_required":  false,
            "session_endpoint":  "POST /api/v1/session",
            "command_endpoint":  "POST /api/v1/session/{id}/command",
        }
    }))
    .into_response()
}

/// `GET /api/v1/session/{id}/latency/stats` — per-session execution latency stats.
///
/// Returns p50/p95/p99 execution latency, throughput, and success rate.
/// Protected by `Authorization: Bearer {token}`.
///
/// Note: in v0.2.1 the in-memory `LatencyTracker` is per-process (not per-session).
/// The global tracker accumulates all trades on this node. Per-session isolation
/// is on the roadmap once the `ConnectorRegistry` is fully wired.
pub(crate) async fn api_v1_session_latency_stats_handler(
    State(app): State<AppState>,
    headers: axum::http::HeaderMap,
    axum::extract::Path(session_id): axum::extract::Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if let Err(e) = validate_bot_session(&app, &headers, &session_id).await {
        return e;
    }
    let window = params.get("window").map(|w| w.as_str()).unwrap_or("1h");
    // Read the latency tracker from app state
    let tracker: tokio::sync::RwLockReadGuard<crate::latency::LatencyTracker> = app.latency_tracker.read().await;
    let stats = tracker.stats();
    axum::response::Json(serde_json::json!({
        "ok":              true,
        "session_id":      session_id,
        "window":          window,
        "sample_count":    stats.sample_count,
        "p50_ms":          stats.p50_ms,
        "p95_ms":          stats.p95_ms,
        "p99_ms":          stats.p99_ms,
        "min_ms":          stats.min_ms,
        "max_ms":          stats.max_ms,
        "mean_ms":         stats.mean_ms,
        "trades_per_minute": stats.trades_per_minute,
        "success_rate_pct":  stats.success_rate_pct,
        "targets": {
            "p50_target_ms":  250.0,
            "p95_target_ms":  450.0,
            "p99_target_ms":  800.0,
        },
        "status": {
            "p50_ok":  stats.p50_ms <= 250.0 || stats.sample_count == 0,
            "p95_ok":  stats.p95_ms <= 450.0 || stats.sample_count == 0,
            "p99_ok":  stats.p99_ms <= 800.0 || stats.sample_count == 0,
        }
    }))
    .into_response()
}

/// `GET /venues` — public venues listing page.
///
/// Shows all available trading venues with specs, leverage limits, and status.
/// No authentication required.
/// `GET /wallet/:name` — personalised read-only wallet dashboard for a named session.
///
/// Looks up the session by `name` field (case-insensitive).
/// Public view — no auth required. Shows balance, P&L, open positions, and trades.
pub(crate) async fn wallet_page_handler(
    State(app): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let name_lc = name.to_lowercase();
    let s = app.bot_state.read().await;

    // Find session by name (case-insensitive)
    let session = s.bot_sessions.values().find(|sess| {
        sess.name.as_deref()
            .map(|n| n.to_lowercase() == name_lc)
            .unwrap_or(false)
    });

    let Some(sess) = session else {
        return (
            axum::http::StatusCode::NOT_FOUND,
            axum::response::Html(format!(r#"<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8">
<title>Wallet not found — tradingbots.fun</title>
<style>body{{font-family:sans-serif;background:#0d1117;color:#c9d1d9;display:flex;align-items:center;justify-content:center;height:100vh;margin:0}}
.box{{text-align:center}}.box h1{{color:#f85149;font-size:1.5rem}}.box a{{color:#58a6ff}}</style></head>
<body><div class="box"><h1>Wallet "{name}" not found</h1>
<p>Check the name or create it via the admin API.</p>
<p><a href="/">← Home</a></p></div></body></html>"#)),
        ).into_response();
    };

    let display_name = sess.name.clone().unwrap_or_else(|| name.clone());
    let balance      = sess.balance_usd;
    let risk_mode    = sess.risk_mode.clone().unwrap_or_else(|| "balanced".to_string());
    let venue        = sess.venue.clone();
    let plan         = sess.plan.clone();
    let created_at   = sess.created_at.clone();
    let expires_at   = sess.expires_at.clone();
    let session_id   = sess.id.clone();
    let pnl          = sess.session_pnl;
    let session_pnl_pct = if balance > 0.0 { (pnl / balance) * 100.0 } else { 0.0 };

    // Gather global positions and trades (shared bot state)
    let open_count  = s.positions.len();
    let total_unrealised: f64 = s.positions.iter().map(|p| p.unrealised_pnl).sum();
    let total_trades = s.closed_trades.len();
    let global_pnl   = s.pnl;

    let risk_color = match risk_mode.as_str() {
        "aggressive"   => "#f85149",
        "conservative" => "#3fb950",
        _              => "#d29922",
    };
    let risk_emoji = match risk_mode.as_str() {
        "aggressive"   => "🔥",
        "conservative" => "🛡️",
        _              => "⚖️",
    };
    let pnl_color = if pnl >= 0.0 { "#3fb950" } else { "#f85149" };
    let pnl_sign  = if pnl >= 0.0 { "+" } else { "" };
    let gpnl_color = if global_pnl >= 0.0 { "#3fb950" } else { "#f85149" };
    let gpnl_sign  = if global_pnl >= 0.0 { "+" } else { "" };

    // Build positions rows
    let pos_rows: String = if s.positions.is_empty() {
        r#"<tr><td colspan="6" style="text-align:center;color:#484f58;padding:20px">No open positions yet</td></tr>"#.to_string()
    } else {
        s.positions.iter().map(|p| {
            let pc = if p.unrealised_pnl >= 0.0 { "#3fb950" } else { "#f85149" };
            let sc = if p.side == "LONG" { "#3fb950" } else { "#f85149" };
            format!(
                "<tr><td><b>{sym}</b></td><td style='color:{sc}'>{side}</td>\
                 <td>${entry:.4}</td><td>${cur:.4}</td>\
                 <td style='color:{pc}'>{sign}${pnl:.2}</td>\
                 <td>{lev:.1}×</td></tr>",
                sym   = p.symbol,
                sc    = sc,
                side  = p.side,
                entry = p.entry_price,
                cur   = p.entry_price + (p.unrealised_pnl / p.quantity.max(0.0001)),
                pc    = pc,
                sign  = if p.unrealised_pnl >= 0.0 { "+" } else { "" },
                pnl   = p.unrealised_pnl,
                lev   = p.leverage,
            )
        }).collect()
    };

    // Build recent trades rows (last 10)
    let trade_rows: String = if s.closed_trades.is_empty() {
        r#"<tr><td colspan="5" style="text-align:center;color:#484f58;padding:20px">No closed trades yet</td></tr>"#.to_string()
    } else {
        s.closed_trades.iter().rev().take(10).map(|t| {
            let pc = if t.pnl >= 0.0 { "#3fb950" } else { "#f85149" };
            let sc = if t.side == "LONG" { "#3fb950" } else { "#f85149" };
            format!(
                "<tr><td><b>{sym}</b></td><td style='color:{sc}'>{side}</td>\
                 <td style='color:{pc}'>{sign}${pnl:.2} ({sign}{pct:.1}%)</td>\
                 <td>{reason}</td><td style='color:#484f58'>{ts}</td></tr>",
                sym    = t.symbol,
                sc     = sc,
                side   = t.side,
                pc     = pc,
                sign   = if t.pnl >= 0.0 { "+" } else { "" },
                pnl    = t.pnl.abs(),
                pct    = t.pnl_pct.abs(),
                reason = t.reason,
                ts     = &t.closed_at.get(11..19).unwrap_or(""),
            )
        }).collect()
    };

    let html = format!(r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta http-equiv="refresh" content="30">
<title>{name} — tradingbots.fun</title>
<style>
*{{box-sizing:border-box;margin:0;padding:0}}
body{{font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;background:#0d1117;color:#c9d1d9;min-height:100vh}}
header{{padding:16px 28px;border-bottom:1px solid #21262d;display:flex;align-items:center;justify-content:space-between}}
.logo{{font-weight:700;font-size:1rem;color:#fff;text-decoration:none}}.logo span{{color:#58a6ff}}
nav a{{color:#8b949e;text-decoration:none;font-size:.85rem;margin-left:16px}}
main{{max-width:960px;margin:0 auto;padding:32px 20px}}
.wallet-hero{{display:flex;align-items:center;gap:20px;margin-bottom:32px}}
.avatar{{width:64px;height:64px;border-radius:50%;background:linear-gradient(135deg,#58a6ff,#3fb950);display:flex;align-items:center;justify-content:center;font-size:1.6rem;font-weight:700;color:#0d1117;flex-shrink:0}}
.wallet-name{{font-size:1.8rem;font-weight:700;color:#fff}}
.wallet-sub{{color:#8b949e;font-size:.9rem;margin-top:3px}}
.risk-pill{{display:inline-block;padding:3px 12px;border-radius:999px;font-size:.78rem;font-weight:600;margin-top:6px}}
.cards{{display:grid;grid-template-columns:repeat(auto-fit,minmax(160px,1fr));gap:14px;margin-bottom:28px}}
.card{{background:#161b22;border:1px solid #30363d;border-radius:10px;padding:18px 20px}}
.card-label{{font-size:.72rem;color:#6e7681;text-transform:uppercase;letter-spacing:.05em;margin-bottom:6px}}
.card-value{{font-size:1.5rem;font-weight:700;color:#fff}}
.card-sub{{font-size:.75rem;color:#6e7681;margin-top:3px}}
section{{margin-bottom:28px}}
.section-title{{font-size:.85rem;font-weight:600;color:#8b949e;text-transform:uppercase;letter-spacing:.06em;margin-bottom:12px;padding-bottom:8px;border-bottom:1px solid #21262d}}
table{{width:100%;border-collapse:collapse;font-size:.85rem}}
th{{text-align:left;color:#6e7681;font-weight:500;padding:8px 10px;border-bottom:1px solid #21262d;font-size:.78rem;text-transform:uppercase}}
td{{padding:10px 10px;border-bottom:1px solid #161b22}}
tr:last-child td{{border-bottom:none}}
.badge{{display:inline-block;font-size:.7rem;padding:2px 8px;border-radius:6px;background:#21262d;color:#8b949e}}
.live-dot{{width:8px;height:8px;border-radius:50%;background:#3fb950;display:inline-block;margin-right:6px;animation:pulse 2s infinite}}
@keyframes pulse{{0%,100%{{opacity:1}}50%{{opacity:.4}}}}
footer{{text-align:center;padding:30px 20px;color:#484f58;font-size:.8rem;border-top:1px solid #21262d;margin-top:40px}}
footer a{{color:#58a6ff;text-decoration:none}}
.refresh-note{{color:#484f58;font-size:.72rem;text-align:right;margin-bottom:16px}}
</style>
</head>
<body>
<header>
  <a class="logo" href="/">trading<span>bots</span>.fun</a>
  <nav>
    <a href="/">Home</a>
    <a href="/venues">Venues</a>
    <a href="/api/v1/status">API</a>
  </nav>
</header>
<main>

  <div class="wallet-hero">
    <div class="avatar">{initial}</div>
    <div>
      <div class="wallet-name">{name}'s Wallet</div>
      <div class="wallet-sub">
        <span class="live-dot"></span>Paper trading · {venue} · Session {session_id_short}
      </div>
      <span class="risk-pill" style="background:rgba(0,0,0,.3);color:{risk_color};border:1px solid {risk_color}40">
        {risk_emoji} {risk_mode_cap} mode
      </span>
    </div>
  </div>

  <div class="cards">
    <div class="card">
      <div class="card-label">Starting Balance</div>
      <div class="card-value">${balance:.0}</div>
      <div class="card-sub">paper USDC</div>
    </div>
    <div class="card">
      <div class="card-label">Session P&amp;L</div>
      <div class="card-value" style="color:{pnl_color}">{pnl_sign}${pnl:.2}</div>
      <div class="card-sub" style="color:{pnl_color}">{pnl_sign}{pnl_pct:.2}%</div>
    </div>
    <div class="card">
      <div class="card-label">Unrealised P&amp;L</div>
      <div class="card-value" style="color:{gpnl_color}">{gpnl_sign}${unrealised:.2}</div>
      <div class="card-sub">{open_count} open position{pos_s}</div>
    </div>
    <div class="card">
      <div class="card-label">Bot Lifetime P&amp;L</div>
      <div class="card-value" style="color:{gpnl_color}">{gpnl_sign}${global_pnl:.2}</div>
      <div class="card-sub">{total_trades} trade{trade_s} closed</div>
    </div>
  </div>

  <p class="refresh-note">↻ Auto-refreshes every 30s</p>

  <section>
    <div class="section-title">Open Positions</div>
    <table>
      <thead><tr><th>Symbol</th><th>Side</th><th>Entry</th><th>Mark</th><th>Unreal. P&amp;L</th><th>Lev</th></tr></thead>
      <tbody>{pos_rows}</tbody>
    </table>
  </section>

  <section>
    <div class="section-title">Recent Trades</div>
    <table>
      <thead><tr><th>Symbol</th><th>Side</th><th>P&amp;L</th><th>Reason</th><th>Time</th></tr></thead>
      <tbody>{trade_rows}</tbody>
    </table>
  </section>

  <section>
    <div class="section-title">Session Details</div>
    <table>
      <tbody>
        <tr><td style="color:#6e7681;width:140px">Session ID</td><td><span class="badge">{session_id}</span></td></tr>
        <tr><td style="color:#6e7681">Plan</td><td>{plan}</td></tr>
        <tr><td style="color:#6e7681">Risk Mode</td><td style="color:{risk_color}">{risk_emoji} {risk_mode_cap}</td></tr>
        <tr><td style="color:#6e7681">Venue</td><td>{venue}</td></tr>
        <tr><td style="color:#6e7681">Created</td><td>{created_at_short}</td></tr>
        <tr><td style="color:#6e7681">Expires</td><td>{expires_at_short}</td></tr>
        <tr><td style="color:#6e7681">API</td><td><a href="/api/wallet/{name_lc}" style="color:#58a6ff">/api/wallet/{name_lc}</a></td></tr>
      </tbody>
    </table>
  </section>

</main>
<footer>
  tradingbots.fun · <a href="/venues">Venues</a> · <a href="/api/v1/status">API Status</a>
</footer>
</body>
</html>"#,
        name             = display_name,
        initial          = display_name.chars().next().unwrap_or('?').to_uppercase().next().unwrap_or('?'),
        venue            = venue,
        session_id       = session_id,
        session_id_short = &session_id[..12.min(session_id.len())],
        risk_color       = risk_color,
        risk_emoji       = risk_emoji,
        risk_mode_cap    = {
            let mut c = risk_mode.chars();
            match c.next() {
                None    => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        },
        balance          = balance,
        pnl              = pnl,
        pnl_color        = pnl_color,
        pnl_sign         = pnl_sign,
        pnl_pct          = session_pnl_pct,
        gpnl_color       = gpnl_color,
        gpnl_sign        = gpnl_sign,
        global_pnl       = global_pnl,
        unrealised       = total_unrealised,
        open_count       = open_count,
        pos_s            = if open_count == 1 { "" } else { "s" },
        total_trades     = total_trades,
        trade_s          = if total_trades == 1 { "" } else { "s" },
        plan             = plan,
        created_at_short = &created_at[..10.min(created_at.len())],
        expires_at_short = &expires_at[..10.min(expires_at.len())],
        name_lc          = name_lc,
        pos_rows         = pos_rows,
        trade_rows       = trade_rows,
    );

    axum::response::Html(html).into_response()
}

/// `GET /api/wallet/:name` — JSON snapshot of a named wallet session.
///
/// Public read-only endpoint. Looks up session by name (case-insensitive).
pub(crate) async fn api_wallet_handler(
    State(app): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let name_lc = name.to_lowercase();
    let s = app.bot_state.read().await;

    let session = s.bot_sessions.values().find(|sess| {
        sess.name.as_deref()
            .map(|n| n.to_lowercase() == name_lc)
            .unwrap_or(false)
    });

    let Some(sess) = session else {
        return (
            axum::http::StatusCode::NOT_FOUND,
            axum::response::Json(serde_json::json!({"error": format!("Wallet '{}' not found", name)})),
        ).into_response();
    };

    let balance         = sess.balance_usd;
    let pnl             = sess.session_pnl;
    let pnl_pct         = if balance > 0.0 { (pnl / balance) * 100.0 } else { 0.0 };
    let total_unrealised: f64 = s.positions.iter().map(|p| p.unrealised_pnl).sum();

    let positions: Vec<serde_json::Value> = s.positions.iter().map(|p| serde_json::json!({
        "symbol":         p.symbol,
        "side":           p.side,
        "entry_price":    p.entry_price,
        "unrealised_pnl": p.unrealised_pnl,
        "size_usd":       p.size_usd,
        "leverage":       p.leverage,
        "venue":          p.venue,
    })).collect();

    let recent_trades: Vec<serde_json::Value> = s.closed_trades.iter().rev().take(20).map(|t| serde_json::json!({
        "symbol":    t.symbol,
        "side":      t.side,
        "pnl":       t.pnl,
        "pnl_pct":   t.pnl_pct,
        "reason":    t.reason,
        "closed_at": t.closed_at,
        "venue":     t.venue,
    })).collect();

    axum::response::Json(serde_json::json!({
        "ok":            true,
        "name":          sess.name,
        "session_id":    sess.id,
        "plan":          sess.plan,
        "risk_mode":     sess.risk_mode,
        "venue":         sess.venue,
        "balance_usd":   balance,
        "session_pnl":   pnl,
        "session_pnl_pct": pnl_pct,
        "unrealised_pnl": total_unrealised,
        "open_positions": positions,
        "recent_trades":  recent_trades,
        "expires_at":    sess.expires_at,
        "wallet_url":    format!("https://tradingbots.fun/wallet/{}", name_lc),
    })).into_response()
}

pub(crate) async fn public_venues_handler() -> impl axum::response::IntoResponse {
    axum::response::Html(r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Trading Venues — tradingbots.fun</title>
<meta name="description" content="Available trading venues on tradingbots.fun: Hyperliquid Perps, x402-native, non-custodial AI agent trading.">
<meta name="keywords" content="Hyperliquid perps, x402 trading API, autonomous agent trading, AI agent Hyperliquid, on-chain perpetuals">
<style>
*{box-sizing:border-box;margin:0;padding:0}
body{font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;background:#0d1117;color:#c9d1d9;min-height:100vh}
header{padding:20px 32px;border-bottom:1px solid #21262d;display:flex;align-items:center;gap:16px}
.logo{font-weight:700;font-size:1.1rem;color:#fff;text-decoration:none}
.logo span{color:#58a6ff}
nav a{color:#8b949e;text-decoration:none;font-size:.9rem;margin-left:20px}
nav a:hover{color:#c9d1d9}
main{max-width:900px;margin:0 auto;padding:48px 24px}
h1{font-size:2rem;font-weight:700;color:#fff;margin-bottom:8px}
.subtitle{color:#8b949e;margin-bottom:40px;font-size:1.05rem}
.venue-card{background:#161b22;border:1px solid #30363d;border-radius:12px;padding:28px 32px;margin-bottom:20px;transition:border-color .2s}
.venue-card:hover{border-color:#58a6ff}
.vc-header{display:flex;align-items:center;gap:16px;margin-bottom:16px}
.vc-name{font-size:1.35rem;font-weight:700;color:#fff}
.vc-status{font-size:.75rem;padding:3px 10px;border-radius:999px;font-weight:600}
.status-live{background:rgba(63,185,80,.15);color:#3fb950;border:1px solid rgba(63,185,80,.3)}
.status-paper{background:rgba(210,153,34,.15);color:#d29922;border:1px solid rgba(210,153,34,.3)}
.status-soon{background:rgba(139,148,158,.15);color:#8b949e;border:1px solid rgba(139,148,158,.3)}
.vc-tagline{color:#8b949e;font-size:.95rem;margin-bottom:20px}
.vc-specs{display:grid;grid-template-columns:repeat(auto-fit,minmax(160px,1fr));gap:12px;margin-bottom:20px}
.spec{background:#0d1117;border:1px solid #21262d;border-radius:8px;padding:12px 14px}
.spec-label{font-size:.72rem;color:#6e7681;text-transform:uppercase;letter-spacing:.05em;margin-bottom:4px}
.spec-value{font-size:.95rem;color:#fff;font-weight:600}
.vc-features{display:flex;flex-wrap:wrap;gap:8px;margin-bottom:20px}
.feat{font-size:.78rem;padding:4px 10px;border-radius:6px;background:rgba(56,139,253,.1);color:#58a6ff;border:1px solid rgba(56,139,253,.2)}
.vc-cta{display:inline-block;padding:10px 22px;background:#238636;color:#fff;border-radius:8px;text-decoration:none;font-size:.9rem;font-weight:600}
.vc-cta:hover{background:#2ea043}
.vc-api{background:#0d1117;border:1px solid #21262d;border-radius:8px;padding:14px 18px;margin-top:16px;font-family:'SF Mono',monospace;font-size:.82rem;color:#8b949e}
.vc-api code{color:#79c0ff}
footer{text-align:center;padding:40px 20px;color:#484f58;font-size:.82rem;border-top:1px solid #21262d;margin-top:60px}
footer a{color:#58a6ff;text-decoration:none}
</style>
</head>
<body>
<header>
  <a class="logo" href="/">trading<span>bots</span>.fun</a>
  <nav>
    <a href="/">Home</a>
    <a href="/app">Dashboard</a>
    <a href="/venues" style="color:#58a6ff">Venues</a>
    <a href="/api/v1/status">API Status</a>
  </nav>
</header>
<main>
  <h1>Trading Venues</h1>
  <p class="subtitle">Available execution venues for x402-powered AI agent sessions.</p>

  <!-- Hyperliquid Perps (Paper) -->
  <div class="venue-card">
    <div class="vc-header">
      <span class="vc-name">Hyperliquid Perps</span>
      <span class="vc-status status-paper">Paper Trading</span>
    </div>
    <p class="vc-tagline">Real Hyperliquid CLOB data, simulated execution. Perfect for agent development and strategy validation before going live.</p>
    <div class="vc-specs">
      <div class="spec"><div class="spec-label">Max Leverage</div><div class="spec-value">50×</div></div>
      <div class="spec"><div class="spec-label">Markets</div><div class="spec-value">~100 Perps</div></div>
      <div class="spec"><div class="spec-label">Settlement</div><div class="spec-value">USDC</div></div>
      <div class="spec"><div class="spec-label">Session Cost</div><div class="spec-value">10 USDC / 30d</div></div>
      <div class="spec"><div class="spec-label">Burst Session</div><div class="spec-value">0.5 USDC / 24h</div></div>
      <div class="spec"><div class="spec-label">Mode</div><div class="spec-value">Paper (live data)</div></div>
    </div>
    <div class="vc-features">
      <span class="feat">x402 payment</span>
      <span class="feat">Per-session wallet</span>
      <span class="feat">Drawdown guard</span>
      <span class="feat">Symbol whitelist</span>
      <span class="feat">Risk modes</span>
      <span class="feat">Webhook events</span>
      <span class="feat">Latency tracking</span>
      <span class="feat">Non-custodial</span>
    </div>
    <div class="vc-api">
      <code>POST /api/v1/session</code> with <code>"venue": "hyperliquid"</code> + <code>X-Payment: 0x...</code>
    </div>
  </div>

  <!-- Internal Engine -->
  <div class="venue-card">
    <div class="vc-header">
      <span class="vc-name">Internal Engine</span>
      <span class="vc-status status-live">Active</span>
    </div>
    <p class="vc-tagline">The built-in AI trading engine: 50+ symbols, adaptive signal weights, regime-aware thresholds. Default venue for all sessions.</p>
    <div class="vc-specs">
      <div class="spec"><div class="spec-label">Symbols</div><div class="spec-value">50+ pairs</div></div>
      <div class="spec"><div class="spec-label">AI Signals</div><div class="spec-value">12 weighted</div></div>
      <div class="spec"><div class="spec-label">Regime Engine</div><div class="spec-value">ADX(14)</div></div>
      <div class="spec"><div class="spec-label">Session Cost</div><div class="spec-value">10 USDC / 30d</div></div>
      <div class="spec"><div class="spec-label">Burst Session</div><div class="spec-value">0.5 USDC / 24h</div></div>
      <div class="spec"><div class="spec-label">Mode</div><div class="spec-value">Paper</div></div>
    </div>
    <div class="vc-features">
      <span class="feat">x402 payment</span>
      <span class="feat">Kelly sizing</span>
      <span class="feat">Adaptive weights</span>
      <span class="feat">Circuit breaker</span>
      <span class="feat">DCA engine</span>
      <span class="feat">AI reviewer</span>
    </div>
    <div class="vc-api">
      <code>POST /api/v1/session</code> (no venue field needed — internal is default)
    </div>
  </div>

  <!-- Coming Soon -->
  <div class="venue-card" style="opacity:.7">
    <div class="vc-header">
      <span class="vc-name">Hyperliquid Perps (Live)</span>
      <span class="vc-status status-soon">Coming Soon</span>
    </div>
    <p class="vc-tagline">Real on-chain execution on Hyperliquid's CLOB. Every trade stamped with a Hyperliquid tx signature. Full non-custodial flow.</p>
    <div class="vc-specs">
      <div class="spec"><div class="spec-label">Max Leverage</div><div class="spec-value">50×</div></div>
      <div class="spec"><div class="spec-label">Markets</div><div class="spec-value">~100 Perps</div></div>
      <div class="spec"><div class="spec-label">Settlement</div><div class="spec-value">USDC (on-chain)</div></div>
      <div class="spec"><div class="spec-label">Funding</div><div class="spec-value">Real HL rates</div></div>
    </div>
    <div class="vc-features">
      <span class="feat">On-chain tx hashes</span>
      <span class="feat">Real CLOB</span>
      <span class="feat">EIP-712 signing</span>
      <span class="feat">Bridge integration</span>
    </div>
  </div>

  <p style="color:#484f58;margin-top:24px;font-size:.85rem">
    Live market data: <a href="/api/v1/venues/hyperliquid/markets" style="color:#58a6ff">/api/v1/venues/hyperliquid/markets</a> (no auth required)
  </p>
</main>
<footer>
  <p>tradingbots.fun — x402-native AI agent trading · <a href="/api/v1/status">API Status</a> · <a href="/venues">Venues</a></p>
</footer>
</body>
</html>"#)
}

// ─────────────────────────────────────────────────────────────────────────────
// /fleet  –  Live aggregate view across all scale-test wallets
// ─────────────────────────────────────────────────────────────────────────────
pub(crate) async fn fleet_handler(State(app): State<AppState>) -> Html<String> {
    // ── Snapshot all tenant states ────────────────────────────────────────────
    let mgr = app.tenants.read().await;

    struct WalletSnap {
        name: String,
        equity: f64,
        initial: f64,
        pnl_pct: f64,
        trades: usize,
        wins: usize,
        cb: bool,
    }

    let mut snaps: Vec<WalletSnap> = Vec::with_capacity(mgr.count());
    let mut symbol_longs: HashMap<String, usize> = HashMap::new();
    let mut symbol_shorts: HashMap<String, usize> = HashMap::new();
    let mut total_open_pos: usize = 0;
    let mut total_long_pos: usize = 0;
    let mut total_short_pos: usize = 0;
    let mut cb_count: usize = 0;

    for handle in mgr.all() {
        // Skip non-scale wallets (demo bots, real users) for fleet view
        let id_str = handle.config.display_name.clone();
        let state = handle.state.read().await;
        let committed: f64 = state.positions.iter().map(|p| p.size_usd).sum();
        let unrealised: f64 = state.positions.iter().map(|p| p.unrealised_pnl).sum();
        let equity = state.capital + committed + unrealised;
        let pnl_pct = if state.initial_capital > 0.0 {
            (equity - state.initial_capital) / state.initial_capital * 100.0
        } else { 0.0 };

        for p in &state.positions {
            let sym = p.symbol.replace("-PERP", "").replace("USDT", "");
            if p.side == "LONG" {
                *symbol_longs.entry(sym).or_insert(0) += 1;
                total_long_pos += 1;
            } else {
                *symbol_shorts.entry(sym.clone()).or_insert(0) += 1;
                *symbol_shorts.entry(sym).or_insert(0) += 0; // ensure key exists
                total_short_pos += 1;
            }
        }
        total_open_pos += state.positions.len();
        if state.cb_active { cb_count += 1; }

        snaps.push(WalletSnap {
            name: id_str,
            equity,
            initial: state.initial_capital,
            pnl_pct,
            trades: state.metrics.total_trades,
            wins: state.metrics.wins,
            cb: state.cb_active,
        });
    }
    drop(mgr);

    let wallet_count = snaps.len();
    let total_initial: f64 = snaps.iter().map(|s| s.initial).sum();
    let total_equity: f64  = snaps.iter().map(|s| s.equity).sum();
    let total_pnl = total_equity - total_initial;
    let total_pnl_pct = if total_initial > 0.0 { total_pnl / total_initial * 100.0 } else { 0.0 };
    let total_trades: usize = snaps.iter().map(|s| s.trades).sum();
    let total_wins: usize   = snaps.iter().map(|s| s.wins).sum();
    let fleet_wr = if total_trades > 0 { total_wins as f64 / total_trades as f64 * 100.0 } else { 0.0 };

    // Sort copies for leaderboard
    snaps.sort_by(|a, b| b.pnl_pct.partial_cmp(&a.pnl_pct).unwrap_or(std::cmp::Ordering::Equal));

    let top10: String = snaps.iter().take(10).enumerate().map(|(i, s)| {
        let pnl_col = if s.pnl_pct >= 0.0 { "#3fb950" } else { "#f85149" };
        let sign    = if s.pnl_pct >= 0.0 { "+" } else { "" };
        format!(r#"<tr>
          <td style="color:#8b949e;width:28px">#{}</td>
          <td style="color:#e6edf3;font-weight:600">{}</td>
          <td style="color:#58a6ff">${:.2}</td>
          <td style="color:{};font-weight:700">{}{:.2}%</td>
          <td style="color:#8b949e">{}</td>
          <td style="color:#e3b341">{:.0}%</td>
          <td style="color:{}">{}</td>
        </tr>"#,
            i+1, s.name,
            s.equity,
            pnl_col, sign, s.pnl_pct,
            s.trades,
            if s.trades > 0 { s.wins as f64 / s.trades as f64 * 100.0 } else { 0.0 },
            if s.cb { "#f85149" } else { "#3fb950" },
            if s.cb { "🔴 CB" } else { "✅ OK" },
        )
    }).collect();

    let bot10: String = snaps.iter().rev().take(10).enumerate().map(|(i, s)| {
        let pnl_col = if s.pnl_pct >= 0.0 { "#3fb950" } else { "#f85149" };
        let sign    = if s.pnl_pct >= 0.0 { "+" } else { "" };
        format!(r#"<tr>
          <td style="color:#8b949e;width:28px">#{}</td>
          <td style="color:#e6edf3;font-weight:600">{}</td>
          <td style="color:#58a6ff">${:.2}</td>
          <td style="color:{};font-weight:700">{}{:.2}%</td>
          <td style="color:#8b949e">{}</td>
          <td style="color:#e3b341">{:.0}%</td>
          <td style="color:{}">{}</td>
        </tr>"#,
            wallet_count - i, s.name,
            s.equity,
            pnl_col, sign, s.pnl_pct,
            s.trades,
            if s.trades > 0 { s.wins as f64 / s.trades as f64 * 100.0 } else { 0.0 },
            if s.cb { "#f85149" } else { "#3fb950" },
            if s.cb { "🔴 CB" } else { "✅ OK" },
        )
    }).collect();

    // Symbol table — all symbols, sorted by total position count
    let mut sym_rows: Vec<(String, usize, usize)> = {
        let mut all_syms: std::collections::HashSet<String> = std::collections::HashSet::new();
        all_syms.extend(symbol_longs.keys().cloned());
        all_syms.extend(symbol_shorts.keys().cloned());
        all_syms.into_iter().map(|sym| {
            let l = symbol_longs.get(&sym).copied().unwrap_or(0);
            let s = symbol_shorts.get(&sym).copied().unwrap_or(0);
            (sym, l, s)
        }).collect()
    };
    sym_rows.sort_by(|a, b| (b.1+b.2).cmp(&(a.1+a.2)));

    let sym_table: String = sym_rows.iter().take(25).map(|(sym, l, sh)| {
        let total = l + sh;
        let net: i64 = *l as i64 - *sh as i64;
        let net_col = if net > 0 { "#3fb950" } else if net < 0 { "#f85149" } else { "#8b949e" };
        let bar_l = if total > 0 { l * 120 / total } else { 60 };
        let bar_s = 120usize.saturating_sub(bar_l);
        format!(r#"<tr>
          <td style="color:#58a6ff;font-weight:700;font-family:monospace">{}</td>
          <td style="color:#3fb950">▲ {}</td>
          <td style="color:#f85149">▼ {}</td>
          <td style="color:{};font-weight:700">{:+}</td>
          <td><div style="display:flex;gap:1px;align-items:center;height:10px">
            <div style="width:{}px;height:8px;background:#3fb950;border-radius:2px 0 0 2px;opacity:.8"></div>
            <div style="width:{}px;height:8px;background:#f85149;border-radius:0 2px 2px 0;opacity:.8"></div>
          </div></td>
        </tr>"#,
            sym, l, sh, net_col, net, bar_l, bar_s
        )
    }).collect();

    let pnl_col = if total_pnl >= 0.0 { "#3fb950" } else { "#f85149" };
    let pnl_sign = if total_pnl >= 0.0 { "+" } else { "" };
    let ts = chrono::Utc::now().format("%H:%M:%S UTC").to_string();

    Html(format!(r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Fleet Dashboard — TradingBots.fun</title>
<meta http-equiv="refresh" content="30">
<style>
*{{box-sizing:border-box;margin:0;padding:0}}
body{{background:#0d1117;color:#e6edf3;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;font-size:14px;min-height:100vh}}
a{{color:#58a6ff;text-decoration:none}}a:hover{{text-decoration:underline}}
.page{{max-width:1200px;margin:0 auto;padding:24px 16px}}
.header{{display:flex;justify-content:space-between;align-items:center;margin-bottom:24px;padding-bottom:16px;border-bottom:1px solid #21262d}}
.header h1{{font-size:1.3em;font-weight:800;background:linear-gradient(90deg,#58a6ff,#3fb950);-webkit-background-clip:text;-webkit-text-fill-color:transparent;background-clip:text}}
.header-right{{display:flex;align-items:center;gap:16px;font-size:.78em;color:#8b949e}}
.live-badge{{display:flex;align-items:center;gap:6px;background:#161b22;border:1px solid rgba(63,185,80,.3);border-radius:20px;padding:4px 12px;color:#3fb950;font-weight:700}}
.dot{{width:7px;height:7px;border-radius:50%;background:#3fb950;animation:pulse 1.4s ease-in-out infinite}}
@keyframes pulse{{0%,100%{{opacity:1}}50%{{opacity:.3}}}}
.nav-links{{display:flex;gap:12px;margin-bottom:20px;font-size:.82em}}
.nav-links a{{color:#8b949e;padding:4px 10px;border-radius:6px;border:1px solid #21262d}}
.nav-links a:hover{{background:#161b22;color:#e6edf3;text-decoration:none}}
.nav-links a.active{{background:#1f3d1f;border-color:rgba(63,185,80,.4);color:#3fb950}}
.metrics{{display:grid;grid-template-columns:repeat(auto-fit,minmax(160px,1fr));gap:12px;margin-bottom:24px}}
.card{{background:#161b22;border:1px solid #21262d;border-radius:10px;padding:16px}}
.card .label{{font-size:.72em;color:#8b949e;text-transform:uppercase;letter-spacing:.8px;margin-bottom:6px}}
.card .value{{font-size:1.6em;font-weight:800;line-height:1.1;font-variant-numeric:tabular-nums}}
.card .sub{{font-size:.78em;color:#8b949e;margin-top:4px}}
.section{{margin-bottom:28px}}
.section-title{{font-size:.78em;font-weight:700;text-transform:uppercase;letter-spacing:.8px;color:#8b949e;margin-bottom:12px;padding-bottom:8px;border-bottom:1px solid #21262d}}
.two-col{{display:grid;grid-template-columns:1fr 1fr;gap:16px}}
@media(max-width:700px){{.two-col{{grid-template-columns:1fr}}}}
table{{width:100%;border-collapse:collapse}}
th{{text-align:left;font-size:.72em;color:#8b949e;text-transform:uppercase;letter-spacing:.6px;padding:6px 8px;border-bottom:1px solid #21262d}}
td{{padding:7px 8px;border-bottom:1px solid #161b22;font-size:.82em}}
tr:last-child td{{border-bottom:none}}
tr:hover td{{background:rgba(255,255,255,.02)}}
.prog-bar{{height:6px;background:#21262d;border-radius:3px;overflow:hidden;margin-top:6px}}
.prog-fill{{height:100%;border-radius:3px}}
.ls-row{{display:flex;gap:8px;align-items:center;margin-bottom:10px}}
.ls-label{{font-size:.75em;color:#8b949e;width:50px}}
.ls-bar-wrap{{flex:1;height:20px;background:#21262d;border-radius:4px;overflow:hidden;display:flex}}
.ls-bar-l{{background:rgba(63,185,80,.7)}}
.ls-bar-s{{background:rgba(248,81,73,.7)}}
.ls-val{{font-size:.8em;font-weight:700;width:60px;text-align:right}}
.refresh-note{{font-size:.7em;color:#484f58;text-align:right;margin-top:16px}}
</style>
</head>
<body>
<div class="page">

  <div class="header">
    <h1>⚡ Fleet Dashboard</h1>
    <div class="header-right">
      <span>Updated {ts}</span>
      <div class="live-badge"><span class="dot"></span>LIVE · auto-refresh 30s</div>
    </div>
  </div>

  <div class="nav-links">
    <a href="/dashboard">📊 Primary Bot</a>
    <a href="/fleet" class="active">⚡ Fleet ({wallet_count} wallets)</a>
    <a href="/leaderboard">🏆 Leaderboard</a>
    <a href="/admin">🔧 Admin</a>
  </div>

  <!-- ── Key Metrics ── -->
  <div class="metrics">
    <div class="card">
      <div class="label">Active Wallets</div>
      <div class="value" style="color:#58a6ff">{wallet_count}</div>
      <div class="sub">scale-test fleet</div>
    </div>
    <div class="card">
      <div class="label">Total Equity</div>
      <div class="value">${total_equity:.0}</div>
      <div class="sub">deployed ${total_initial:.0} initial</div>
    </div>
    <div class="card">
      <div class="label">Fleet PnL</div>
      <div class="value" style="color:{pnl_col}">{pnl_sign}${total_pnl:.2}</div>
      <div class="sub" style="color:{pnl_col}">{pnl_sign}{total_pnl_pct:.2}%</div>
    </div>
    <div class="card">
      <div class="label">Open Positions</div>
      <div class="value" style="color:#e3b341">{total_open_pos}</div>
      <div class="sub">across all wallets</div>
    </div>
    <div class="card">
      <div class="label">Fleet Win Rate</div>
      <div class="value" style="color:#3fb950">{fleet_wr:.1}%</div>
      <div class="sub">{total_wins}/{total_trades} trades</div>
    </div>
    <div class="card">
      <div class="label">Circuit Breakers</div>
      <div class="value" style="color:{cb_col}">{cb_count}</div>
      <div class="sub">wallets in drawdown CB</div>
    </div>
  </div>

  <!-- ── Long/Short breakdown ── -->
  <div class="section">
    <div class="section-title">Position Direction Breakdown</div>
    <div style="max-width:500px">
      <div class="ls-row">
        <span class="ls-label" style="color:#3fb950">LONG</span>
        <div class="ls-bar-wrap">
          <div class="ls-bar-l" style="width:{long_pct:.0}%"></div>
          <div class="ls-bar-s" style="width:{short_pct:.0}%"></div>
        </div>
        <span class="ls-val" style="color:#3fb950">▲ {total_long_pos}</span>
        <span class="ls-val" style="color:#f85149">▼ {total_short_pos}</span>
      </div>
      <div style="font-size:.75em;color:#8b949e;margin-top:4px">
        Long {long_pct:.1}%  ·  Short {short_pct:.1}%
        {net_bias_note}
      </div>
    </div>
  </div>

  <!-- ── Symbol heat table + Leaderboards ── -->
  <div class="two-col">

    <div class="section">
      <div class="section-title">Most-Traded Symbols (top 25)</div>
      <div style="background:#161b22;border:1px solid #21262d;border-radius:10px;overflow:hidden">
        <table>
          <tr>
            <th>Symbol</th><th>Long</th><th>Short</th><th>Net</th><th>Bias</th>
          </tr>
          {sym_table}
        </table>
      </div>
    </div>

    <div>
      <div class="section">
        <div class="section-title">🥇 Top 10 Performers</div>
        <div style="background:#161b22;border:1px solid #21262d;border-radius:10px;overflow:hidden">
          <table>
            <tr><th>#</th><th>Wallet</th><th>Equity</th><th>PnL%</th><th>Trades</th><th>WR</th><th>Status</th></tr>
            {top10}
          </table>
        </div>
      </div>

      <div class="section">
        <div class="section-title">📉 Bottom 10 Performers</div>
        <div style="background:#161b22;border:1px solid #21262d;border-radius:10px;overflow:hidden">
          <table>
            <tr><th>#</th><th>Wallet</th><th>Equity</th><th>PnL%</th><th>Trades</th><th>WR</th><th>Status</th></tr>
            {bot10}
          </table>
        </div>
      </div>
    </div>

  </div>

  <div class="refresh-note">Page auto-refreshes every 30 s · <a href="/fleet">Refresh now</a></div>

</div>
</body>
</html>"#,
        ts = ts,
        wallet_count = wallet_count,
        total_equity = total_equity,
        total_initial = total_initial,
        total_pnl = total_pnl,
        pnl_col = pnl_col,
        pnl_sign = pnl_sign,
        total_pnl_pct = total_pnl_pct,
        total_open_pos = total_open_pos,
        fleet_wr = fleet_wr,
        total_wins = total_wins,
        total_trades = total_trades,
        cb_count = cb_count,
        cb_col = if cb_count > 50 { "#f85149" } else if cb_count > 10 { "#e3b341" } else { "#3fb950" },
        long_pct = if total_open_pos > 0 { total_long_pos as f64 / total_open_pos as f64 * 100.0 } else { 50.0 },
        short_pct = if total_open_pos > 0 { total_short_pos as f64 / total_open_pos as f64 * 100.0 } else { 50.0 },
        total_long_pos = total_long_pos,
        total_short_pos = total_short_pos,
        net_bias_note = {
            let net = total_long_pos as i64 - total_short_pos as i64;
            if net.abs() < 10 { "· Balanced".to_string() }
            else if net > 0 { format!("· Fleet leaning LONG by {} positions", net) }
            else { format!("· Fleet leaning SHORT by {} positions", net.abs()) }
        },
        sym_table = sym_table,
        top10 = top10,
        bot10 = bot10,
    ))
}

