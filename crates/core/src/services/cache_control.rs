/// Parsed directives from a `Cache-Control` header value.
#[derive(Debug, Default, Clone)]
pub struct CacheControlDirectives {
    /// `no-store`: do not persist this response anywhere.
    pub no_store: bool,
    /// `no-cache`: always revalidate before serving from cache.
    pub no_cache: bool,
    /// `max-age=<seconds>`: treat as stale after this many seconds.
    pub max_age: Option<u64>,
}

/// Parse a raw `Cache-Control` header value into its directives.
///
/// Unknown or malformed directives are silently ignored, matching typical
/// HTTP client behaviour.
pub fn parse_cache_control(header: &str) -> CacheControlDirectives {
    let mut out = CacheControlDirectives::default();
    for token in header.split(',') {
        let token = token.trim();
        if token.eq_ignore_ascii_case("no-store") {
            out.no_store = true;
        } else if token.eq_ignore_ascii_case("no-cache") {
            out.no_cache = true;
        } else if let Some((key, val)) = token.split_once('=') {
            if key.trim().eq_ignore_ascii_case("max-age") {
                if let Ok(secs) = val.trim().parse::<u64>() {
                    out.max_age = Some(secs);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_no_store() {
        let d = parse_cache_control("no-store");
        assert!(d.no_store);
        assert!(!d.no_cache);
    }

    #[test]
    fn parses_max_age() {
        let d = parse_cache_control("public, max-age=3600");
        assert!(!d.no_store);
        assert_eq!(d.max_age, Some(3600));
    }

    #[test]
    fn parses_combined() {
        let d = parse_cache_control("no-cache, max-age=0");
        assert!(d.no_cache);
        assert_eq!(d.max_age, Some(0));
    }

    #[test]
    fn empty_string_yields_all_false() {
        let d = parse_cache_control("");
        assert!(!d.no_store);
        assert!(!d.no_cache);
        assert!(d.max_age.is_none());
    }

    #[test]
    fn no_store_case_insensitive() {
        let d = parse_cache_control("NO-STORE");
        assert!(d.no_store);
    }

    #[test]
    fn unknown_directives_are_ignored() {
        let d = parse_cache_control("public, s-maxage=3600, must-revalidate");
        assert!(!d.no_store);
        assert!(!d.no_cache);
        assert!(d.max_age.is_none());
    }

    #[test]
    fn max_age_zero_is_valid() {
        let d = parse_cache_control("max-age=0");
        assert_eq!(d.max_age, Some(0));
    }

    #[test]
    fn max_age_case_insensitive() {
        let d = parse_cache_control("MAX-AGE=120");
        assert_eq!(d.max_age, Some(120));
    }

    #[test]
    fn max_age_non_numeric_is_ignored() {
        let d = parse_cache_control("max-age=abc");
        assert!(d.max_age.is_none());
    }

    #[test]
    fn whitespace_around_tokens_is_trimmed() {
        let d = parse_cache_control("  no-store  ,  max-age=60  ");
        assert!(d.no_store);
        assert_eq!(d.max_age, Some(60));
    }
}
