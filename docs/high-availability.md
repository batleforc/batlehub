# High Availability

BatleHub is a **stateless HTTP server** — all durable state lives in PostgreSQL and an object store, not in the process. Running multiple replicas safely requires swapping two single-instance defaults for shared backends: the in-memory cache store and the local filesystem storage.

---

## Table of Contents

1. [Architecture overview](#1-architecture-overview)
2. [Prerequisites](#2-prerequisites)
3. [Configuration changes](#3-configuration-changes)
   - [Cache backend](#31-cache-backend)
   - [Artifact storage](#32-artifact-storage)
   - [Database connection pool](#33-database-connection-pool)
   - [CORS](#34-cors)
   - [Complete example](#35-complete-multi-instance-config-example)
4. [Docker Compose — single-host redundancy](#4-docker-compose--single-host-redundancy)
5. [Kubernetes / Helm — production HA](#5-kubernetes--helm--production-ha)
   - [HA values file](#51-ha-values-file)
   - [Horizontal Pod Autoscaler](#52-horizontal-pod-autoscaler)
6. [Rolling updates and zero-downtime deploys](#6-rolling-updates-and-zero-downtime-deploys)
7. [Health probes](#7-health-probes)
8. [Observability](#8-observability)
9. [Known limitations](#9-known-limitations)

---

## 1. Architecture overview

```
                 ┌───────────────┐
                 │  Load balancer │
                 └──────┬────────┘
          ┌─────────────┼─────────────┐
          ▼             ▼             ▼
   ┌────────────┐ ┌────────────┐ ┌────────────┐
   │ BatleHub 1 │ │ BatleHub 2 │ │ BatleHub 3 │
   └─────┬──────┘ └─────┬──────┘ └─────┬──────┘
         │               │               │
         └───────────────┼───────────────┘
                         │
           ┌─────────────┼─────────────┐
           ▼             ▼             ▼
     ┌──────────┐  ┌──────────┐  ┌──────────┐
     │PostgreSQL│  │  Redis   │  │    S3    │
     │(primary) │  │ (cache)  │  │(storage) │
     └──────────┘  └──────────┘  └──────────┘
```

All state is shared externally. No sticky sessions are needed — any replica can serve any request.

### What changes between single-instance and HA

| Component | Single-instance default | Multi-instance requirement |
|-----------|------------------------|---------------------------|
| Metadata cache | `InMemoryCacheStore` (per-process) | PostgreSQL or Redis (`[cache]`) |
| Rate limiting | `InMemoryRateLimitStore` (per-process) | Same `[cache]` backend — automatic |
| IP blocking | `InMemoryIpBlockStore` (per-process) | Same `[cache]` backend — automatic |
| Artifact storage | Filesystem (`/var/cache/batlehub`) | S3-compatible object store |
| Canonical data | PostgreSQL | PostgreSQL — already shared |

The `[cache]` section controls all three in-memory stores with a single setting. Switching it also fixes rate limiting and IP blocking without any additional config.

---

## 2. Prerequisites

Before scaling beyond one replica:

- **PostgreSQL 14+** — already required; no change needed.
- **S3-compatible object store** — AWS S3, MinIO, or RustFS. Filesystem storage is single-node only.
- **Shared cache backend** — either the same PostgreSQL instance (simplest) or a Redis 7+ instance.
- **Load balancer / ingress** — anything that does round-robin HTTP (nginx, Traefik, Kubernetes Ingress). No session affinity required.

---

## 3. Configuration changes

These are the only config changes needed to go from single-instance to multi-instance. Everything else stays the same.

### 3.1 Cache backend

Replace the default in-memory cache with a shared backend. This single change covers metadata cache, rate limiting, and IP blocking.

**Option A — PostgreSQL** (uses your existing database, no extra service):

```toml
[cache]
type = "postgres"
# url defaults to database.url — omit it unless you want a separate connection string
```

**Option B — Redis** (lower latency, recommended for high request volume):

```toml
[cache]
type = "redis"
url  = "redis://redis:6379"
```

### 3.2 Artifact storage

Switch from filesystem to S3. All replicas read and write to the same bucket.

```toml
[storage]
type   = "s3"
bucket = "batlehub-artifacts"
region = "us-east-1"

# For self-hosted S3 (MinIO, RustFS):
# endpoint         = "http://minio:9000"
# force_path_style = true

# Credentials (omit on AWS with an IAM role):
# access_key_id     = "AKIAIOSFODNN7EXAMPLE"
# secret_access_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
```

### 3.3 Database connection pool

Each replica opens its own connection pool. Lower `max_connections` per replica when running behind PgBouncer, or leave it at the default (10) when connecting directly.

```toml
[database]
type            = "postgresql"
url             = "postgresql://batlehub:changeme@postgres:5432/batlehub"
max_connections = 5   # recommended per replica when using a connection pooler
```

### 3.4 CORS

Set `cors_allowed_origins` to the load-balancer hostname so browser clients are not blocked by CORS:

```toml
[server]
host                 = "0.0.0.0"
port                 = 8080
cors_allowed_origins = ["https://batlehub.example.com"]
```

### 3.5 Complete multi-instance config example

```toml
[server]
host                 = "0.0.0.0"
port                 = 8080
static_dir           = "/app/ui/dist"
cors_allowed_origins = ["https://batlehub.example.com"]

[database]
type            = "postgresql"
url             = "postgresql://batlehub:changeme@postgres:5432/batlehub"
max_connections = 5

[cache]
type = "redis"
url  = "redis://redis:6379"

[storage]
type   = "s3"
bucket = "batlehub-artifacts"
region = "us-east-1"

[[auth]]
type = "token"

[[auth.tokens]]
value   = "change-me-admin-token"
role    = "admin"
user_id = "admin"

[[registries]]
type = "npm"
name = "npm"

[registries.rbac]
anonymous = ["releases:read", "source:read"]
user      = ["releases:read", "source:read"]
admin     = ["*"]
```

---

## 4. Docker Compose — single-host redundancy

Docker Compose can run multiple server replicas on a single host. This protects against process crashes but not host failure. Use it for staging environments or when you want process-level redundancy without a full Kubernetes cluster.

```yaml
# docker-compose.ha.yml
services:
  postgres:
    image: postgres:17-alpine
    environment:
      POSTGRES_DB:       batlehub
      POSTGRES_USER:     batlehub
      POSTGRES_PASSWORD: changeme
    volumes:
      - postgres_data:/var/lib/postgresql/data

  redis:
    image: redis:7-alpine
    command: redis-server --save "" --appendonly no

  batlehub:
    image: ghcr.io/batleforc/batlehub:1.0.0
    deploy:
      replicas: 2
      restart_policy:
        condition: on-failure
    depends_on: [postgres, redis]
    volumes:
      - ./config.toml:/etc/batlehub/config.toml:ro
    # No cache volume needed — storage is S3.

  proxy:
    image: nginx:alpine
    ports:
      - "8080:80"
    volumes:
      - ./nginx.conf:/etc/nginx/nginx.conf:ro
    depends_on: [batlehub]

volumes:
  postgres_data:
```

Minimal `nginx.conf`:

```nginx
events {}
http {
  upstream batlehub {
    server batlehub:8080;   # Docker's internal DNS round-robins across replicas
  }
  server {
    listen 80;
    location / {
      proxy_pass http://batlehub;
      proxy_set_header Host              $host;
      proxy_set_header X-Real-IP         $remote_addr;
      proxy_set_header X-Forwarded-For   $proxy_add_x_forwarded_for;
      proxy_set_header X-Forwarded-Proto $scheme;
    }
  }
}
```

> **IP blocking behind a proxy** — If `[ip_blocking]` is enabled, BatleHub reads the client IP from `X-Real-IP` or `X-Forwarded-For`. Ensure your load balancer sets these headers; otherwise all requests appear to come from the proxy's IP.

---

## 5. Kubernetes / Helm — production HA

The bundled Helm chart supports multi-replica deployments out of the box once S3 and a shared cache backend are configured.

### 5.1 HA values file

```yaml
# ha-values.yaml
replicaCount: 3

image:
  repository: ghcr.io/batleforc/batlehub
  tag: "1.0.0"    # pin to a specific version

database:
  url: "postgresql://batlehub:changeme@postgres-svc:5432/batlehub"

storage:
  type: s3
  s3:
    bucket:          batlehub-artifacts
    region:          us-east-1
    accessKeyId:     "AKIAIOSFODNN7EXAMPLE"
    secretAccessKey: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"

# PVC not needed — all artifacts go to S3.
persistence:
  enabled: false

# Inject the shared cache backend via extraConfig.
extraConfig: |
  [cache]
  type = "redis"
  url  = "redis://redis-svc:6379"

ingress:
  enabled: true
  className: nginx
  host: batlehub.example.com
  annotations:
    cert-manager.io/cluster-issuer: letsencrypt-prod
  tls:
    - secretName: batlehub-tls
      hosts:
        - batlehub.example.com

# Spread replicas across nodes.
affinity:
  podAntiAffinity:
    preferredDuringSchedulingIgnoredDuringExecution:
      - weight: 100
        podAffinityTerm:
          topologyKey: kubernetes.io/hostname
          labelSelector:
            matchLabels:
              app.kubernetes.io/name: batlehub

resources:
  requests:
    cpu:    200m
    memory: 256Mi
  limits:
    cpu:    1000m
    memory: 512Mi
```

```sh
helm install batlehub ./helm/batlehub \
  --namespace batlehub \
  --create-namespace \
  -f ha-values.yaml
```

### 5.2 Horizontal Pod Autoscaler

Scale replicas automatically based on CPU load:

```yaml
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: batlehub
  namespace: batlehub
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: batlehub
  minReplicas: 2
  maxReplicas: 10
  metrics:
    - type: Resource
      resource:
        name: cpu
        target:
          type: Utilization
          averageUtilization: 70
```

---

## 6. Rolling updates and zero-downtime deploys

BatleHub applies database migrations automatically on startup. Migrations are designed to be additive — they never drop columns or tables that the previous version still reads. This means a rolling deploy is safe:

1. New pods start, run migrations, and become ready.
2. Old pods continue serving requests while new ones apply migrations.
3. Old pods are terminated once new ones pass their readiness probe.

Configure the Deployment strategy to guarantee zero downtime:

```sh
kubectl patch deployment batlehub \
  -n batlehub \
  --type=json \
  -p='[{"op":"add","path":"/spec/strategy","value":{"type":"RollingUpdate","rollingUpdate":{"maxUnavailable":0,"maxSurge":1}}}]'
```

---

## 7. Health probes

The Helm chart configures liveness and readiness probes automatically:

| Probe | Endpoint | Initial delay | Period |
|-------|----------|--------------|--------|
| Readiness | `GET /api/v1/admin/health` | 5 s | 10 s |
| Liveness | `GET /api/v1/admin/health` | 10 s | 30 s |

The health endpoint does **not** require an `Authorization` header. Kubernetes can reach it directly from the kubelet.

Traffic is only routed to a pod once its readiness probe passes — so clients are never sent to a replica that is still applying migrations or warming its cache.

---

## 8. Observability

Distributed tracing works across replicas without any extra configuration. Each span carries the same trace ID regardless of which replica handles a request. Point all replicas at the same OpenTelemetry collector:

```toml
[otel]
endpoint     = "http://otel-collector:4317"
service_name = "batlehub"
```

In Helm:

```yaml
otel:
  enabled:  true
  endpoint: "http://otel-collector:4317"
```

Each replica emits its own spans; the collector stitches them into complete traces by trace ID. See `docs/configuration.md` for the full `[otel]` reference.

---

## 9. Known limitations

These are accepted trade-offs documented in [§9 of contributing.md](contributing.md#9-known-limitations-and-accepted-trade-offs):

- **Quota TOCTOU race** — publish quota enforcement reads the current usage and then increments it in two separate DB operations. Under concurrent publishes across replicas the quota can be exceeded by at most one upload per concurrent writer. Enforcement is eventually consistent, not strict.

- **Cache warm-up duplicates** — each replica runs its own warm-up pass on startup. Multiple replicas starting simultaneously will each independently fetch the same upstream packages. The downloads are idempotent (last writer wins in S3) but generate duplicate upstream traffic.

- **Async quota rollback** — if a publish fails after storage but before the DB commit, the quota counter is decremented asynchronously. A short window exists where the counter is overcounted.

- **In-memory stores if misconfigured** — if `[cache]` is left at the default `type = "memory"`, each replica maintains its own independent rate-limit and IP-block state. Rate limits will be N times more permissive than configured (where N is the replica count), and IP blocks set on one replica will not propagate to others. Always set a shared cache backend for multi-replica deployments.
