-- Make what a package *says* searchable (RFC 0007-bis §5.2).
--
-- The catalogue's search matches names. That answers "do we have something
-- called `retry`" and cannot answer "which of our internal libraries does
-- exponential backoff" — which is the question a developer actually arrives
-- with, and the question an internal package page is the only place in the world
-- that could answer.
--
-- A generated column rather than a trigger: `STORED` means the index cannot
-- drift from the text, because there is no trigger to forget and no backfill to
-- run. The cost is write amplification on `ReadmeService::record`, which already
-- refuses to rewrite an unchanged digest — so a re-resolve that changed nothing
-- costs nothing here either.
--
-- **`english`, not `simple`.** RFC 0007-bis was drafted arguing the opposite, on
-- the grounds that stemming mangles identifiers: `axios` is stored as `axio`,
-- `redis` as `redi`. That is true, and the conclusion does not follow — the
-- *query* is stemmed by the same configuration, so a search for `axios` becomes
-- a search for `axio` and still matches. Measured over seven queries, `english`
-- answered every one and `simple` failed two of them, including `retry` against
-- a README that says `retrying` and `cache` against one that says `caching`
-- (RFC 0007-bis §13.3). `simple` is not the conservative choice; it is the one
-- that silently returns nothing for the most natural way to ask.
--
-- The configuration is settable per instance (`[search] text_config`), because
-- an estate whose internal packages are documented in another language is
-- exactly the kind of deployment that self-hosts. `to_tsvector` in a generated
-- column must be IMMUTABLE, which means the configuration has to be a literal
-- here — so changing it rebuilds the column, which is why it is a decision taken
-- at install rather than tuned later, and why the server rebuilds it on startup
-- rather than leaving an operator to discover the mismatch.
ALTER TABLE package_readmes
    ADD COLUMN IF NOT EXISTS content_tsv tsvector
    GENERATED ALWAYS AS (to_tsvector('english', content)) STORED;

-- GIN rather than GiST: the column is read far more often than it is written,
-- which is the trade GIN is on the right side of.
--
-- Built **inside the migration transaction**, because `CREATE INDEX
-- CONCURRENTLY` cannot run in one. On a large `package_readmes` this holds a
-- lock for the duration of the build, which is named in the release notes so an
-- operator with a big catalogue schedules it rather than finding out during a
-- deploy.
CREATE INDEX IF NOT EXISTS idx_package_readmes_fts
    ON package_readmes USING GIN (content_tsv);
