-- Add IP address and User-Agent to access_events for SOC 2 compliance.
-- Both columns are nullable: events recorded before this migration, and events
-- from proxy paths that do not capture the caller's network information, will
-- have NULL in both columns.
ALTER TABLE access_events ADD COLUMN ip_address TEXT;
ALTER TABLE access_events ADD COLUMN user_agent TEXT;
