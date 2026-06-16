# --- Stage 1: Build & Compilation ---
FROM rust:1.85-slim-bookworm AS builder
WORKDIR /app

# Install compilation system dependencies
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

# Copy workspace code configuration files
COPY Cargo.toml ./
COPY src ./src
COPY certs ./certs

# Build optimized binary target
RUN cargo build --release

# --- Stage 2: Minimal Execution Runtime ---
FROM debian:bookworm-slim AS runtime
WORKDIR /app

# Install CA certificates to allow outgoing secure client requests to AI backends
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

# Copy compiled binary artifact from the builder phase
COPY --from=builder /app/target/release/gateway /app/fluxgate-gateway

# Copy certs with the correct names - IMPORTANT FIX!
COPY --from=builder /app/certs/*.pem /app/certs/

# Set standard environment pointers
ENV DOWNSTREAM_AI_URL=https://api.openai.com
ENV CARGO_MANIFEST_DIR=/app
ENV TLS_CERT_NAME=api.fluxgate.local+2.pem
ENV TLS_KEY_NAME=api.fluxgate.local+2-key.pem

EXPOSE 8443 9090
CMD ["./fluxgate-gateway"]
