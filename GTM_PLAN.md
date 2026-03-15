# TradingBots.fun — Go-To-Market Plan

> **How to read this document**
> Every task has a checkbox `[ ]`. Work through them top to bottom.
> Do not skip a phase — each one is a gate for the next.
> Commands shown in code blocks are meant to be run on your **VPS** unless labelled *(local)*.

---

## Current State (as of CI Run #40)

| Item | Status |
|---|---|
| All 431 tests passing, 0 failures | ✅ Done |
| Clippy clean, audit clean | ✅ Done |
| Paper-mode simulation working | ✅ Done |
| Live price data from Hyperliquid + Binance | ✅ Done |
| Friendly error messages for 502/429/network | ✅ Done |
| **Real order execution (`exchange.rs`)** | ❌ All stubs — no real trades placed yet |

**What "paper mode" means right now:** The bot fetches real live prices and runs all its signal/risk logic, but when it decides to trade, `exchange.rs` returns fake UUIDs instead of sending anything to Hyperliquid. No money moves. This is safe and intentional.

---

## The Three Phases

```
Phase 1  →  Testnet Paper   (env change only — price data from testnet, still no real orders)
Phase 2  →  Testnet Live    (implement real order execution, real testnet orders, $0 real risk)
Phase 3  →  Mainnet Live    (real money, real orders)
```

You must complete Phase 2 and run it for at least 72 hours before touching Phase 3.

---

## Phase 1 — Testnet Paper Trading

> **Goal:** Run the bot pointing at the testnet API so price data comes from testnet.
> The bot still won't place real orders (stubs are still in place), but you can validate
> the environment, wallet setup, and config before implementing live execution.

### 1.1 — Create a Hyperliquid testnet wallet

Hyperliquid uses standard Ethereum-compatible wallets. You need a wallet address (`0x…`) and its private key (64 hex characters). **Use a fresh wallet — never reuse a mainnet wallet for testnet.**

**Option A — MetaMask (easiest for beginners)**

- [ ] Install MetaMask: https://metamask.io/download/
- [ ] Click **Add a new account** (the circle icon → Create account)
- [ ] Name it `TradingBots.fun Testnet`
- [ ] Click the three-dot menu next to the account → **Account details** → **Show private key**
- [ ] Enter your MetaMask password
- [ ] Copy the private key (64 hex chars, no `0x` prefix needed — strip it if present)
- [ ] Copy your wallet address (`0x…` shown at the top — this is public and safe to save)

**Option B — Generate from command line (advanced, no browser needed)**

```bash
# Install cast (part of Foundry toolkit)
curl -L https://foundry.paradigm.xyz | bash
foundryup

# Generate a new wallet
cast wallet new
# Output:
#   Address:     0xABC123...
#   Private key: 0xDEF456...
```

- [ ] Save the address and private key somewhere secure (password manager, not a text file)

---

### 1.2 — Get testnet USDC from the Hyperliquid faucet

The testnet faucet gives you free fake USDC to trade with. You do not need real money.

- [ ] Go to: https://app.hyperliquid-testnet.xyz
- [ ] Click **Connect Wallet** → connect your testnet MetaMask account
  *(If using Option B above: import the private key into MetaMask first via Import Account)*
- [ ] Once connected, click your address in the top right → **Deposit**
- [ ] The testnet deposit page has a **"Get testnet funds"** or **faucet** button — click it
- [ ] You will receive **10,000 USDC** (fake) — this is more than enough
- [ ] Verify balance shows on the dashboard before continuing

