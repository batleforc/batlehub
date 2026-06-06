CREATE TABLE notification_subscriptions (
    id           UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    registry     TEXT,
    package_name TEXT,
    event_types  TEXT[]      NOT NULL,
    channel_name TEXT        NOT NULL,
    created_by   TEXT        NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    enabled      BOOLEAN     NOT NULL DEFAULT TRUE
);

CREATE INDEX idx_notif_subs_lookup
    ON notification_subscriptions (registry, package_name)
    WHERE enabled = TRUE;
