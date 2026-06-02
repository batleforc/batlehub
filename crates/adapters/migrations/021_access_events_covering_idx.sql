-- Upgrade access_events indexes so both LATERAL sub-queries in list_packages
-- can be answered with index-only scans, eliminating heap fetches entirely.

-- (1) Add created_at as a key column so MAX(created_at) is answered by the
--     first entry of the index scan and COUNT(*) never touches the heap.
DROP INDEX IF EXISTS idx_access_events_pkg;
CREATE INDEX idx_access_events_pkg
    ON access_events (registry, package_name, package_version, created_at DESC);

-- (2) Add user_id as an INCLUDE column so the LIMIT-1 last_accessed_by lookup
--     is a true covering scan: one index entry read, no heap fetch.
DROP INDEX IF EXISTS idx_access_events_pkg_allowed_recent;
CREATE INDEX idx_access_events_pkg_allowed_recent
    ON access_events (registry, package_name, package_version, outcome, created_at DESC)
    INCLUDE (user_id);
