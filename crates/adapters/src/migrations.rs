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
        ]),
        ignore_missing: false,
        locking: true,
        no_tx: false,
    }
}
