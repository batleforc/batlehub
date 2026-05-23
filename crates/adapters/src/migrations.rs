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
            mig!(1, "init",                "../migrations/001_init.sql"),
            mig!(2, "artifact storage",   "../migrations/002_artifact_storage.sql"),
            mig!(3, "user tokens",        "../migrations/003_user_tokens.sql"),
            mig!(4, "artifact size bytes","../migrations/004_artifact_size_bytes.sql"),
            mig!(5, "metadata cache",     "../migrations/005_metadata_cache.sql"),
            mig!(6, "local packages",     "../migrations/006_local_packages.sql"),
            mig!(7, "artifact cache meta","../migrations/007_artifact_cache_meta.sql"),
        ]),
        ignore_missing: false,
        locking: true,
        no_tx: false,
    }
}
