-- Allowed version-listing reads, per registry per hour (RFC 0006 §4.5).
--
-- A listing is not a download, and with every ecosystem's listing routes now
-- going through the proxy it happens a great deal more often than one: a
-- `cargo build` over a 400-crate graph is 400 listing fetches. One
-- `access_events` row per listing would put rows in the audit trail that
-- transferred no bytes, on the hottest path in the system.
--
-- So the allowed case is a counter and lands here, as a per-window delta
-- exactly like `hits`/`misses`. Denials keep their own `access_events` row with
-- identity, coordinate and reason — a denial is a security event that has to be
-- inspectable one at a time, and there are few of them.
ALTER TABLE stats_history
    ADD COLUMN IF NOT EXISTS listing_reads BIGINT NOT NULL DEFAULT 0;
