CREATE TABLE inbound_webhook_events (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    webhook_name    TEXT        NOT NULL,
    payload         JSONB       NOT NULL,
    source_ip       TEXT,
    received_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    signature_valid BOOLEAN
);

CREATE INDEX idx_inbound_webhook_events_name_time
    ON inbound_webhook_events (webhook_name, received_at DESC);
