FROM rust:1.75 as builder

WORKDIR /app

# Copy files
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations

# Build
RUN cargo build --release

# Runtime
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libpq5 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary
COPY --from=builder /app/target/release/tradingbots /app/tradingbots
COPY --from=builder /app/migrations ./migrations
COPY --from=builder /app/.env ./

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD /app/tradingbots health || exit 1

EXPOSE 8080

CMD ["/app/tradingbots"]
