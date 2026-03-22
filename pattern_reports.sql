BEGIN;

-- Snapshot of the daily pattern insights that agents/alerts consume.
CREATE TABLE IF NOT EXISTS pattern_reports (
    id               UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    report_date      DATE        NOT NULL,
    guardrail_hash   TEXT,
    trade_log_hash   TEXT,
    daily_winner     TEXT,
    daily_winner_pnl NUMERIC(18,8),
    daily_loser      TEXT,
    daily_loser_pnl  NUMERIC(18,8),
    payload          JSONB       NOT NULL,
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_pattern_reports_date
    ON pattern_reports (report_date);

COMMENT ON TABLE pattern_reports IS
    'Snapshot of the latest guardrail report + signal combos stored for analytics';
COMMENT ON COLUMN pattern_reports.payload IS
    'Full pattern_insights JSON so dashboards/agents can rehydrate the data quickly';

-- Alert history so webhooks/agents can know when cache data changes.
CREATE TABLE IF NOT EXISTS pattern_cache_alerts (
    id                  BIGSERIAL PRIMARY KEY,
    pattern_report_id   UUID      NOT NULL REFERENCES pattern_reports (id) ON DELETE CASCADE,
    updated_at          TIMESTAMPTZ NOT NULL,
    winner_symbol       TEXT,
    loser_symbol        TEXT,
    payload             JSONB     NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_pattern_alerts_created
    ON pattern_cache_alerts (created_at DESC);

COMMENT ON TABLE pattern_cache_alerts IS
    'Alert history emitted after each pattern cache refresh (used by automation/webhooks)';

COMMIT;

