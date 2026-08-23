use async_trait::async_trait;
use sqlx::{PgPool, Row};

use crate::db::DbResultExt;
use batlehub_core::{
    entities::{PackageReadme, ReadmeFormat, ReadmeSource},
    error::CoreError,
    ports::{ReadmeRepository, ReadmeSearchHit},
};

pub struct PgReadmeRepository {
    pool: PgPool,
    /// The Postgres text search configuration the FTS column was built with.
    ///
    /// Held here rather than read per query because it has to match the
    /// generated column's own literal exactly: searching with `french` against a
    /// column built with `english` silently matches almost nothing, which is the
    /// worst kind of wrong answer — a `200` with an empty list.
    ///
    /// Always a name that exists in `pg_ts_config`, because
    /// [`ensure_readme_text_config`] is what puts it here.
    text_config: String,
}

impl PgReadmeRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            text_config: DEFAULT_TEXT_CONFIG.to_owned(),
        }
    }

    /// The same repository searching with `config`.
    ///
    /// Call [`ensure_readme_text_config`] first: this trusts the value, and the
    /// value reaches SQL as a literal.
    pub fn with_text_config(mut self, config: impl Into<String>) -> Self {
        self.text_config = config.into();
        self
    }
}

/// What migration `035` builds the generated column with.
///
/// `english` rather than `simple`, against the recommendation RFC 0007-bis was
/// drafted with: stemming does mangle identifiers, and it does so *symmetrically*
/// — the query is stemmed too, so `axios` still matches — while `simple` fails
/// `retry` against a README that says `retrying` (§13.3).
pub const DEFAULT_TEXT_CONFIG: &str = "english";

/// Make the FTS column match the configured text search configuration.
///
/// Two things have to hold and neither is free:
///
/// - the name must exist on **this** server. Checked against `pg_ts_config` with
///   a bound parameter, which both validates it and — because what comes back is
///   the catalogue's own `cfgname` — makes it safe to interpolate into the DDL
///   below. `to_tsvector` in a generated column must be IMMUTABLE, so there is no
///   way to parameterise it;
/// - the column must actually have been built with it. A `GENERATED … STORED`
///   column cannot be altered in place, so a change means dropping and re-adding
///   it, which **rebuilds every row**. That is why `text_config` is a decision to
///   take at install rather than to tune later, and why this says so in the log
///   rather than doing it quietly.
///
/// Idempotent: the common case is that the column already matches and this runs
/// two catalogue queries and returns.
pub async fn ensure_readme_text_config(pool: &PgPool, config: &str) -> Result<String, CoreError> {
    let Some(config) = known_text_config(pool, config).await? else {
        return Err(CoreError::Config(format!(
            "[search] text_config = '{config}' is not a text search configuration on this \
             Postgres server; `SELECT cfgname FROM pg_ts_config` lists the available ones"
        )));
    };

    if column_text_config(pool).await?.as_deref() == Some(config.as_str()) {
        return Ok(config);
    }

    tracing::warn!(
        text_config = %config,
        "search: rebuilding the README full-text column for a changed [search] text_config — \
         this rewrites every stored README's index and holds a lock while it runs"
    );
    sqlx::query("ALTER TABLE package_readmes DROP COLUMN IF EXISTS content_tsv")
        .execute(pool)
        .await
        .db_err()?;
    // `AssertSqlSafe` because `config` came back from `pg_ts_config` as that
    // catalogue's own `cfgname` — it is an identifier this server already has,
    // not a string a caller supplied — and a generated column's expression
    // cannot be parameterised.
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "ALTER TABLE package_readmes ADD COLUMN content_tsv tsvector \
         GENERATED ALWAYS AS (to_tsvector('{config}', content)) STORED"
    )))
    .execute(pool)
    .await
    .db_err()?;
    // Dropping the column dropped its index with it.
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_package_readmes_fts ON package_readmes USING GIN (content_tsv)")
        .execute(pool)
        .await
        .db_err()?;
    Ok(config)
}

/// `config` as `pg_ts_config` spells it, or `None` if this server has no such
/// text search configuration.
///
/// The name comes back from the catalogue rather than being echoed, which is
/// what makes it safe to interpolate into the DDL `ensure_readme_text_config`
/// builds: what is returned is an identifier this server already has.
pub async fn known_text_config(pool: &PgPool, config: &str) -> Result<Option<String>, CoreError> {
    sqlx::query_scalar("SELECT cfgname::text FROM pg_ts_config WHERE cfgname = $1")
        .bind(config)
        .fetch_optional(pool)
        .await
        .db_err()
}

/// Every text search configuration this server knows.
///
/// Read once at startup so a *reload* can refuse a `[search] text_config` this
/// Postgres has never heard of, instead of accepting it, running with the old
/// one, and only failing on the next restart — which is where an operator finds
/// out about a typo hours after making it, from a server that will not come back
/// up.
pub async fn text_config_names(pool: &PgPool) -> Result<Vec<String>, CoreError> {
    sqlx::query_scalar("SELECT cfgname::text FROM pg_ts_config ORDER BY 1")
        .fetch_all(pool)
        .await
        .db_err()
}

