// ── Pure helper functions ─────────────────────────────────────────────────────

/// Normalise a NuGet package ID to lower-case (IDs are case-insensitive in the protocol).
pub fn normalize_id(id: &str) -> String {
    id.to_lowercase()
}
