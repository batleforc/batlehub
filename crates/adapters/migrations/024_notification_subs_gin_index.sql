-- Accelerate the array-overlap query used by get_matching_subscriptions:
--   event_types && ARRAY[$3::TEXT]
-- A B-tree cannot evaluate the && operator on TEXT[]; a GIN index can.
CREATE INDEX idx_notif_subs_event_types
    ON notification_subscriptions USING GIN (event_types);
