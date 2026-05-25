-- Tracks per-package ownership for private/local registries.
-- The first publisher of a package is automatically granted the 'admin' role.
-- Admins may add or remove other owners (users or groups) with 'maintainer' access.
CREATE TABLE IF NOT EXISTS package_owners (
    id             SERIAL PRIMARY KEY,
    registry       TEXT NOT NULL,
    package_name   TEXT NOT NULL,
    principal_type TEXT NOT NULL CHECK (principal_type IN ('user', 'group')),
    principal_id   TEXT NOT NULL,
    role           TEXT NOT NULL DEFAULT 'maintainer'
                       CHECK (role IN ('admin', 'maintainer')),
    granted_by     TEXT,
    granted_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_package_owner
        UNIQUE (registry, package_name, principal_type, principal_id)
);

CREATE INDEX IF NOT EXISTS idx_package_owners_lookup
    ON package_owners (registry, package_name);
