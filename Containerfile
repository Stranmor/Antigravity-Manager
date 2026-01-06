# Antigravity Server - Multi-stage Container Build
# Runtime: Debian Bookworm slim (~100MB) with glibc binary
#
# Build: podman build -t antigravity-server .
# Run:   podman run -d -p 8045:8045 -v ./data:/var/lib/antigravity antigravity-server

# ============================================================================
# Stage 1: Builder - Rust toolchain with glibc (Debian-based)
# ============================================================================
FROM docker.io/rust:1.92-slim-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    git \
    perl \
    make \
    && rm -rf /var/lib/apt/lists/*

# Set environment for optimized build
ENV RUSTFLAGS="-C link-arg=-s"

WORKDIR /build

# Copy manifests first for better layer caching
COPY src-tauri/Cargo.toml src-tauri/Cargo.lock ./

# Create minimal build.rs that works without Tauri context
RUN echo 'fn main() {}' > build.rs

# Create dummy lib.rs to cache dependencies (headless mode)
RUN mkdir -p src/bin src/proxy src/commands src/modules src/models src/utils && \
    echo "pub mod proxy; pub mod modules; pub mod models; pub mod utils; pub mod error;" > src/lib.rs && \
    echo "fn main() {}" > src/main.rs && \
    echo "fn main() {}" > src/bin/server.rs && \
    touch src/proxy/mod.rs src/modules/mod.rs src/models/mod.rs src/utils/mod.rs src/error.rs && \
    cargo build --release --no-default-features --features headless --bin antigravity-server 2>/dev/null || true

# Copy actual source code
COPY src-tauri/src ./src

# Build the server binary (release profile, headless mode - no Tauri/GTK deps)
RUN cargo build --release --no-default-features --features headless --bin antigravity-server && \
    strip /build/target/release/antigravity-server

# ============================================================================
# Stage 2: WARP Tools - Build wgcf and wireproxy for Cloudflare WARP support
# ============================================================================
FROM docker.io/golang:1.23-alpine AS warp-builder

RUN apk add --no-cache git

# Build wgcf (WARP configuration generator)
WORKDIR /wgcf
RUN git clone --depth=1 https://github.com/ViRb3/wgcf.git . && \
    CGO_ENABLED=0 go build -ldflags="-s -w" -o /usr/local/bin/wgcf .

# Build wireproxy (WireGuard to SOCKS5 proxy)
WORKDIR /wireproxy
RUN git clone --depth=1 https://github.com/pufferffish/wireproxy.git . && \
    CGO_ENABLED=0 go build -ldflags="-s -w" -o /usr/local/bin/wireproxy ./cmd/wireproxy

# ============================================================================
# Stage 3: Runtime - Minimal Debian image (for glibc compatibility)
# ============================================================================
FROM docker.io/debian:bookworm-slim AS runtime

# Labels for container metadata
LABEL org.opencontainers.image.title="Antigravity Server" \
      org.opencontainers.image.description="Headless proxy server for AI API management" \
      org.opencontainers.image.version="3.3.15" \
      org.opencontainers.image.vendor="Antigravity" \
      org.opencontainers.image.source="https://github.com/lbjlaq/Antigravity-Manager"

# Install runtime dependencies (minimal)
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    tzdata \
    wireguard-tools \
    curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -u 1000 -m -d /var/lib/antigravity antigravity \
    && mkdir -p /var/lib/antigravity/accounts /var/lib/antigravity/logs \
    && chown -R antigravity:antigravity /var/lib/antigravity

# Copy binaries from builders
COPY --from=builder /build/target/release/antigravity-server /usr/local/bin/
COPY --from=warp-builder /usr/local/bin/wgcf /usr/local/bin/
COPY --from=warp-builder /usr/local/bin/wireproxy /usr/local/bin/

# Make binaries executable
RUN chmod +x /usr/local/bin/antigravity-server \
             /usr/local/bin/wgcf \
             /usr/local/bin/wireproxy

# Copy entrypoint script
COPY deploy/scripts/entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

# Environment defaults
ENV ANTIGRAVITY_DATA_DIR=/var/lib/antigravity \
    ANTIGRAVITY_PROXY_PORT=8045 \
    ANTIGRAVITY_ADMIN_PORT=9101 \
    ANTIGRAVITY_ALLOW_LAN=true \
    ANTIGRAVITY_ENABLE_LOGGING=true \
    RUST_LOG=info,antigravity_tools_lib=debug \
    TZ=UTC

# Expose ports
EXPOSE 8045 9101

# Health check
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -sf http://localhost:8045/healthz || exit 1

# Run as non-root user
USER antigravity
WORKDIR /var/lib/antigravity

# Volume for persistent data
VOLUME ["/var/lib/antigravity"]

ENTRYPOINT ["/entrypoint.sh"]
CMD ["antigravity-server"]
