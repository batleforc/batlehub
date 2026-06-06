# ── Build stage ───────────────────────────────────────────────────────────────
FROM rust:1.95-slim-bookworm@sha256:d7482085ff5b415f84dba5647ae71606650bdef00db7aeb69f4b3d170c3e4082 AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Cache dependency compilation separately from source changes.
COPY Cargo.toml Cargo.lock ./
COPY crates/core/Cargo.toml        crates/core/Cargo.toml
COPY crates/config/Cargo.toml      crates/config/Cargo.toml
COPY crates/adapters/Cargo.toml    crates/adapters/Cargo.toml
COPY crates/web/Cargo.toml         crates/web/Cargo.toml
COPY crates/examples/Cargo.toml    crates/examples/Cargo.toml
COPY server/Cargo.toml             server/Cargo.toml
COPY cli/Cargo.toml                cli/Cargo.toml
COPY patches/ patches/

# Stub out every lib/main so cargo can resolve and compile deps.
RUN for crate in crates/core crates/config crates/adapters crates/web; do \
    mkdir -p $crate/src && echo "pub fn _stub() {}" > $crate/src/lib.rs; \
    done && \
    mkdir -p crates/examples/src && echo "pub fn _stub() {}" > crates/examples/src/lib.rs && \
    mkdir -p server/src && echo "fn main() {}" > server/src/main.rs && \
    mkdir -p cli/src    && echo "fn main() {}" > cli/src/main.rs

RUN cargo build --release -p batlehub-server -p batlehub-cli 2>/dev/null; exit 0

# Now copy real source and rebuild (only changed crates recompile).
COPY crates/ crates/
COPY server/ server/
COPY cli/    cli/

# Touch lib/main files so cargo detects the change.
RUN touch crates/*/src/lib.rs server/src/main.rs cli/src/main.rs

RUN cargo build --release -p batlehub-server -p batlehub-cli

# Pre-create runtime directories so they can be copied into the shell-less distroless image.
RUN mkdir -p /var/cache/batlehub

# ── Frontend build stage ───────────────────────────────────────────────────────
FROM node:26-slim@sha256:1e738cb88890a15c71880323fbc35a739b7bbc703d72e8bfd1613128f8182f78 AS ui-builder

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
FROM gcr.io/distroless/cc-debian12 AS runtime

COPY --from=builder  /build/target/release/batlehub     /usr/local/bin/batlehub
COPY --from=builder  /build/target/release/batlehub-cli /usr/local/bin/batlehub-cli
COPY --from=builder  /var/cache/batlehub                /var/cache/batlehub
COPY --from=ui-builder /ui/dist                         /app/ui/dist

EXPOSE 8080

ENTRYPOINT ["batlehub"]
CMD ["--config", "/etc/batlehub/config.toml"]
