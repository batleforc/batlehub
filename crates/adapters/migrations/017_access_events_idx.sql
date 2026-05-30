-- Composite indexes on access_events to speed up the list_packages JOIN and
-- correlated last_accessed_by subquery, which previously scanned the whole table.
--
-- (registry, package_name, package_version) covers the JOIN condition in list_packages.
CREATE INDEX IF NOT EXISTS idx_access_events_pkg
    ON access_events (registry, package_name, package_version);

-- Covering index for the last_accessed_by correlated subquery:
--   WHERE registry=? AND package_name=? AND package_version=? AND outcome='allowed'
--   ORDER BY created_at DESC LIMIT 1
CREATE INDEX IF NOT EXISTS idx_access_events_pkg_allowed_recent
    ON access_events (registry, package_name, package_version, outcome, created_at DESC);

-- (registry, package_name) covers the explore collapsed-list LATERAL subquery.
CREATE INDEX IF NOT EXISTS idx_access_events_registry_name
    ON access_events (registry, package_name);

-- Composite index on package_statuses(registry, package_name) for the explore GROUP BY.
CREATE INDEX IF NOT EXISTS idx_package_statuses_registry_name
    ON package_statuses (registry, package_name);
