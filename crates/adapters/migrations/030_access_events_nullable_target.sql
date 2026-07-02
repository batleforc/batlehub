-- Allow account-wide / network-wide audit events (e.g. blocking a user or an
-- IP address) that are not scoped to any specific package coordinate.
-- Package-scoped events (download, block, add_owner, ...) continue to carry
-- registry/package_name/package_version as before; only the column
-- constraints are relaxed so a NULL coordinate becomes representable.
ALTER TABLE access_events ALTER COLUMN registry DROP NOT NULL;
ALTER TABLE access_events ALTER COLUMN package_name DROP NOT NULL;
ALTER TABLE access_events ALTER COLUMN package_version DROP NOT NULL;
