CREATE TABLE IF NOT EXISTS artifact_vulnerabilities (
    id            UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    artifact_key  TEXT        NOT NULL,    -- SBOM key the finding was derived from
    registry      TEXT        NOT NULL,
    package_name  TEXT        NOT NULL,
    version       TEXT        NOT NULL,
    osv_id        TEXT        NOT NULL,    -- e.g. 'RUSTSEC-2021-0001', 'GHSA-…'
    severity      TEXT        NOT NULL,    -- 'unknown' | 'low' | 'medium' | 'high' | 'critical'
    summary       TEXT        NOT NULL,
    fixed_version TEXT,                    -- first fixing version, when reported
    purl          TEXT        NOT NULL,    -- affected component PURL the finding matched
    detected_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_artifact_vuln UNIQUE (artifact_key, osv_id)
);

CREATE INDEX IF NOT EXISTS idx_artifact_vulns_coord
    ON artifact_vulnerabilities (registry, package_name, version);
CREATE INDEX IF NOT EXISTS idx_artifact_vulns_key
    ON artifact_vulnerabilities (artifact_key);
