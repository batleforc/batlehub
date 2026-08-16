-- The licence a package's own manifest declared, read by LicenseGateRule on
-- the request path (RFC 0004-bis §13.1).
--
-- A column rather than a read into `document`: the rule queries it per request,
-- `document` is an opaque JSON blob whose shape differs between SPDX and
-- CycloneDX, and for `source = 'upstream'` it is a document BatleHub did not
-- write and cannot rely on the shape of.
--
-- Nullable, and NULL means *unknown*, never "no licence": rows written before
-- this migration have no extraction behind them, and sixteen of the twenty-one
-- registry types have no manifest parser at all. `license_gate.allow_unknown`
-- is what decides how that is treated.
ALTER TABLE artifact_sboms ADD COLUMN IF NOT EXISTS license TEXT;

-- Supports the gate's lookup, which is by coordinate and not by artifact_key —
-- proxy keys carry a per-registry artifact suffix the rule cannot predict.
-- Partial, because rows with no licence are never the answer to that query.
CREATE INDEX IF NOT EXISTS idx_artifact_sboms_license_coord
    ON artifact_sboms (registry, package_name, version)
    WHERE license IS NOT NULL;
