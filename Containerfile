# Antigravity Server - Multi-stage Container Build
# Optimized for minimal image size (<100MB) with static musl binary
#
# Build: podman build -t antigravity-server .
# Run:   podman run -d -p 8045:8045 -v ./data:/var/lib/antigravity antigravity-server

# ============================================================================
# Stage 1: Builder - Rust toolchain with musl for static linking
# ============================================================================
FROM docker.io/rust:1.83-alpine AS builder

# Install build dependencies
RUN apk add --no-cache \
    musl-dev \
    openssl-dev \
    openssl-libs-static \
    pkgconf \
    git \
    perl \
    make \
    # Required for ring crate (cryptography)
    linux-headers

# Set environment for static linking
ENV OPENSSL_STATIC=1 \
    OPENSSL_LIB_DIR=/usr/lib \
    OPENSSL_INCLUDE_DIR=/usr/include \
    PKG_CONFIG_ALLOW_CROSS=1 \
    RUSTFLAGS="-C target-feature=+crt-static -C link-arg=-s"

WORKDIR /build

# Copy manifests first for better layer caching
COPY src-tauri/Cargo.toml src-tauri/Cargo.lock ./

# Create minimal build.rs that works without Tauri context
RUN echo 'fn main() {}' > build.rs

# Create dummy lib.rs to cache dependencies
RUN mkdir -p src/bin src/proxy src/commands src/modules src/models src/utils && \
    echo "pub mod proxy; pub mod commands; pub mod modules; pub mod models; pub mod utils; pub mod error;" > src/lib.rs && \
    echo "fn main() {}" > src/main.rs && \
    echo "fn main() {}" > src/bin/server.rs && \
    touch src/proxy/mod.rs src/commands/mod.rs src/modules/mod.rs src/models/mod.rs src/utils/mod.rs src/error.rs && \
    cargo build --release --bin antigravity-server 2>/dev/null || true

# Copy actual source code
COPY src-tauri/src ./src

# Build the server binary (release profile optimized for size)
RUN cargo build --release --bin antigravity-server && \
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
# Stage 3: Runtime - Minimal Alpine image
# ============================================================================
FROM docker.io/alpine:3.21 AS runtime

# Labels for container metadata
LABEL org.opencontainers.image.title="Antigravity Server" \
      org.opencontainers.image.description="Headless proxy server for AI API management" \
      org.opencontainers.image.version="3.3.15" \
      org.opencontainers.image.vendor="Antigravity" \
      org.opencontainers.image.source="https://github.com/lbjlaq/Antigravity-Manager"

# Install runtime dependencies (minimal)
RUN apk add --no-cache \
    ca-certificates \
    tzdata \
    # For WARP (WireGuard userspace)
    wireguard-tools \
    # For health checks
    curl && \
    # Create non-root user
    addgroup -g 1000 antigravity && \
    adduser -u 1000 -G antigravity -h /var/lib/antigravity -D antigravity && \
    # Create data directories
    mkdir -p /var/lib/antigravity/accounts /var/lib/antigravity/logs && \
    chown -R antigravity:antigravity /var/lib/antigravity

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