> **Tip:** If the faucet button is not visible on the UI, join the Hyperliquid Discord
> (https://discord.gg/hyperliquid) and use the `#testnet-faucet` channel:
> post your wallet address and a bot will send funds within a few seconds.

---

### 1.3 — Understand Hyperliquid API keys

Unlike Binance, **Hyperliquid does not issue separate API keys.** Your wallet IS your API credential:

| What you call it | What it actually is |
|---|---|
| `HYPERLIQUID_KEY` | Your wallet address (`0x…`) |
| `HYPERLIQUID_SECRET` | Your wallet private key (hex, 64 chars) |

The bot signs every API request using your private key (EIP-712 Ethereum signature). This is the standard Web3 authentication pattern — no separate key generation step needed.

---

### 1.4 — Update the `.env` file on your VPS

SSH into your VPS, then:

```bash
cd ~/tradingbots-fun       # or wherever the repo lives
cp .env .env.backup           # always back up before editing
nano .env
```

Change or add these lines (replace placeholder values with your real ones):

```env
# Trading mode — 'testnet' uses https://api.hyperliquid-testnet.xyz
MODE=testnet

# Starting capital for the paper simulation (fake USDC amount)
INITIAL_CAPITAL=1000.0

# Your testnet wallet address (public — safe in this file)
HYPERLIQUID_KEY=0xYOUR_TESTNET_WALLET_ADDRESS_HERE

# Your testnet private key — 64 hex chars, no 0x prefix
# ⛔  NEVER commit this file to GitHub
HYPERLIQUID_SECRET=YOUR_64_CHAR_HEX_PRIVATE_KEY_HERE

# Logging level (use debug during testnet to see everything)
RUST_LOG=debug
```

Save and exit (`Ctrl+O`, `Enter`, `Ctrl+X` in nano).

**Lock down the file permissions so only your user can read it:**

```bash
chmod 600 .env
```

Verify `.env` is in `.gitignore` (it should already be):

```bash
grep ".env" .gitignore
# Expected output:  .env
```

---

### 1.5 — Confirm `.env` is not tracked by git

```bash
git status
# .env must NOT appear in the output — if it does, run:
git rm --cached .env
echo ".env" >> .gitignore
git add .gitignore && git commit -m "chore: ensure .env is gitignored"
```

---

### 1.6 — Restart the bot and verify testnet mode

```bash
sudo systemctl restart tradingbots-fun
sudo journalctl -u tradingbots-fun -f
```

Look for this line in the logs within the first 5 seconds:

```
✓ Config: mode=Testnet  capital=$1000  paper=false
```

And confirm price fetches are reaching the testnet endpoint:

```
📡 Fetching prices…
```

If you see `mode=Paper` instead of `mode=Testnet`, the `.env` change hasn't been picked up — double-check there is no stray `MODE=paper` override in your systemd unit file:

```bash
sudo systemctl cat tradingbots-fun | grep -i mode
# If MODE appears there, edit the unit file and remove it:
sudo systemctl edit tradingbots-fun
```

---

### Phase 1 Complete Checklist

- [ ] Testnet wallet created (address saved)
- [ ] Private key saved securely (password manager)
- [ ] 10,000 testnet USDC received from faucet
- [ ] `.env` updated with `MODE=testnet`, `HYPERLIQUID_KEY`, `HYPERLIQUID_SECRET`
- [ ] `.env` permissions set to `600`
- [ ] `.env` confirmed absent from `git status`
- [ ] Bot restarted, logs show `mode=Testnet`
- [ ] Price data flowing (no errors in logs for 10+ minutes)

---

## Phase 2 — Testnet Live Trading (Real Orders on Testnet)

> **Goal:** Replace the 4 stub functions in `src/exchange.rs` with real Hyperliquid API calls.
> Orders will be placed on testnet — fake USDC only, zero real-money risk.
> This is the most technically complex phase.

### 2.1 — Understand what needs to be implemented

Open `src/exchange.rs`. There are 4 stub functions to replace:

| Function | What it must do |
|---|---|
| `get_account()` | POST `/info` with `clearinghouseState` to get real balance |
| `place_order()` | EIP-712 sign + POST `/exchange` to place a real order |
| `get_positions()` | Parse `assetPositions` from the clearinghouse response |
| `close_position()` | Place a reduce-only market order to close an open position |

**Reference documentation:**
- Hyperliquid API overview: https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api
- Exchange endpoint (orders): https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/exchange-endpoint
- Info endpoint (account/positions): https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/info-endpoint

---

### 2.2 — Add required Rust dependencies

In `Cargo.toml`, verify or add these dependencies (needed for EIP-712 signing):

```toml
[dependencies]
ethers = { version = "2", features = ["rustls"] }
# OR the lighter alternative:
alloy-signer = "0.1"
alloy-primitives = "0.7"
```

- [ ] Run `cargo add ethers --features rustls` (or the alloy equivalent)
- [ ] Run `cargo build` to confirm it compiles before writing any logic

---

### 2.3 — Implement `get_account()`

Replace the stub with a real POST to `/info`:

```
POST https://api.hyperliquid-testnet.xyz/info
Body: { "type": "clearinghouseState", "user": "0xYOUR_WALLET_ADDRESS" }
```

Parse the response fields:
- `marginSummary.accountValue` → `equity`
- `marginSummary.crossMaintenanceMarginUsed` → `margin`
- `crossMaintenanceMarginUsed / accountValue` → derive `health_factor`

- [ ] Implement `get_account()` with real HTTP call
- [ ] Test with: `cargo test test_get_account -- --nocapture` (add this test)

---

### 2.4 — Implement `get_positions()`

From the same `clearinghouseState` response, parse:

```
response["assetPositions"]  →  array of position objects
each item["position"]["coin"]          → symbol
each item["position"]["szi"]           → size (positive=long, negative=short)
each item["position"]["entryPx"]       → entry price
each item["position"]["positionValue"] → current value
```

- [ ] Implement `get_positions()` parsing `assetPositions`
- [ ] Test: open a manual position on testnet UI, confirm `get_positions()` returns it

---

### 2.5 — Implement `place_order()` with EIP-712 signing

This is the most complex step. Hyperliquid requires orders to be signed using the EIP-712 structured data standard.

Steps:
1. Build the order struct (coin, is_buy, limit_px, sz, reduce_only, order_type, cloid)
2. Construct the EIP-712 domain + type hash
3. Sign with your private key using `ethers::signers::LocalWallet`
4. POST to `/exchange` with the signed action

The exact type hash and domain for Hyperliquid is documented at:
https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/exchange-endpoint#signing

- [ ] Implement the EIP-712 signing helper
- [ ] Implement `place_order()` calling POST `/exchange`
- [ ] Add a test that places a tiny 0.001 BTC market order on testnet and confirms order ID returned
- [ ] Verify the order appears in the testnet UI at https://app.hyperliquid-testnet.xyz

---

### 2.6 — Implement `close_position()`

This is a `place_order()` call with `reduce_only: true` and direction opposite to the open position:

```
if position.size > 0  →  sell (is_buy: false)
if position.size < 0  →  buy  (is_buy: true)
size = position.size.abs()
```

- [ ] Implement `close_position()` as reduce-only market order
- [ ] Test: open a position manually, call `close_position()`, verify it closes

---

### 2.7 — Connect `exchange.rs` to the main decision loop

Currently `main.rs` calls `hl.place_order(&decision).await` but ignores the real result because it's a stub. Once the stubs are replaced:

- [ ] Review `run_cycle()` in `main.rs` to confirm order placement paths
- [ ] Add logging for successful order IDs (e.g. `info!("✅ Order placed: {}", order_id)`)
- [ ] Add logging for position opens and closes to the trade logger

---

### 2.8 — Run on testnet for 72 hours straight

- [ ] Deploy updated code: `git push origin master` → wait for CI to pass → `sudo systemctl restart tradingbots-fun`
- [ ] Monitor logs: `sudo journalctl -u tradingbots-fun -f`
- [ ] Confirm real orders appearing at: https://app.hyperliquid-testnet.xyz (Order History tab)
- [ ] Run for minimum **72 consecutive hours**
- [ ] Check daily analysis reports in `logs/analysis_YYYY-MM-DD.md`

---

### 2.9 — Testnet success criteria (all must pass before Phase 3)

- [ ] Zero unhandled panics in 72 hours
- [ ] Zero runaway order loops (same symbol ordered repeatedly)
- [ ] Daily loss limit fires correctly (check logs on a high-volatility day)
- [ ] Circuit breaker activates when drawdown exceeds 8% (simulate by lowering `INITIAL_CAPITAL` briefly)
- [ ] Position close logic works — no orphaned open positions after stop-outs
- [ ] Win rate ≥ 50% over 72 hours (check `logs/analysis_*.md`)
- [ ] Max drawdown < 15% over 72 hours
- [ ] All positions have correct stop-losses attached

---

### Phase 2 Complete Checklist

- [ ] All 4 stubs in `exchange.rs` replaced with real API calls
- [ ] EIP-712 signing implemented and tested
- [ ] Unit tests added for each new function
- [ ] CI still passing (`cargo test` + `cargo clippy`)
- [ ] 72-hour testnet run completed
- [ ] All success criteria met (section 2.9)
- [ ] Daily analysis reports reviewed and look reasonable

---

## Phase 3 — Mainnet Go-Live

> **Goal:** Switch to real money. Do this only after Phase 2 success criteria are all checked.
> Start with the minimum viable capital. You can always add more — you cannot un-lose it.

### 3.1 — Create a dedicated mainnet wallet

**Do not reuse your testnet wallet for mainnet.** Create a fresh wallet following the same steps as section 1.1 (Option A or B).

- [ ] New mainnet wallet created
- [ ] Address and private key stored in password manager
- [ ] Seed phrase written on paper and stored in a physically secure location (not digitally)
- [ ] Wallet address noted here (safe to record): `0x________________________________`

---

### 3.2 — Fund the mainnet wallet

Hyperliquid trades with USDC on Arbitrum. You need to bridge USDC onto Hyperliquid.

**Minimum recommended starting capital: $500 USDC**
(The risk system sizes positions at 1–2% per trade. Below $500 you hit exchange minimum order sizes.)

**Step-by-step funding:**

- [ ] Buy USDC on a centralised exchange (Coinbase, Kraken, Binance)
  - Transfer to your mainnet wallet address on the **Arbitrum network**
  - (Arbitrum is cheaper gas than Ethereum mainnet — Hyperliquid bridges from Arbitrum)
- [ ] Go to https://app.hyperliquid.xyz → Connect your mainnet wallet → Deposit
- [ ] Deposit your USDC amount
- [ ] Confirm balance appears on the Hyperliquid portfolio page before continuing

---

### 3.3 — Update `.env` on the VPS for mainnet

```bash
cd ~/tradingbots-fun
cp .env .env.testnet-backup     # keep testnet config as backup
nano .env
```

Change these values:

```env
# Switch to mainnet
MODE=mainnet

# Start conservatively — you can raise this later
INITIAL_CAPITAL=500.0

# Your MAINNET wallet address (different from testnet)
HYPERLIQUID_KEY=0xYOUR_MAINNET_WALLET_ADDRESS

# Your MAINNET private key — 64 hex chars
HYPERLIQUID_SECRET=YOUR_MAINNET_64_CHAR_HEX_PRIVATE_KEY

# Drop to info level for production (less noise)
RUST_LOG=info
```

```bash
chmod 600 .env
```

---

### 3.4 — Pre-launch safety checks

Run each of these before starting the bot on mainnet:

- [ ] Confirm the testnet wallet and mainnet wallet are different addresses
- [ ] Confirm `INITIAL_CAPITAL` in `.env` matches the USDC you actually deposited
- [ ] Confirm `DAILY_LOSS_LIMIT` is set conservatively (suggest `50.0` = $50/day max loss to start)
- [ ] Confirm `MAX_LEVERAGE` is set to `3.0` or lower for the first week (edit `.env`)
- [ ] Read through the last 3 testnet daily analysis reports — no red flags?
- [ ] Confirm you can SSH into the VPS and tail logs at any time

---

### 3.5 — Launch on mainnet

```bash
sudo systemctl restart tradingbots-fun
sudo journalctl -u tradingbots-fun -f
```

Confirm in the logs:

```
✓ Config: mode=Mainnet  capital=$500  paper=false
```

- [ ] First cycle completes without error (wait 60 seconds)
- [ ] Check the Hyperliquid mainnet portfolio page — no unexpected orders or positions
- [ ] Watch for the first real trade being placed and verify it appears on Hyperliquid UI

---

### 3.6 — Week 1 monitoring protocol

For the first 7 days, check in twice daily:

**Morning check (5 minutes):**
```bash
sudo journalctl -u tradingbots-fun --since "yesterday" | grep -E "ERROR|WARN|order|position"
cat logs/analysis_$(date -d yesterday +%Y-%m-%d).md   # yesterday's AI report
```

**Evening check (5 minutes):**
```bash
sudo journalctl -u tradingbots-fun --since "12 hours ago" | tail -50
```

- [ ] Day 1: No errors, first orders placed ✓
- [ ] Day 2: Daily loss limit not hit ✓
- [ ] Day 3: Review P&L in daily analysis ✓
- [ ] Day 5: Win rate > 50%? If not, investigate before continuing ✓
- [ ] Day 7: Full week review — compare actual vs backtest expectations ✓

---

### 3.7 — Scaling up capital (after 2 profitable weeks)

- [ ] Two consecutive profitable weeks confirmed
- [ ] Max drawdown < 10% over those 2 weeks
- [ ] No incident of position stuck open or missed stop-loss
- [ ] Increase `INITIAL_CAPITAL` in `.env` by no more than 2× at a time
- [ ] Restart service after each capital change

---

### Phase 3 Complete Checklist

- [ ] Mainnet wallet created and funded
- [ ] `.env` updated for mainnet, permissions set to 600
- [ ] Pre-launch checks passed (section 3.4)
- [ ] Bot live on mainnet, first trades placed
- [ ] Week 1 monitoring protocol followed
- [ ] Ready to scale up (after 2 profitable weeks)

---

## Ongoing Operations

### How to deploy code updates

```bash
# On your Mac (local):
git add -A && git commit -m "your message"
git push origin master

# CI runs on the VPS automatically — wait for it to pass, then:
# On VPS:
sudo systemctl restart tradingbots-fun
sudo journalctl -u tradingbots-fun -f   # watch for clean startup
```

### How to check if the bot is healthy

```bash
# Is the service running?
sudo systemctl status tradingbots-fun

# Recent errors?
sudo journalctl -u tradingbots-fun --since "1 hour ago" | grep ERROR

# Live log stream
sudo journalctl -u tradingbots-fun -f
```

### How to run the daily analysis manually

```bash
cd ~/tradingbots-fun
ANTHROPIC_API_KEY=your_key ./target/release/tradingbots --analyze 2026-03-14
# Output written to:  logs/analysis_2026-03-14.md
```

### How to emergency stop

```bash
sudo systemctl stop tradingbots-fun
# Then manually close any open positions on the Hyperliquid UI
```

### Environment variable reference

| Variable | Required | Example | Notes |
|---|---|---|---|
| `MODE` | Yes | `testnet` / `mainnet` / `paper` | Controls API endpoint |
| `INITIAL_CAPITAL` | Yes | `1000.0` | Starting USDC amount |
| `HYPERLIQUID_KEY` | For testnet/mainnet | `0xABC…` | Your wallet address |
| `HYPERLIQUID_SECRET` | For testnet/mainnet | `abc123…` (64 hex) | Your private key — keep secret |
| `RUST_LOG` | No | `info` | Use `debug` for troubleshooting |
| `ANTHROPIC_API_KEY` | No | `sk-ant-…` | Enables daily AI analysis |
| `LUNARCRUSH_API_KEY` | No | (has default) | Sentiment data |
| `DATABASE_URL` | No | `sqlite:./tradingbots.db` | Defaults to SQLite |

---

## Risk Warnings

- Never put more money into the bot than you can afford to lose entirely.
- The bot is a tool, not a guarantee. Algorithmic trading can lose money.
- Always keep enough USDC outside the bot to cover exchange fees and gas.
- A private key exposed even briefly (in logs, screenshots, git commits) means the wallet is compromised — move funds immediately if this happens.
- Do not run two instances of the bot simultaneously against the same wallet — they will conflict on position management.

---

*Last updated: 2026-03-14 — CI Run #40 baseline*
