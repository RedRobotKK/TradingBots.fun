//! `handlers_consumer` — part of the `web_dashboard` module tree.
//!
//! Shared types and helpers available via `use super::*;`.
#![allow(unused_imports)]

use super::*;

pub(crate) fn consumer_shell_open(title: &str, active: &str) -> String {
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

pub(crate) fn consumer_shell_close() -> String {
    format!(r#"</div>
<footer style="text-align:center;padding:32px 16px 80px;font-size:.72rem;color:#484f58;
               border-top:1px solid #21262d;margin-top:24px">
  &copy; 2026 TradingBots Ltd. &nbsp;&middot;&nbsp;
  <a href="https://tradingbots.fun" style="color:#484f58;text-decoration:none">tradingbots.fun</a> &nbsp;&middot;&nbsp;
  <a href="/app/onboarding" style="color:#484f58;text-decoration:none">Terms &amp; Risk Disclosure</a>
  &nbsp;&middot;&nbsp;
  <span style="font-family:monospace;font-size:.68rem;color:#3a3f47">
    v{pkg_ver} &middot; {git_rev}
  </span>
</footer>"#,
        pkg_ver = env!("CARGO_PKG_VERSION"),
        git_rev = env!("GIT_COMMIT_HASH"),
    ) + r#"

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
pub(crate) async fn consumer_app_handler(
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
  function poll(){{fetch('/api/state').then(function(r){{return r.json();}}).then(applyPoll).catch(function(e){{/* silently degrade */}});}}
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
    html.push_str(&consumer_shell_close());
    axum::response::Html(html).into_response()
}

pub(crate) async fn agent_app_handler(
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
      // Equity = free cash + committed margin + unrealised P&L
      const _committed   = (data.positions || []).reduce((s, p) => s + (p.size_usd        || 0), 0);
      const _unrealised  = (data.positions || []).reduce((s, p) => s + (p.unrealised_pnl  || 0), 0);
      document.getElementById('equity-value').textContent = formatMoney((data.capital || 0) + _committed + _unrealised);
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

pub(crate) async fn consumer_history_handler(
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
    html.push_str(&consumer_shell_close());
    axum::response::Html(html).into_response()
}

// ─── Tax report page /app/tax ─────────────────────────────────────────────────

pub(crate) async fn consumer_tax_handler(
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

pub(crate) fn consumer_tax_page() -> axum::response::Html<String> {
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
    html.push_str(&consumer_shell_close());
    axum::response::Html(html)
}

// ─── CSV download /app/tax/csv ────────────────────────────────────────────────

pub(crate) async fn consumer_tax_csv_handler(
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
pub(crate) async fn resolve_consumer_state(
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
pub(crate) async fn auth_session_handler(
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
pub(crate) async fn auth_logout_handler(State(_app): State<AppState>) -> axum::response::Response {
    axum::response::Response::builder()
        .status(302)
        .header("Location", "/login")
        .header("Set-Cookie", crate::privy::clear_session_header())
        .body(axum::body::Body::empty())
        .unwrap()
}

// Serve the pre-built Privy SDK ESM bundle.
// Cached by the browser for 24 h; no external CDN required at runtime.
pub(crate) async fn privy_bundle_handler() -> impl axum::response::IntoResponse {
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
pub(crate) async fn login_handler(State(app): State<AppState>) -> axum::response::Html<String> {
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
pub(crate) async fn onboarding_handler(
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
pub(crate) async fn onboarding_accept_handler(
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
pub(crate) async fn consumer_settings_handler(
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
    html.push_str(&consumer_shell_close());
    axum::response::Html(html).into_response()
}

/// `POST /app/settings/wallet` — validate and store HL wallet address.
pub(crate) async fn consumer_settings_wallet_handler(
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
            Ok(_) => log::info!("🔗 Tenant {} updated wallet to {}…{}", tid, &address[..6.min(address.len())], &address[address.len().saturating_sub(4)..]),
            Err(e) => {
                log::warn!("⚠ Wallet link failed for tenant {}: {}", tid, e);
                return axum::response::Redirect::to("/app/settings?error=invalid_address")
                    .into_response();
            }
        }
    }

    axum::response::Redirect::to("/app/settings?ok=wallet_linked").into_response()
}
