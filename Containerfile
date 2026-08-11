# ── Build stage ───────────────────────────────────────────────────────────────
FROM rust:1.97-slim-bookworm@sha256:2775a09d208ff0d7c1f50490c45b62db929e87ba1dcbc3f2132ac71a704bcdd3 AS builder

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
FROM node:26-slim@sha256:4ebb5ace66f15a24c14c492e01a8beeed4fddf970a856109f5126e703e5fe503 AS ui-builder

WORKDIR /ui
# Corepack is no longer distributed with Node (removed in Node 25), so pnpm is
# installed explicitly. Keep this version in sync with the `packageManager`
# field in ui/package.json.
RUN npm install -g pnpm@11.20.0
COPY ui/package.json ui/pnpm-lock.yaml ui/pnpm-workspace.yaml ./
# --frozen-lockfile is the `npm ci` equivalent: it fails rather than silently
# resolving something the committed lockfile does not describe.
RUN pnpm install --frozen-lockfile

COPY ui/ ./

# Generate the OpenAPI spec from the just-built binary and then the TS client.
COPY --from=builder /build/target/release/batlehub /usr/local/bin/batlehub
COPY config.example.toml /etc/batlehub/config.toml
RUN batlehub --config /etc/batlehub/config.toml dump-spec > openapi.json && \
    pnpm run generate && \
    pnpm run build

# ── Runtime image ─────────────────────────────────────────────────────────────
FROM gcr.io/distroless/cc-debian12:latest@sha256:6e1871c34683dc9ee996d13084497783fd98ac0200213d0826625f4e9d4be1d0 AS runtime

COPY --from=builder  /build/target/release/batlehub     /usr/local/bin/batlehub
COPY --from=builder  /build/target/release/batlehub-cli /usr/local/bin/batlehub-cli
# The cache directory is the one path the process writes to when
# `[storage] type = "filesystem"`, so it must be owned by the runtime user.
# Everything else (the binaries, the SPA bundle) stays root-owned and
# read-only to that user, which is what we want.
COPY --from=builder --chown=65532:65532 /var/cache/batlehub /var/cache/batlehub
COPY --from=ui-builder /ui/dist                         /app/ui/dist

EXPOSE 8080

# 65532 is distroless's `nonroot` user. Declared numerically because the image
# ships no shell and no /etc/passwd lookup is guaranteed at runtime; Kubernetes
# also needs a numeric UID to satisfy `runAsNonRoot` without resolving names.
# The chart pins the same UID in its podSecurityContext — keep the two in sync.
USER 65532:65532

ENTRYPOINT ["batlehub"]
CMD ["--config", "/etc/batlehub/config.toml"]
