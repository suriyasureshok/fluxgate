# Fluxgate Edge — Data Plane (`gateway/`)

> High-performance TLS-terminating request gateway built in Rust. Handles authentication, AI inference proxying, real-time metrics export, and DNS-over-HTTPS resolution for the Fluxgate ecosystem.

[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](https://www.rust-lang.org/)
[![Axum](https://img.shields.io/badge/axum-async-blue)](https://github.com/tokio-rs/axum)
[![License](https://img.shields.io/badge/license-MIT-green)](../../LICENSE)

---

## Table of Contents

- [Overview](#overview)
- [Architecture Position](#architecture-position)
- [Core Responsibilities](#core-responsibilities)
- [Technical Stack](#technical-stack)
- [Environment Configuration](#environment-configuration)
- [Dataflow Sequences](#dataflow-sequences)
  - [1. DoH Resolution](#1-doh-resolution-local-override--upstream-fallback)
  - [2. JWT Authentication](#2-jwt-authentication-flow)
  - [3. AI Inference Streaming](#3-ai-inference-streaming-flow)
  - [4. Security Guard Threat Detection](#4-security-guard-threat-detection-flow)
  - [5. Redis Metrics Write](#5-redis-metrics-write-flow)
- [Security Model](#security-model)
- [Admin API (Port 9090)](#admin-api-port-9090)
- [Development](#development)
- [TLS Setup](#tls-setup)
- [Operational Notes](#operational-notes)

---

## Overview

The `gateway/` service is the **Data Plane** of Fluxgate — the first point of contact for every client request. It is intentionally designed as a _fast, stateless relay_ that enforces authentication and streams bytes efficiently, while delegating all intelligent threat analysis to the Python-based Control Plane.

This separation of concerns means the gateway never blocks on expensive ML inference during the hot path. Security decisions arrive asynchronously via the admin port and are applied in-memory with zero request overhead.

<!--
```
Client → [Nginx SPA] → [Axum Gateway :8443] → [AI Model / Ollama]
                               ↕                      ↕
                         Redis (metrics)        Postgres (auth)
                               ↕
                    [Security Guard :9090] → [LLM Analysis]
``` -->

---

## Architecture Position

This service operates exclusively within the **Data Plane** tier. It has no knowledge of threat classification logic — that responsibility belongs to the Control Plane (`guard/`). The gateway exposes a private admin interface on port `9090` that the Control Plane uses to push throttle and pardon commands.

| Tier              | Service                   | Language | Role                           |
| ----------------- | ------------------------- | -------- | ------------------------------ |
| **Data Plane**    | `gateway/` (this service) | Rust     | TLS, auth, proxy, metrics      |
| **Data Plane**    | `doh/`                    | Rust     | DNS-over-HTTPS resolution      |
| **Frontend**      | `frontend/`               | Nginx    | SPA static serving             |
| **Control Plane** | `control-plane/`          | Python   | Threat detection, rate policy  |
| **Control Plane** | `control-plane/`          | Python   | Behavioral analysis via Ollama |
| **Storage**       | Redis                     | —        | Hot-cache, metrics, events     |
| **Storage**       | Supabase Postgres         | —        | User auth, identity            |

---

## Core Responsibilities

### Secure Ingress

Terminates TLS on port `8443`. Enforces strict `SameSite=None; Secure` cookie policy for all session tokens. All plaintext connections are rejected at the socket level — the gateway does not issue redirects.

### Authentication & Authorization

Validates `Authorization: Bearer <token>` headers and session cookies against the PostgreSQL `users` table. JWTs are verified using HS256 with the `JWT_SECRET` env var. Expired or tampered tokens receive an immediate `401` with no upstream call made.

### Reverse Proxying

Authenticated requests are forwarded to downstream AI inference providers (Ollama by default). The `tower` middleware stack maintains streaming response integrity, preserving SSE/chunked-transfer semantics from the model all the way to the client with minimal TTFT overhead.

### Real-time Metrics Export

Token consumption, request counts, and per-route latency are written to Redis on every request -- after the response is dispatched, never before. This ensures Redis write latency has zero effect on client-perceived response time.

### DNS-over-HTTPS Resolution

A built-in DoH resolver handles DNS queries from clients via HTTP POST. Local overrides (e.g. `api.fluxgate.local → 0.0.0.0`) are applied before any upstream query is made, enabling zero-DNS-round-trip resolution for internal services.

### Admin Interface

Port `9090` is an internal-only HTTP API, not exposed through Nginx. The Control Plane uses this endpoint to push throttle commands, pardon identities, and adjust per-user rate limits at runtime without a restart.

---

## Technical Stack

| Component        | Library            | Version        | Purpose                                           |
| ---------------- | ------------------ | -------------- | ------------------------------------------------- |
| Language         | Rust               | 1.96+          | Systems-level performance, memory safety          |
| DNS-over-HTTPS   | trust-dns-proto    | latest         | Async HTTP client for DoH queries                 |
| Web framework    | Axum               | latest         | Async HTTP, routing, middleware                   |
| Async runtime    | Tokio              | latest         | Non-blocking I/O, task scheduling                 |
| Database driver  | SQLx               | latest         | Async PostgreSQL with compile-time query checking |
| Password hashing | Argon2             | latest         | Argon2id for credential storage                   |
| JWT              | jsonwebtoken       | latest         | HS256 session token signing/verification          |
| Cache            | async-redis        | latest         | Async Redis driver for metrics and session state  |
| TLS              | rustls / rcgen     | latest         | Pure-Rust TLS stack, no OpenSSL dependency        |
| Middleware       | tower / tower-http | latest / 0.5.2 | Request tracing, compression, timeouts            |

---

## Environment Configuration

All variables are required at boot unless a default is listed. Missing required variables cause the process to exit with a descriptive error before binding any port.

| Variable        | Description                                                 | Default                    |
| --------------- | ----------------------------------------------------------- | -------------------------- |
| `DATABASE_URL`  | Postgres connection string (`postgres://user:pass@host/db`) | **required**               |
| `REDIS_URL`     | Redis server address                                        | `redis://redis-cache:6379` |
| `TLS_CERT_PATH` | Path to public SSL certificate (PEM)                        | `/app/certs/cert.pem`      |
| `TLS_KEY_PATH`  | Path to private SSL key (PEM)                               | `/app/certs/key.pem`       |
| `PUBLIC_PORT`   | Data Plane listener port                                    | `8443`                     |
| `ADMIN_PORT`    | Control Plane admin API port                                | `9090`                     |
| `JWT_SECRET`    | HMAC secret for JWT signing and verification                | **required**               |

> **Security note:** `JWT_SECRET` must be at least 256 bits of cryptographically random data. Generate with: `openssl rand -hex 32`

### Example `.env`

```env
DATABASE_URL=postgres://fluxgate:secret@localhost:5432/fluxgate
REDIS_URL=redis://localhost:6379
TLS_CERT_PATH=./certs/cert.pem
TLS_KEY_PATH=./certs/key.pem
PUBLIC_PORT=8443
ADMIN_PORT=9090
JWT_SECRET=your-256-bit-random-secret-here
```

---

## Dataflow Sequences

The following diagrams document every major interaction path through the gateway. These are the source-of-truth references for integration and debugging.

---

### 1. DoH Resolution — Local Override & Upstream Fallback

The gateway intercepts DNS-over-HTTPS queries from clients. Local hostname overrides are applied before any upstream DNS call is made, ensuring internal hostnames (e.g. `api.fluxgate.local`) resolve instantly without external round-trips.

```mermaid
%%{init: {"theme": "base", "themeVariables": {"primaryColor": "#E6F1FB", "primaryBorderColor": "#185FA5", "primaryTextColor": "#0C447C", "actorBkg": "#E6F1FB", "actorBorder": "#185FA5", "actorTextColor": "#0C447C", "activationBkgColor": "#E1F5EE", "activationBorderColor": "#0F6E56", "sequenceNumberColor": "#0C447C", "fontFamily": "Google Sans, sans-serif", "fontSize": "14px"}} }%%
sequenceDiagram
    autonumber
    participant C as Client
    participant G as Your Gateway
    participant U as Upstream DoH Server

    C->>G: HTTP POST /dns-query<br/>(DNS query over HTTPS)

    Note over G: Decrypt HTTPS body<br/>Extract raw DNS question

    G-->>G: Intercept & check local overrides<br/>api.fluxgate.local → 127.0.0.1?

    alt Local override matches
        Note over G: Fabricate DNS answer<br/>api.fluxgate.local → 127.0.0.1
        G-->>G: Serialize & encrypt response<br/>(wrap in HTTPS response envelope)
        G->>C: HTTP 200 — DNS answer<br/>(A record: 127.0.0.1)
    else No local override
        G->>U: Forward DNS query upstream
        U->>G: Real DNS answer
        G-->>G: Serialize & encrypt response
        G->>C: HTTP 200 — DNS answer<br/>(upstream A record)
    end
```

---

### 2. JWT Authentication Flow

Every request passes through the authentication middleware before any proxying occurs. Invalid or expired tokens are rejected immediately, with no upstream call made.

```mermaid
%%{init: {"theme": "base", "themeVariables": {"primaryColor": "#E6F1FB", "primaryBorderColor": "#185FA5", "primaryTextColor": "#0C447C", "actorBkg": "#E6F1FB", "actorBorder": "#185FA5", "actorTextColor": "#0C447C", "activationBkgColor": "#E1F5EE", "activationBorderColor": "#0F6E56", "fontFamily": "Google Sans, sans-serif", "fontSize": "14px"}} }%%
sequenceDiagram
    autonumber
    participant U as User
    participant F as Nginx SPA
    participant G as Axum Gateway
    participant P as Supabase Postgres

    U->>F: HTTPS request + JWT in header

    F->>G: Proxy request with JWT forwarded

    G->>G: Extract & decode JWT<br/>(verify signature, check expiry)

    alt JWT invalid or expired
        G->>F: 401 Unauthorized
        F->>U: Redirect to login
    else JWT valid
        G->>P: SELECT user WHERE id = jwt.sub
        P->>G: User record + roles
        G->>G: Attach user context to request
        G->>F: Proxy response (200)
        F->>U: Serve response
    end
```

---

### 3. AI Inference Streaming Flow

Authenticated requests destined for the AI model are rate-checked against Redis before forwarding. Response tokens are streamed chunk-by-chunk back to the client, preserving low TTFT regardless of total generation length.

```mermaid
%%{init: {"theme": "base", "themeVariables": {"primaryColor": "#E6F1FB", "primaryBorderColor": "#185FA5", "primaryTextColor": "#0C447C", "actorBkg": "#E6F1FB", "actorBorder": "#185FA5", "actorTextColor": "#0C447C", "activationBkgColor": "#FAEEDA", "activationBorderColor": "#854F0B", "fontFamily": "Google Sans, sans-serif", "fontSize": "14px"}} }%%
sequenceDiagram
    autonumber
    participant U as User
    participant G as Axum Gateway
    participant R as Redis
    participant AI as AI Model (Ollama)

    U->>G: POST /chat (prompt payload)

    G->>R: GET session context / rate-limit key
    R->>G: Session state + request count

    alt Rate limit exceeded
        G->>U: 429 Too Many Requests
    else Within limit
        G->>AI: Stream invoke (prompt + context)
        Note over AI: LLM inference begins<br/>(token-by-token generation)

        loop Stream chunks
            AI-->>G: Token chunk (SSE / HTTP stream)
            G-->>U: Forward chunk in real time
        end

        G->>R: Increment request counter<br/>Store updated session state
        G->>U: Stream closed (done)
    end
```

---

### 4. Security Guard Threat Detection Flow

The Control Plane's Security Guard polls the gateway's metrics feed from Redis. On detecting anomalous patterns, it escalates to the LLM for behavioral analysis and pushes throttle/pardon commands back to the gateway's admin port.

```mermaid
%%{init: {"theme": "base", "themeVariables": {"primaryColor": "#FAEEDA", "primaryBorderColor": "#854F0B", "primaryTextColor": "#633806", "actorBkg": "#FAEEDA", "actorBorder": "#854F0B", "actorTextColor": "#633806", "activationBkgColor": "#FCEBEB", "activationBorderColor": "#A32D2D", "fontFamily": "Google Sans, sans-serif", "fontSize": "14px"}} }%%
sequenceDiagram
    autonumber
    participant GW as Axum Gateway
    participant GD as Security Guard
    participant R as Redis
    participant LM as Ollama / LLM

    loop Every polling interval
        GD-->>GW: Poll — fetch current metrics
        GW-->>GD: Request rates, error counts, patterns
    end

    GD->>GD: Evaluate metrics against thresholds

    alt Anomaly detected
        GD->>LM: Analyze — send suspicious payload/pattern
        LM->>GD: Threat verdict + confidence score

        alt LLM flags as threat
            GD->>GW: Throttle — apply rate limit / block IP
            GD->>R: Store incident record<br/>(timestamp, source, verdict)
        else LLM clears as benign
            GD->>R: Log false-positive for tuning
        end
    else No anomaly
        Note over GD: Continue polling, no action
    end

    GD-->>GW: Pardon — lift throttle after cooldown
```

---

### 5. Redis Metrics Write Flow

Metrics are written to Redis in a non-blocking fire-and-forget pattern after the response is dispatched. This guarantees that Redis latency or unavailability never affects client-perceived response time.

```mermaid
%%{init: {"theme": "base", "themeVariables": {"primaryColor": "#E1F5EE", "primaryBorderColor": "#0F6E56", "primaryTextColor": "#085041", "actorBkg": "#E1F5EE", "actorBorder": "#0F6E56", "actorTextColor": "#085041", "activationBkgColor": "#E6F1FB", "activationBorderColor": "#185FA5", "fontFamily": "Google Sans, sans-serif", "fontSize": "14px"}} }%%
sequenceDiagram
    autonumber
    participant U as User
    participant G as Axum Gateway
    participant R as Redis

    U->>G: Any authenticated request

    G->>R: INCR requests:total<br/>INCR requests:ip:{user_ip}
    G->>R: SET session:{user_id} {state} EX 3600
    R->>G: ACK

    Note over G: Request processed normally...

    G->>R: HSET metrics:latency {route} {ms}
    G->>R: LPUSH events:stream {event_json}<br/>(ring buffer for Guard to poll)
    R->>G: ACK

    G->>U: Response
```

---

## Security Model

### Defense layers

| Layer            | Mechanism                             | Where                  |
| ---------------- | ------------------------------------- | ---------------------- |
| TLS              | rustls — no OpenSSL dependency        | Port `8443`            |
| Authentication   | JWT HS256 + Postgres user record      | Axum middleware        |
| Password storage | Argon2id                              | Postgres (users table) |
| Cookie policy    | `SameSite=None; Secure; HttpOnly`     | Session issuance       |
| Rate limiting    | Per-identity Redis counter            | Hot path               |
| Admin isolation  | Port `9090` not exposed through Nginx | Docker network policy  |

### Threat model assumptions

- The admin port (`9090`) is **never reachable from the public internet**. It must be isolated to the Docker internal network only. Misconfiguring this is a critical security gap.
- `JWT_SECRET` rotation requires a rolling restart. All existing sessions will be invalidated on secret change.
- Redis is treated as a trusted internal service. If Redis is compromised, rate limit state and the security event stream are compromised. Redis should not be exposed outside the container network.

---

## Admin API (Port 9090)

The Control Plane communicates with the gateway via this private HTTP interface. All endpoints are unauthenticated by design — network isolation (Docker internal network) is the only access control.

| Method | Path              | Body                                                       | Effect                                 |
| ------ | ----------------- | ---------------------------------------------------------- | -------------------------------------- |
| `POST` | `/admin/throttle` | `{ "identity": "ip\|user_id", "limit": n, "window_s": n }` | Apply rate limit override              |
| `POST` | `/admin/pardon`   | `{ "identity": "ip\|user_id" }`                            | Lift existing throttle                 |
| `GET`  | `/admin/metrics`  | —                                                          | Return current metrics snapshot (JSON) |
| `GET`  | `/admin/health`   | —                                                          | Liveness check (`200 OK`)              |

> The Security Guard (`guard/`) is the only intended caller of this API. Do not expose port `9090` in `docker-compose.yml` port mappings.

---

## Development

### Prerequisites

- Rust 1.96+
- A running local Postgres instance with the Fluxgate schema applied
- A running local Redis instance
- Valid TLS certificates in `./certs/` (see [TLS Setup](#tls-setup) below)

### Build and run

```bash
# Build in release mode
cargo build --release

# Run with environment loaded from .env
cargo run

# Run with explicit env vars
DATABASE_URL=postgres://... JWT_SECRET=... cargo run
```

### Running tests

```bash
# Unit + integration tests (requires live Postgres and Redis)
cargo test

# Unit tests only (no external dependencies)
cargo test --lib
```

### Clippy and formatting

```bash
cargo clippy -- -D warnings
cargo fmt --check
```

---

## TLS Setup

The gateway requires a valid TLS certificate before it will bind any port. For local development, generate a self-signed certificate trusted by your local machine using `mkcert`:

```bash
# Install mkcert (macOS)
brew install mkcert
mkcert -install

# Generate a cert for local development hostnames
mkcert api.fluxgate.local localhost 127.0.0.1 ::1

# Move certs to the expected location
mkdir -p certs
mv api.fluxgate.local+2.pem certs/cert.pem
mv api.fluxgate.local+2-key.pem certs/key.pem
```

For production, replace these with certificates issued by your CA (e.g. Let's Encrypt via Certbot or a managed cert from your cloud provider). Map the paths via `TLS_CERT_PATH` and `TLS_KEY_PATH`.

> The gateway will **fail to start** if either cert file is missing or unreadable. Check file permissions (`chmod 600 certs/key.pem`) if the process exits immediately.

---

## Operational Notes

### Startup order dependency

The gateway depends on both Postgres and Redis being healthy before it can serve traffic. In `docker-compose.yml`, use `depends_on` with `condition: service_healthy` and define healthchecks on both storage containers. A failed Postgres connection at startup causes an immediate exit.

### Redis unavailability behavior

If Redis becomes unreachable mid-operation:

- Rate limiting falls back to **permissive** (requests are not blocked)
- Metrics writes are silently dropped (no client impact)
- Session state reads fail gracefully — the gateway falls back to JWT-only auth

### Graceful shutdown

The gateway handles `SIGTERM` with a configurable drain period (default: 30 seconds). In-flight streaming responses are allowed to complete before the process exits. Kubernetes `preStop` hooks should use `sleep 5` before sending `SIGTERM` to account for load balancer connection draining.

### Logging

Logs are structured JSON via `tracing`. Set `RUST_LOG=info` for standard output, `RUST_LOG=debug` for request-level detail, and `RUST_LOG=trace` for middleware-level diagnostics (high volume — not recommended in production).

```bash
RUST_LOG=info cargo run
```

---

## Related Services

| Service        | Path                 | Description                                            |
| -------------- | -------------------- | ------------------------------------------------------ |
| Security Guard | `control-plane/`     | Python Control Plane — polls metrics, escalates to LLM |
| Nginx SPA      | `frontend/`          | Static frontend — proxies to this gateway              |
| DoH Resolver   | `doh/`               | DNS-over-HTTPS handler embedded in the Data Plane      |
| Compose config | `docker-compose.yml` | Full stack orchestration                               |
| Certs          | `certs/`             | TLS certificates (gitignored)                          |
