//! `handlers_hl` — part of the `web_dashboard` module tree.
//!
//! Shared types and helpers available via `use super::*;`.
#![allow(unused_imports)]

use super::*;

pub(crate) async fn hl_setup_handler(
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
                log::error!("❌ HL wallet key decrypt failed for tenant {}", tid);
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
pub(crate) async fn hl_setup_complete_handler(
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
pub(crate) async fn hl_balance_api_handler(
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
pub(crate) async fn hl_export_key_handler(
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
            log::error!("❌ HL key export decrypt failed for tenant {}", tid);
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
pub(crate) struct CommandRequest {
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
pub(crate) fn parse_trade_command(cmd: &str) -> Option<BotCommand> {
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
pub(crate) async fn command_handler(
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
pub(crate) async fn thesis_get_handler(
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
pub(crate) struct ThesisCommand {
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
pub(crate) async fn thesis_update_handler(
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

