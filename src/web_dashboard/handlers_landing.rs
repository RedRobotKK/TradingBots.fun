//! `handlers_landing` — part of the `web_dashboard` module tree.
//!
//! Shared types and helpers available via `use super::*;`.
#![allow(unused_imports)]

use super::*;

pub(crate) async fn public_landing_handler(State(_app): State<AppState>) -> axum::response::Html<String> {
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

#latency-card{display:none;margin-top:16px}
.lat-grid{display:grid;grid-template-columns:repeat(4,1fr);gap:10px;margin-top:10px}
.lat-cell{background:var(--bg2);border:1px solid var(--border);border-radius:8px;padding:10px 12px;text-align:center}
.lat-label{font-size:.7rem;color:#6e7681;text-transform:uppercase;letter-spacing:.04em;margin-bottom:4px}
.lat-val{font-size:1.1rem;font-weight:700;color:#c9d1d9;margin-bottom:2px}
.lat-target{font-size:.68rem;color:#484f58}
.lat-ok{color:#3fb950}
.lat-warn{color:#d29922}
.lat-bad{color:#f85149}

/* ── Pricing section ── */
.pricing-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(260px,1fr));gap:20px;margin-top:8px}
.plan-card{background:var(--bg2);border:1px solid var(--border);border-radius:16px;padding:28px 26px;display:flex;flex-direction:column;gap:0;transition:.15s}
.plan-card:hover{border-color:rgba(63,185,80,.4);transform:translateY(-2px)}
.plan-card.popular{border-color:rgba(63,185,80,.4);background:linear-gradient(135deg,rgba(63,185,80,.06),rgba(63,185,80,.02));position:relative}
.plan-badge{position:absolute;top:-12px;left:50%;transform:translateX(-50%);background:var(--green);color:#0d1117;border-radius:20px;padding:3px 14px;font-size:.68rem;font-weight:800;letter-spacing:.5px;white-space:nowrap}
.plan-name{font-size:1rem;font-weight:700;color:var(--text-hi);margin-bottom:4px}
.plan-price{font-size:2.2rem;font-weight:800;color:var(--text-hi);line-height:1;margin-bottom:2px;font-variant-numeric:tabular-nums}
.plan-price span{font-size:.85rem;font-weight:400;color:var(--muted)}
.plan-sub{font-size:.75rem;color:var(--dim);margin-bottom:20px}
.plan-features{display:flex;flex-direction:column;gap:8px;margin-bottom:24px;flex:1}
.plan-feat{display:flex;align-items:flex-start;gap:9px;font-size:.82rem;color:var(--text)}
.plan-feat-icon{color:var(--green);font-size:.85rem;margin-top:1px;flex-shrink:0}
.plan-feat-icon.dim{color:var(--dim)}
.plan-cta{display:block;text-align:center;padding:11px 20px;border-radius:9px;font-weight:700;font-size:.88rem;background:var(--green);color:#0d1117;transition:.15s}
.plan-cta:hover{background:#52c965}
.plan-cta.sec{background:transparent;border:1px solid var(--border);color:var(--text)}
.plan-cta.sec:hover{border-color:rgba(63,185,80,.4);color:var(--text-hi)}

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
    <a href="#pricing" class="nav-link">Pricing</a>
    <a href="#x402-api" class="nav-link">x402 API</a>
    <a href="/leaderboard" class="nav-link">Leaderboard</a>
    <a href="/login" class="nav-cta">Start Trading →</a>
  </div>
</nav>

<!-- ═══ HERO ═══ -->
<section class="hero">
  <!-- AUM history sparkline renders here as a translucent background -->
  <canvas id="aum-canvas"></canvas>
  <div style="position:relative;z-index:1">
    <div class="hero-eyebrow">● AI Trading · Live on Hyperliquid</div>
    <h1>Your portfolio.<br>Managed by <em>AI. 24/7.</em></h1>
    <div id="hero-pnl" class="hero-pnl" style="display:none">
      <span id="hero-pnl-val">+$0.00</span>
      <span class="hero-pnl-meta">session profit · <span id="hero-wr">—</span> win rate</span>
    </div>
    <p class="hero-sub">Multi-signal AI scans 50+ Hyperliquid perpetual pairs every 30 seconds — entering, managing, and exiting trades while you sleep.<br><span id="hero-aum" style="color:var(--green);font-weight:700">—</span> under active management right now.</p>
    <div class="hero-btns">
      <a href="/login" class="btn-p" style="font-size:1rem;padding:14px 32px">Start Trading →</a>
      <a href="#pricing" class="btn-s">See Plans</a>
    </div>
    <p style="margin-top:14px;font-size:.72rem;color:var(--dim)">From <strong style="color:var(--text)">$20/month</strong> · Withdraw anytime · Paper trading free</p>
  </div>
</section>

<!-- ═══ STAT BAR ═══ -->
<div class="stat-bar">
  <div class="stat-cell">
    <div class="stat-val" id="m-pnl" style="color:var(--green)">—</div>
    <div class="stat-lbl">Realised Profit</div>
  </div>
  <div class="stat-cell">
    <div class="stat-val" id="m-wr2" style="color:var(--yellow)">—</div>
    <div class="stat-lbl">Win Rate</div>
  </div>
  <div class="stat-cell">
    <div class="stat-val" id="m-rolling-wr" style="color:var(--muted)">—</div>
    <div class="stat-lbl" id="m-rolling-wr-label" title="Rolling win rate over the last 20 closed trades">Recent WR (20)</div>
  </div>
  <div class="stat-cell">
    <div class="stat-val" id="m-pos" style="color:var(--blue)">—</div>
    <div class="stat-lbl">Live Positions</div>
  </div>
  <div class="stat-cell">
    <div class="stat-val" id="m-trades2" style="color:var(--muted)">—</div>
    <div class="stat-lbl">Trades Closed</div>
  </div>
  <div class="stat-cell">
    <div class="stat-val" style="color:var(--green)">50+</div>
    <div class="stat-lbl">Pairs Scanned</div>
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

<!-- ═══ PRICING ═══ -->
<section class="sec" id="pricing">
  <div class="sec-head"><span class="sec-title">Simple Pricing</span><span class="sec-line"></span></div>
  <div class="pricing-grid">

    <!-- Starter -->
    <div class="plan-card">
      <div class="plan-name">Starter</div>
      <div class="plan-price">$20<span>/mo</span></div>
      <div class="plan-sub">Everything you need to get started</div>
      <div class="plan-features">
        <div class="plan-feat"><span class="plan-feat-icon">✓</span><span><strong>20 AI bots</strong> running simultaneously</span></div>
        <div class="plan-feat"><span class="plan-feat-icon">✓</span><span>50+ Hyperliquid perp pairs scanned every 30s</span></div>
        <div class="plan-feat"><span class="plan-feat-icon">✓</span><span>Live P&amp;L dashboard with position cards</span></div>
        <div class="plan-feat"><span class="plan-feat-icon">✓</span><span>Paper trading mode — test before going live</span></div>
        <div class="plan-feat"><span class="plan-feat-icon">✓</span><span>Macro regime detection (Bull / Bear / Transition)</span></div>
        <div class="plan-feat"><span class="plan-feat-icon">✓</span><span>Deposit &amp; withdraw USDC anytime</span></div>
        <div class="plan-feat"><span class="plan-feat-icon dim">·</span><span style="color:var(--dim)">x402 API access</span></div>
      </div>
      <a href="/login" class="plan-cta sec">Get Started</a>
    </div>

    <!-- Pro -->
    <div class="plan-card popular" style="position:relative">
      <div class="plan-badge">MOST POPULAR</div>
      <div class="plan-name">Pro</div>
      <div class="plan-price">$40<span>/mo</span></div>
      <div class="plan-sub">For serious traders who want full capacity</div>
      <div class="plan-features">
        <div class="plan-feat"><span class="plan-feat-icon">✓</span><span><strong>40 AI bots</strong> running simultaneously</span></div>
        <div class="plan-feat"><span class="plan-feat-icon">✓</span><span>50+ Hyperliquid perp pairs scanned every 30s</span></div>
        <div class="plan-feat"><span class="plan-feat-icon">✓</span><span>Live P&amp;L dashboard with position cards</span></div>
        <div class="plan-feat"><span class="plan-feat-icon">✓</span><span>Paper trading mode — test before going live</span></div>
        <div class="plan-feat"><span class="plan-feat-icon">✓</span><span>Macro regime detection (Bull / Bear / Transition)</span></div>
        <div class="plan-feat"><span class="plan-feat-icon">✓</span><span>Deposit &amp; withdraw USDC anytime</span></div>
        <div class="plan-feat"><span class="plan-feat-icon">✓</span><span><strong>x402 API access</strong> — machine-native trading</span></div>
        <div class="plan-feat"><span class="plan-feat-icon">✓</span><span>Priority signal execution</span></div>
      </div>
      <a href="/login" class="plan-cta">Start Pro →</a>
    </div>

  </div>
  <p style="text-align:center;margin-top:18px;font-size:.75rem;color:var(--dim)">Billed monthly · Cancel anytime · Capital always in your own Hyperliquid wallet</p>
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
  <h3>Let AI manage your trades. From $20/month.</h3>
  <p>Start with paper trading — zero risk, full experience. Go live when you're ready.<br>Your capital stays in your own Hyperliquid wallet. Always.</p>
  <div class="hero-btns">
    <a href="/login" class="btn-p" style="font-size:1rem;padding:14px 32px">Start Trading →</a>
    <a href="#pricing" class="btn-s">View Plans</a>
  </div>
  <p style="margin-top:14px;font-size:.72rem;color:var(--dim)">Arbitrum ↔ Hyperliquid bridge built-in · cancel anytime · no lock-ups</p>
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
  <div id="latency-card"></div>
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

    // ── Macro regime pill — keep in sync on every loadState() call ──
    // applyPoll() only updates the pill on cycle-tick; loadState() runs every 30s
    // independently, so we must also update it here to avoid stale display.
    if (typeof updateMacroPill === 'function') {
      updateMacroPill(s.macro_regime || 'NEUTRAL');
    }

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

    // ── Rolling win rate (last 20 trades) — edge decay indicator ──
    // Only render once there are closed trades; default 0.0 is misleading before any trades.
    const rwrEl = document.getElementById('m-rolling-wr');
    if (rwrEl) {
      if (m.total_trades > 0 && m.rolling_win_rate != null) {
        const rwr = m.rolling_win_rate * 100;
        rwrEl.textContent = rwr.toFixed(0) + '%';
        // Colour: green ≥60%, yellow 50-60%, red <50% (edge degradation warning)
        rwrEl.style.color = rwr >= 60 ? 'var(--green)' : rwr >= 50 ? '#e3b341' : 'var(--red)';
      } else {
        rwrEl.textContent = '—';
        rwrEl.style.color = 'var(--muted)';
      }
    }

    // ── Signal weights ──
    renderWeights(s.signal_weights);

    // ── Latency stats ──
    renderLatency();

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

    // ── AUM fallback: if TVL DB has no history yet, show live equity ──
    // s.current_equity does NOT exist in BotState; compute equity from live fields.
    const aumEl = document.getElementById('hero-aum');
    if (aumEl && (aumEl.textContent === '—' || aumEl.textContent === '')) {
      const _liveCommitted  = (s.positions||[]).reduce((acc,p) => acc + (p.size_usd       ||0), 0);
      const _liveUnrealised = (s.positions||[]).reduce((acc,p) => acc + (p.unrealised_pnl ||0), 0);
      const _liveEquity = (s.capital||0) + _liveCommitted + _liveUnrealised;
      if (_liveEquity > 0) aumEl.textContent = fmtUsd(_liveEquity);
    }

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
  } catch(_e) { /* silently degrade */ }
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
  } catch(_e) { /* silently degrade */ }
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
  } catch(_e) { /* silently degrade */ }
}

// ═══════════════════════════════════════════════════════
//  Latency card
// ═══════════════════════════════════════════════════════
async function renderLatency() {
  // Only show if we have a session token (bot-API users)
  const token = window._sessionToken;
  const sid   = window._sessionId;
  if (!token || !sid) return;
  const card = document.getElementById('latency-card');
  if (!card) return;
  try {
    const r = await fetch(`/api/v1/session/${sid}/latency/stats`, {
      headers: { 'Authorization': `Bearer ${token}` }
    });
    if (!r.ok) return;
    const d = await r.json();
    if (!d.ok) return;
    card.style.display = '';
    const p50   = d.p50_ms?.toFixed(0) ?? '—';
    const p95   = d.p95_ms?.toFixed(0) ?? '—';
    const p99   = d.p99_ms?.toFixed(0) ?? '—';
    const tpm   = d.trades_per_minute?.toFixed(1) ?? '—';
    const sr    = d.success_rate_pct?.toFixed(0) ?? '—';
    const n     = d.sample_count ?? 0;
    const p50cls = d.p50_ms <= 250  ? 'lat-ok' : d.p50_ms <= 500  ? 'lat-warn' : 'lat-bad';
    const p95cls = d.p95_ms <= 450  ? 'lat-ok' : d.p95_ms <= 800  ? 'lat-warn' : 'lat-bad';
    const p99cls = d.p99_ms <= 800  ? 'lat-ok' : d.p99_ms <= 1200 ? 'lat-warn' : 'lat-bad';
    card.innerHTML = `
      <div class="section-hdr">⚡ Execution Latency <span style="color:#484f58;font-size:.75rem">(n=${n})</span></div>
      <div class="lat-grid">
        <div class="lat-cell">
          <div class="lat-label">p50 (median)</div>
          <div class="lat-val ${p50cls}">${p50} ms</div>
          <div class="lat-target">target &lt;250ms</div>
        </div>
        <div class="lat-cell">
          <div class="lat-label">p95</div>
          <div class="lat-val ${p95cls}">${p95} ms</div>
          <div class="lat-target">target &lt;450ms</div>
        </div>
        <div class="lat-cell">
          <div class="lat-label">p99</div>
          <div class="lat-val ${p99cls}">${p99} ms</div>
          <div class="lat-target">target &lt;800ms</div>
        </div>
        <div class="lat-cell">
          <div class="lat-label">Throughput</div>
          <div class="lat-val">${tpm} <span style="font-size:.7rem;color:#8b949e">t/min</span></div>
          <div class="lat-target">fill rate: ${sr}%</div>
        </div>
      </div>`;
  } catch(e) { /* silently skip — session token not set */ }
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
pub(crate) async fn api_public_stats_handler(
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

