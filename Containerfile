# ── Build stage ───────────────────────────────────────────────────────────────
FROM rust:1.96-slim-bookworm@sha256:4732ca96fd086cb9be682050c3f0176288eebaac2b80aa2bcefccfaf198e1950 AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev curl make \
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
FROM node:26-slim@sha256:a1d9d671994fc2d26e297ac56b4b1522a8bc7fa71c43b14cd1b1fe6c5116f7dc AS ui-builder

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
FROM gcr.io/distroless/cc-debian12:latest@sha256:d703b626ba455c4e6c6fbe5f36e6f427c85d51445598d564652a2f334179f96e AS runtime

COPY --from=builder  /build/target/release/batlehub     /usr/local/bin/batlehub
COPY --from=builder  /build/target/release/batlehub-cli /usr/local/bin/batlehub-cli
COPY --from=builder  /var/cache/batlehub                /var/cache/batlehub
COPY --from=ui-builder /ui/dist                         /app/ui/dist

EXPOSE 8080

ENTRYPOINT ["batlehub"]
CMD ["--config", "/etc/batlehub/config.toml"]
