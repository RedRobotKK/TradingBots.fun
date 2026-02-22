-- RedRobot Trading System Database Schema

CREATE TABLE IF NOT EXISTS trades (
    id BIGSERIAL PRIMARY KEY,
    trade_id VARCHAR(100) UNIQUE NOT NULL,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    action VARCHAR(20) NOT NULL,
    confidence NUMERIC(3,2),
    position_size NUMERIC(18,8),
    leverage NUMERIC(4,1),
    entry_price NUMERIC(18,8),
    stop_loss NUMERIC(18,8),
    take_profit NUMERIC(18,8),
    strategy TEXT,
    rationale TEXT,
    exit_price NUMERIC(18,8),
    exit_time TIMESTAMPTZ,
    pnl NUMERIC(18,8),
    status VARCHAR(20) DEFAULT 'OPEN'
);

CREATE TABLE IF NOT EXISTS positions (
    id BIGSERIAL PRIMARY KEY,
    symbol VARCHAR(20) NOT NULL,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    size NUMERIC(18,8),
    entry_price NUMERIC(18,8),
    current_price NUMERIC(18,8),
    pnl NUMERIC(18,8),
    leverage NUMERIC(4,1),
    health_factor NUMERIC(8,2),
    status VARCHAR(20) DEFAULT 'OPEN'
);

CREATE TABLE IF NOT EXISTS system_logs (
    id BIGSERIAL PRIMARY KEY,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    level VARCHAR(20),
    message TEXT,
    component VARCHAR(100)
);

CREATE TABLE IF NOT EXISTS whale_movements (
    id BIGSERIAL PRIMARY KEY,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    whale_address VARCHAR(200),
    token VARCHAR(50),
    action VARCHAR(100),
    amount_usd NUMERIC(18,2),
    confidence_adjustment NUMERIC(3,2)
);

-- Indices for performance
CREATE INDEX IF NOT EXISTS idx_trades_created ON trades(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_trades_status ON trades(status);
CREATE INDEX IF NOT EXISTS idx_positions_symbol ON positions(symbol);
CREATE INDEX IF NOT EXISTS idx_positions_status ON positions(status);
CREATE INDEX IF NOT EXISTS idx_system_logs_created ON system_logs(created_at DESC);
