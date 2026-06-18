# --- Stage 1: Build & Dependency Caching ---
FROM rust:1.96-slim-bookworm AS builder
WORKDIR /app

# Install compilation system dependencies
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

# Cache dependencies: Create a dummy project to build dependencies first
COPY Cargo.toml ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src

# Copy actual workspace code configuration files
COPY src ./src
COPY certs ./certs

# Touch the main file to force Cargo to recompile our actual code, not the dummy file
RUN touch src/main.rs

# Build optimized binary target
RUN cargo build --release

# --- Stage 2: Minimal Execution Runtime ---
FROM debian:bookworm-slim AS runtime
WORKDIR /app

# Create a non-root user for security
RUN useradd -m -s /bin/bash fluxgate

# Install CA certificates to allow outgoing secure client requests
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

# Copy compiled binary artifact and assign ownership to the non-root user
COPY --from=builder --chown=fluxgate:fluxgate /app/target/release/gateway /app/fluxgate-gateway
COPY --from=builder --chown=fluxgate:fluxgate /app/certs/*.pem /app/certs/

# Secure the container by switching to the non-root user
USER fluxgate

# Set standard environment pointers
ENV TLS_CERT_PATH=/app/certs/cert.pem
ENV TLS_KEY_PATH=/app/certs/key.pem

EXPOSE 8443 9090
CMD ["./fluxgate-gateway"]