/// The text search configuration the FTS column is **actually** built with.
///
/// Read back from the generated column's own expression rather than remembered:
/// an instance that has been through two different settings is exactly the case
/// where a remembered value would be wrong, and searching with a configuration
/// the column was not built with silently matches almost nothing.
///
/// `None` when the column does not exist (a database that has not run migration
/// `035` yet).
pub async fn column_text_config(pool: &PgPool) -> Result<Option<String>, CoreError> {
    let expr: Option<String> = sqlx::query_scalar(
        "SELECT pg_get_expr(d.adbin, d.adrelid) \
         FROM pg_attrdef d \
         JOIN pg_attribute a ON a.attrelid = d.adrelid AND a.attnum = d.adnum \
         WHERE d.adrelid = 'package_readmes'::regclass AND a.attname = 'content_tsv'",
    )
    .fetch_optional(pool)
    .await
    .db_err()?
    .flatten();
    Ok(expr.as_deref().and_then(text_config_from_expr))
}

/// The configuration name inside a `to_tsvector('english'::regconfig, content)`
/// default expression.
///
/// Substring-matching the expression for `'{name}'` — what this replaced — is a
/// near-miss rather than a check: `content` is in there too, and any name that
/// happens to be a substring of the column list would match a column built with
/// something else.
fn text_config_from_expr(expr: &str) -> Option<String> {
    let rest = expr.split_once('\'')?.1;
    let (name, _) = rest.split_once('\'')?;
    (!name.is_empty()).then(|| name.to_owned())
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

    async fn search(
        &self,
        registries: &[String],
        query: &str,
        limit: u64,
    ) -> Result<Vec<ReadmeSearchHit>, CoreError> {
        if registries.is_empty() || query.trim().is_empty() {
            return Ok(Vec::new());
        }

        // `websearch_to_tsquery`, not `to_tsquery`: it accepts what a person
        // actually types — quoted phrases, `or`, `-excluded` — and does not
        // error on syntax. A search box that 500s on an apostrophe is not a
        // search box.
        //
        // `DISTINCT ON` collapses to one row per package, keeping the
        // best-ranked version: a package whose README repeats across forty patch
        // releases would otherwise fill the page by itself. The outer query
        // re-sorts, because `DISTINCT ON` dictates the inner ordering.
        //
        // `ts_headline` with **empty delimiters**: what comes back is text and
        // nothing downstream has to strip markup out of it. That is the whole of
        // §7.4 — the snippet is a second surface for package-authored content,
        // and it is not going to be a second place where markup is interpreted.
        //
        // The quotes around the empty values are load-bearing. Written bare as
        // `StartSel=,StopSel=`, Postgres reads the *next option's name* as
        // StartSel's value and leaves StopSel at its default `</b>` — so every
        // snippet came back wrapped in `,StopSel=…</b>`. Caught by the assertion
        // that the snippet contains no `</b>`, which is exactly the assertion a
        // looser test would not have made.
        // **`ts_headline` runs above the `LIMIT`, not below it.** It is the one
        // expensive thing here: it re-parses the whole `content` column — up to
        // `readme.max_bytes`, whose ceiling is 4 MiB — and it cannot use the GIN
        // index, which only covers `content_tsv`. With it in the `DISTINCT ON`
        // subquery's target list, Postgres computed a snippet for *every* matching
        // README before the limit could discard all but 201 of them, so one
        // `?q=…&in=readme` on an instance with tens of thousands of stored READMEs
        // detoasted and re-parsed the lot. Ranking and limiting first and
        // snippeting the survivors is the same answer for the cost of the rows
        // actually returned.
        //
        // The `LIMIT` is what keeps the layers apart: a subquery carrying one is
        // never flattened into its parent, so the outer target list cannot be
        // pulled back down into the scan.
        let cfg = &self.text_config;
        let sql = format!(
            "SELECT registry, package_name, version, rank, \
               ts_headline('{cfg}', content, websearch_to_tsquery('{cfg}', $2), \
                 'StartSel=\"\",StopSel=\"\",MaxWords=32,MinWords=12,MaxFragments=1') \
                 AS snippet \
             FROM ( \
               SELECT registry, package_name, version, content, rank FROM ( \
                 SELECT DISTINCT ON (registry, package_name) \
                   registry, package_name, version, content, \
                   ts_rank_cd(content_tsv, websearch_to_tsquery('{cfg}', $2)) AS rank \
                 FROM package_readmes \
                 WHERE registry = ANY($1) \
                   AND content_tsv @@ websearch_to_tsquery('{cfg}', $2) \
                 ORDER BY registry, package_name, rank DESC \
               ) best \
               ORDER BY rank DESC, package_name ASC \
               LIMIT $3 \
             ) top \
             ORDER BY rank DESC, package_name ASC"
        );

        // `AssertSqlSafe` for the same reason as above, and only for that
        // reason: `self.text_config` is a `pg_ts_config` name validated at
        // startup. The **query** is a bound parameter, as it must be — that is
        // the string a caller controls.
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(registries)
            .bind(query)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await
            .db_err()?;

        Ok(rows
            .iter()
            .map(|r| ReadmeSearchHit {
                registry: r.get("registry"),
                name: r.get("package_name"),
                version: r.get("version"),
                snippet: r.get::<Option<String>, _>("snippet").unwrap_or_default(),
                rank: r.get::<f32, _>("rank"),
            })
            .collect())
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

    /// The shape Postgres actually renders a generated column's default as.
    #[test]
    fn the_column_expression_names_its_own_configuration() {
        assert_eq!(
            text_config_from_expr("to_tsvector('english'::regconfig, content)").as_deref(),
            Some("english")
        );
        assert_eq!(
            text_config_from_expr("to_tsvector('french'::regconfig, content)").as_deref(),
            Some("french")
        );
        // No literal to read: not a `to_tsvector` default at all.
        assert_eq!(text_config_from_expr("content"), None);
        assert_eq!(text_config_from_expr("to_tsvector(content)"), None);
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
