# Fluxgate Developer Portal (`frontend/`)

> Lightweight developer portal for the Fluxgate ecosystem. Provides account management, API key provisioning, and a real-time AI playground for validating streaming behavior through the Data Plane.

[![HTML5](https://img.shields.io/badge/html5-vanilla-orange?logo=html5)](https://developer.mozilla.org/en-US/docs/Web/HTML)
[![JavaScript](https://img.shields.io/badge/javascript-es6-yellow?logo=javascript)](https://developer.mozilla.org/en-US/docs/Web/JavaScript)
[![Tailwind CSS](https://img.shields.io/badge/tailwindcss-cdn-blue?logo=tailwindcss)](https://tailwindcss.com/)
[![License](https://img.shields.io/badge/license-MIT-green)](../../LICENSE)

---

## Table of Contents

- Overview
- Architecture Position
- Core Responsibilities
- Technical Stack
- Authentication Model
- Streaming Playground
- Session Management
- Development
- Local HTTPS Setup
- Nginx Configuration
- Deployment
- Operational Notes
- Related Services

---

# Overview

The `frontend/` service is the public-facing developer portal for Fluxgate. It provides a lightweight Single Page Application (SPA) that allows users to authenticate, manage API keys, and test real-time AI inference streaming through the Gateway.

The frontend intentionally contains no framework dependencies. All UI interactions are implemented using vanilla JavaScript and native browser APIs, minimizing bundle size and startup overhead.

```
Browser
    ↓
Frontend SPA (Nginx)
    ↓
Gateway (:8443)
    ↓
AI Providers
```

---

# Architecture Position

This service belongs to the **Frontend Tier** and contains no business logic.

| Tier          | Service          | Language  | Responsibility                  |
| ------------- | ---------------- | --------- | ------------------------------- |
| Frontend      | `frontend/`      | HTML / JS | User interface                  |
| Data Plane    | `gateway/`       | Rust      | Authentication, streaming proxy |
| Control Plane | `control-plane/` | Python    | Security analysis               |
| Storage       | Postgres         | —         | User accounts                   |
| Storage       | Redis            | —         | Metrics and state               |

---

# Core Responsibilities

## User Authentication

Provides registration and login interfaces backed by the Gateway authentication endpoints.

User credentials are securely hashed by the Gateway using Argon2id. The frontend never performs cryptographic operations locally.

---

## API Key Provisioning

Authenticated users can generate API keys associated with their account and access tier.

Generated keys are displayed once and are intended to be stored securely by the user.

---

## Streaming Playground

An embedded AI playground allows developers to validate streaming behavior through the Data Plane.

Responses are consumed using the browser's native `ReadableStream` API, preserving chunked transfer and Server-Sent Event semantics.

---

## Responsive Interface

The portal is optimized for desktop and mobile devices and uses Tailwind CSS via CDN for styling.

Dark mode is the default experience.

---

# Technical Stack

| Component  | Technology         | Purpose                          |
| ---------- | ------------------ | -------------------------------- |
| Markup     | HTML5              | User interface                   |
| Logic      | ES6 JavaScript     | Client-side behavior             |
| Styling    | Tailwind CSS       | Responsive design                |
| Streaming  | ReadableStream API | Incremental response consumption |
| Storage    | sessionStorage     | JWT persistence                  |
| Web Server | Nginx Alpine       | Static asset serving             |
| Routing    | SPA fallback       | Client-side navigation           |

---

# Authentication Model

Authentication is delegated entirely to the Gateway.

Supported operations:

- User registration
- User login
- JWT validation
- Account information retrieval

JWT tokens are stored inside browser `sessionStorage` and transmitted through the `Authorization: Bearer` header.

No cookies are required, avoiding localhost cross-origin restrictions during development.

---

# Streaming Playground

The AI playground validates end-to-end streaming functionality through the Gateway.

Supported capabilities:

- OpenAI-compatible chat requests
- Streaming responses
- SSE chunk consumption
- Incremental rendering
- Low time-to-first-token (TTFT)

Responses are rendered progressively without buffering the entire output in memory.

---

# Development

## Prerequisites

- Running Fluxgate Gateway
- Modern browser with Fetch API support
- Local HTTP server

Examples:

- VSCode Live Server
- Python HTTP server
- Nginx

---

## Start Development Server

```bash
python3 -m http.server 3000
```

or

```bash
npx serve .
```

Open:

```text
http://localhost:3000
```

---

# Local HTTPS Setup

The Gateway runs with TLS enabled.

For local development using self-signed certificates, the browser may require manual certificate trust before cross-origin requests can succeed.

Verify connectivity:

```text
https://localhost:8443/health
```

Once the certificate is trusted by the browser, the portal can communicate with the Gateway normally.

---

# Nginx Configuration

Nginx serves the SPA and performs fallback routing.

```nginx
location / {
    try_files $uri $uri/ /index.html;
}
```

Unknown routes are redirected back to `index.html`, ensuring page refreshes do not break client-side navigation.

Static assets are configured with long-lived cache headers and gzip compression is enabled.

---

# Deployment

The frontend is packaged as an Nginx Alpine container.

Responsibilities:

- Static file hosting
- Asset compression
- SPA routing fallback
- Browser caching

No server-side rendering or runtime dependencies are required.

---

# Operational Notes

## Session Persistence

Authentication state is stored inside browser `sessionStorage`.

Sessions are automatically cleared when the browser tab is closed.

---

## Streaming Performance

Streaming responses are rendered incrementally using the browser's `ReadableStream` interface.

No response buffering occurs inside the UI.

---

## Failure Behavior

If the Gateway becomes unavailable:

- Authentication requests fail immediately.
- Existing sessions are invalidated.
- Streaming requests terminate gracefully.
- The UI remains functional and recoverable after reconnection.

---

# Related Services

| Service              | Description                       |
| -------------------- | --------------------------------- |
| `gateway/`           | Rust Data Plane                   |
| `control-plane/`     | Threat analysis and policy engine |
| `docker-compose.yml` | Full stack orchestration          |
| `certs/`             | TLS certificates                  |
| `nginx.conf`         | SPA routing configuration         |
