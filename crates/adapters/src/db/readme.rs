use async_trait::async_trait;
use sqlx::{PgPool, Row};

use crate::db::DbResultExt;
use batlehub_core::{
    entities::{PackageReadme, ReadmeFormat, ReadmeSource},
    error::CoreError,
    ports::ReadmeRepository,
};

pub struct PgReadmeRepository {
    pool: PgPool,
}

impl PgReadmeRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn row_to_readme(r: &sqlx::postgres::PgRow) -> PackageReadme {
    let format: String = r.get("format");
    let source: String = r.get("source");
    PackageReadme {
        registry: r.get("registry"),
        name: r.get("package_name"),
        version: r.get("version"),
        content: r.get("content"),
        // A row whose discriminant this binary does not recognise is a row an
        // older or newer server wrote. Falling back to the escaping renderers —
        // `Plain` shows the source in a `<pre>`, and nothing is interpreted as
        // markup — is the safe direction: the reader sees the text and no
        // markup path runs on a value we could not parse.
        format: ReadmeFormat::parse(&format).unwrap_or(ReadmeFormat::Plain),
        source: ReadmeSource::parse(&source).unwrap_or(ReadmeSource::Archive),
        digest: r.get("digest"),
        truncated: r.get("truncated"),
        package_level: r.get("package_level"),
        extracted_at: r.get("extracted_at"),
    }
}

#[async_trait]
impl ReadmeRepository for PgReadmeRepository {
    async fn upsert(&self, readme: PackageReadme) -> Result<(), CoreError> {
        sqlx::query(
            r#"
            INSERT INTO package_readmes
                (registry, package_name, version, content, format, source,
                 digest, truncated, package_level, extracted_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (registry, package_name, version) DO UPDATE
                SET content      = EXCLUDED.content,
                    format       = EXCLUDED.format,
                    source       = EXCLUDED.source,
                    digest       = EXCLUDED.digest,
                    truncated     = EXCLUDED.truncated,
                    package_level = EXCLUDED.package_level,
                    extracted_at  = EXCLUDED.extracted_at
            "#,
        )
        .bind(&readme.registry)
        .bind(&readme.name)
        .bind(&readme.version)
        .bind(&readme.content)
        .bind(readme.format.as_str())
        .bind(readme.source.as_str())
        .bind(&readme.digest)
        .bind(readme.truncated)
        .bind(readme.package_level)
        .bind(readme.extracted_at)
        .execute(&self.pool)
        .await
        .db_err()?;
        Ok(())
    }

    async fn get(
        &self,
        registry: &str,
        name: &str,
        version: &str,
    ) -> Result<Option<PackageReadme>, CoreError> {
        let row = sqlx::query(
            "SELECT registry, package_name, version, content, format, source, digest, \
             truncated, package_level, extracted_at FROM package_readmes \
             WHERE registry = $1 AND package_name = $2 AND version = $3",
        )
        .bind(registry)
        .bind(name)
        .bind(version)
        .fetch_optional(&self.pool)
        .await
        .db_err()?;

        Ok(row.as_ref().map(row_to_readme))
    }

    async fn get_latest_with_readme(
        &self,
        registry: &str,
        name: &str,
        exclude_versions: &[String],
    ) -> Result<Option<PackageReadme>, CoreError> {
        // Newest *recorded*, not newest version: version strings do not sort as
        // versions in SQL, and inventing an ordering here would put a different
        // answer on this path than the one the version table shows.
        let row = sqlx::query(
            "SELECT registry, package_name, version, content, format, source, digest, \
             truncated, package_level, extracted_at FROM package_readmes \
             WHERE registry = $1 AND package_name = $2 AND NOT (version = ANY($3)) \
             ORDER BY extracted_at DESC LIMIT 1",
        )
        .bind(registry)
        .bind(name)
        .bind(exclude_versions)
        .fetch_optional(&self.pool)
        .await
        .db_err()?;

        Ok(row.as_ref().map(row_to_readme))
    }

    async fn list_versions_with_readme(
        &self,
        registry: &str,
        name: &str,
    ) -> Result<Vec<String>, CoreError> {
        // The version table asks this once per page load; a lookup per row would
        // be N round trips for one answer.
        let rows = sqlx::query(
            "SELECT version FROM package_readmes WHERE registry = $1 AND package_name = $2",
        )
        .bind(registry)
        .bind(name)
        .fetch_all(&self.pool)
        .await
        .db_err()?;

        Ok(rows.iter().map(|r| r.get("version")).collect())
    }

    async fn delete_for_version(
        &self,
        registry: &str,
        name: &str,
        version: &str,
    ) -> Result<(), CoreError> {
        sqlx::query(
            "DELETE FROM package_readmes \
             WHERE registry = $1 AND package_name = $2 AND version = $3",
        )
        .bind(registry)
        .bind(name)
        .bind(version)
        .execute(&self.pool)
        .await
        .db_err()?;
        Ok(())
    }

    async fn delete_for_package(&self, registry: &str, name: &str) -> Result<(), CoreError> {
        sqlx::query("DELETE FROM package_readmes WHERE registry = $1 AND package_name = $2")
            .bind(registry)
            .bind(name)
            .execute(&self.pool)
            .await
            .db_err()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The discriminants this adapter writes are the ones it can read back. A
    /// rename on either side without the other would silently downgrade every
    /// stored README to `Plain`/`Archive` on the next read.
    #[test]
    fn the_stored_discriminants_round_trip() {
        for f in [
            ReadmeFormat::Markdown,
            ReadmeFormat::Html,
            ReadmeFormat::Rst,
            ReadmeFormat::Plain,
        ] {
            assert_eq!(ReadmeFormat::parse(f.as_str()), Some(f));
        }
        for s in [
            ReadmeSource::UpstreamMetadata,
            ReadmeSource::Archive,
            ReadmeSource::LocalPublish,
        ] {
            assert_eq!(ReadmeSource::parse(s.as_str()), Some(s));
        }
    }

    /// An unreadable discriminant degrades to the renderer that interprets
    /// nothing, not to one that parses markup.
    #[test]
    fn an_unknown_format_degrades_to_escaped_source() {
        assert_eq!(
            ReadmeFormat::parse("asciidoc").unwrap_or(ReadmeFormat::Plain),
            ReadmeFormat::Plain
        );
    }
}
