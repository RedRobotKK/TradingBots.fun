## Automated Bridge Workflow

This project now includes a prototype bridge automation that manages Hyperliquid → Arbitrum withdrawals, validates trusted destinations, and records every movement in the existing fund tracker history.

### Environment configuration

- `BRIDGE_MIN_WITHDRAW_USD` – minimum USD you allow per automated withdrawal (default `50.0`).
- `BRIDGE_TRUSTED_DESTINATIONS` – comma-separated list of allowed destination prefixes (e.g. `0xabc...,0xdef...`). When empty, any `0x` address is accepted.

These values live alongside the other `CONFIG` env vars and are read at startup via `Config::from_env()`.

### New API endpoints

#### `POST /api/bridge/withdraw`

*Request body*
```json
{
  "amount_usd": 120.0,
  "destination": "0xabc123…"
}
```

Requires an authenticated Privy/KMS session (same cookie used across `/app/*`). The handler runs a few validations (amount vs. current HL equity, destination whitelist) and enqueues the withdrawal through `BridgeManager`. The JSON response mirrors the stored `BridgeRequestRecord` so the caller knows the ID to poll.

#### `GET /api/bridge/status/:id`

Returns the latest status for the specified `id`. Statuses are: `Pending`, `Initiated`, `Completed`, or `Failed`. Only the tenant that requested the withdrawal may view the status; other tenants receive HTTP 403.

### Prototype behaviour

1. The manager checks the HL reserved balance (`HyperliquidClient::get_account`) and ensures the requested amount is available.
2. If valid, it stores a `BridgeRequestRecord` (including tenant, destination, USD amount) and spawns an async `process_withdrawal` task.
3. `process_withdrawal` currently simulates the withdrawal (sleep + update) while recording the event via `fund_tracker::append`. A real integration would replace the `tokio::sleep` block with the Hyperliquid withdrawal API call, signing the request as required.
4. Once completed, `BridgeManager` marks the record status `Completed` and updates timestamps. Failed or insufficient-funds scenarios mark the record `Failed`.

### Manual bridging reference

The existing wallet setup page still walks users through:

1. Creating their Hyperliquid wallet and exporting the private key.
2. Funding the account via the Hyperliquid bridge (`https://app.hyperliquid.xyz/deposit`) using Arbitrum USDC.
3. Confirming the deposit by polling `/api/hl/balance`.

Use the manual flow for deposits, and rely on `POST /api/bridge/withdraw` plus `/api/bridge/status/:id` to pull funds back to a trusted Arbitrum wallet automatically. Replace the simulated step with Hyperliquid's real withdrawal endpoint when ready.
