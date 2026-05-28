# ── Build stage ───────────────────────────────────────────────────────────────
FROM rust:1.87-slim-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Cache dependency compilation separately from source changes.
COPY Cargo.toml Cargo.lock ./
COPY crates/core/Cargo.toml        crates/core/Cargo.toml
COPY crates/config/Cargo.toml      crates/config/Cargo.toml
COPY crates/adapters/Cargo.toml    crates/adapters/Cargo.toml
COPY crates/web/Cargo.toml         crates/web/Cargo.toml
COPY server/Cargo.toml             server/Cargo.toml

# Stub out every lib/main so cargo can resolve and compile deps.
RUN for crate in crates/core crates/config crates/adapters crates/web; do \
      mkdir -p $crate/src && echo "pub fn _stub() {}" > $crate/src/lib.rs; \
    done && \
    mkdir -p server/src && echo "fn main() {}" > server/src/main.rs

RUN cargo build --release -p batlehub-server 2>/dev/null; exit 0

# Now copy real source and rebuild (only changed crates recompile).
COPY crates/ crates/
COPY server/ server/

# Touch lib/main files so cargo detects the change.
RUN touch crates/*/src/lib.rs server/src/main.rs

RUN cargo build --release -p batlehub-server

# ── Frontend build stage ───────────────────────────────────────────────────────
FROM node:24-slim AS ui-builder

WORKDIR /ui
COPY ui/package.json ui/package-lock.json ./
RUN npm ci

COPY ui/ ./

# Generate the OpenAPI spec from the just-built binary and then the TS client.
COPY --from=builder /build/target/release/batlehub /usr/local/bin/batlehub
COPY config.example.toml /etc/batlehub/config.toml
RUN batlehub --config /etc/batlehub/config.toml dump-spec > openapi.json && \
    npm run generate && \
    npm run build

# ── Runtime image ─────────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder  /build/target/release/batlehub /usr/local/bin/batlehub
COPY --from=ui-builder /ui/dist                        /app/ui/dist

RUN mkdir -p /var/cache/batlehub

EXPOSE 8080

ENTRYPOINT ["batlehub"]
CMD ["--config", "/etc/batlehub/config.toml"]
