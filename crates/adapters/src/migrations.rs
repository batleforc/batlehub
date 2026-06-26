// Embedded SQL migrator — avoids the sqlx `macros` feature which transitively
// pulls in sqlx-mysql → rsa (RUSTSEC-2023-0071, no upstream fix available).
// Migration::new() computes the SHA-384 checksum automatically, matching
// what `sqlx::migrate!()` would have embedded.

use sqlx::migrate::{Migration, MigrationType, Migrator};
use std::borrow::Cow;

macro_rules! mig {
    ($ver:expr, $desc:literal, $path:literal) => {
        Migration::new(
            $ver,
            Cow::Borrowed($desc),
            MigrationType::Simple,
            Cow::Borrowed(include_str!($path)),
            false,
        )
    };
}

/// Build the embedded SQL migrator without connecting to a database.
pub fn embedded_migrator() -> Migrator {
    Migrator {
        migrations: Cow::Owned(vec![
            mig!(1, "init", "../migrations/001_init.sql"),
            mig!(
                2,
                "artifact storage",
                "../migrations/002_artifact_storage.sql"
            ),
            mig!(3, "user tokens", "../migrations/003_user_tokens.sql"),
            mig!(
                4,
                "artifact size bytes",
                "../migrations/004_artifact_size_bytes.sql"
            ),
            mig!(5, "metadata cache", "../migrations/005_metadata_cache.sql"),
            mig!(6, "local packages", "../migrations/006_local_packages.sql"),
            mig!(
                7,
                "artifact cache meta",
                "../migrations/007_artifact_cache_meta.sql"
            ),
            mig!(8, "quota", "../migrations/008_quota.sql"),
            mig!(
                9,
                "local packages status",
                "../migrations/009_local_packages_status.sql"
            ),
            mig!(10, "rate limit", "../migrations/010_rate_limit.sql"),
            mig!(
                11,
                "package ownership",
                "../migrations/011_package_ownership.sql"
            ),
            mig!(12, "signing", "../migrations/012_signing.sql"),
            mig!(13, "beta channel", "../migrations/013_beta_channel.sql"),
            mig!(14, "ip blocks", "../migrations/014_ip_blocks.sql"),
            mig!(
                15,
                "team namespaces",
                "../migrations/015_team_namespaces.sql"
            ),
            mig!(
                16,
                "package visibility",
                "../migrations/016_package_visibility.sql"
            ),
            mig!(
                17,
                "access events indexes",
                "../migrations/017_access_events_idx.sql"
            ),
            mig!(18, "config changes", "../migrations/018_config_changes.sql"),
            mig!(19, "system kv", "../migrations/019_system_kv.sql"),
            mig!(20, "artifact sboms", "../migrations/020_artifact_sboms.sql"),
            mig!(
                21,
                "access events covering idx",
                "../migrations/021_access_events_covering_idx.sql"
            ),
            mig!(
                22,
                "notification subscriptions",
                "../migrations/022_notification_subscriptions.sql"
            ),
            mig!(
                23,
                "inbound webhook events",
                "../migrations/023_inbound_webhook_events.sql"
            ),
            mig!(
                24,
                "notification subscriptions gin index",
                "../migrations/024_notification_subs_gin_index.sql"
            ),
            mig!(
                25,
                "artifact vulnerabilities",
                "../migrations/025_artifact_vulnerabilities.sql"
            ),
            mig!(
                26,
                "artifact cache checksum",
                "../migrations/026_artifact_cache_checksum.sql"
            ),
            mig!(
                27,
                "deprecation unlisting",
                "../migrations/027_deprecation_unlisting.sql"
            ),
            mig!(28, "user blocks", "../migrations/028_user_blocks.sql"),
        ]),
        ignore_missing: false,
        locking: true,
        no_tx: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The embedded list must have a `mig!` entry for every `.sql` file in
    /// `migrations/`, numbered contiguously from 1. This replaces a hand-bumped
    /// count: adding a migration needs only the new `mig!` entry + `.sql` file —
    /// no test edit — while a forgotten entry (or a numbering gap) fails here.
    #[test]
    fn embedded_migrator_is_contiguous_and_complete() {
        let m = embedded_migrator();

        // Versions are strictly increasing and contiguous starting at 1.
        for (i, mig) in m.migrations.iter().enumerate() {
            assert_eq!(
                mig.version,
                (i + 1) as i64,
                "migration #{i} should have version {} (versions must be contiguous from 1)",
                i + 1
            );
        }

        // Every `NNN_*.sql` on disk is embedded (catches a new file with no `mig!` entry).
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/migrations");
        let sql_count = std::fs::read_dir(dir)
            .expect("read migrations dir")
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("sql"))
            })
            .count();
        assert_eq!(
            m.migrations.len(),
            sql_count,
            "every .sql file in {dir} must have a mig!() entry (and vice versa)"
        );
    }
}
