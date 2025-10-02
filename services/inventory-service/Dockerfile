# Build stage
FROM rust:1-slim-bookworm AS builder
WORKDIR /repo

# system deps for SQLx/Postgres + rdkafka (zlib)
RUN apt-get update && apt-get install -y libssl-dev pkg-config cmake g++ zlib1g-dev && rm -rf /var/lib/apt/lists/*

# Ensure zlib is linked dynamically in Linux builds
ENV LIBZ_SYS_STATIC=0 \
    ZLIB_STATIC=0

# Leverage caching by copying manifests first
COPY services/Cargo.toml services/Cargo.lock ./
COPY services/auth-service/Cargo.toml services/auth-service/Cargo.toml
COPY services/order-service/Cargo.toml services/order-service/Cargo.toml
COPY services/product-service/Cargo.toml services/product-service/Cargo.toml
COPY services/payment-service/Cargo.toml services/payment-service/Cargo.toml
COPY services/integration-gateway/Cargo.toml services/integration-gateway/Cargo.toml
COPY services/analytics-service/Cargo.toml services/analytics-service/Cargo.toml
COPY services/inventory-service/Cargo.toml services/inventory-service/Cargo.toml

# Copy full sources and build only this package
COPY services/ services/
RUN cargo build --release --manifest-path services/Cargo.toml --target-dir /repo/target -p inventory-service

# Runtime
FROM debian:bookworm-slim
# Runtime deps: SSL and zlib for any dynamic linkage
RUN apt-get update && apt-get install -y ca-certificates libssl3 zlib1g && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /repo/target/release/inventory-service /app/inventory-service
ENV RUST_LOG=info
EXPOSE 8087
CMD ["/app/inventory-service"]
