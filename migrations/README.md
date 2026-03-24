# Database Migrations

Migrations are plain SQL files named `NNNN_description.sql` and applied in order.

To apply on the VPS:
```bash
psql $DATABASE_URL -f migrations/0021_v021_session_fields.sql
psql $DATABASE_URL -f migrations/0022_v021_hyperliquid_trade_logs.sql
psql $DATABASE_URL -f migrations/0023_v021_latency_measurements.sql
```

| File | Description |
|------|-------------|
| `0021_v021_session_fields.sql` | Adds 9 new columns to `bot_sessions` for venue, risk controls, drawdown guard, and latency config |
| `0022_v021_hyperliquid_trade_logs.sql` | New table for on-chain Hyperliquid trade records (tx_ref, coin, pnl_delta, raw_response) |
| `0023_v021_latency_measurements.sql` | New table with 5-primitive execution latency per trade + computed ms columns |
