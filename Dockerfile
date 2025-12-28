# Build stage
FROM rust:alpine AS builder

# Install build dependencies
RUN apk add --no-cache musl-dev pkgconfig

WORKDIR /app

# Copy manifests
COPY Cargo.toml Cargo.lock ./

# Create dummy files to cache dependencies
RUN mkdir src && \
    echo "fn main() {}" > src/main.rs && \
    echo "" > src/lib.rs

# Build dependencies (this layer will be cached)
RUN cargo build --release && rm -rf src

# Copy source code and config files
COPY src ./src
COPY views ./views
COPY migrations ./migrations
COPY askama.toml ./

# Touch main.rs to trigger rebuild
RUN touch src/main.rs

# Build the application
RUN cargo build --release

# Runtime stage
FROM alpine:3.20

# Install runtime dependencies
RUN apk add --no-cache ca-certificates

WORKDIR /app

# Copy the binary from builder
COPY --from=builder /app/target/release/url-shortener .

# Copy views and migrations
COPY --from=builder /app/views ./views
COPY --from=builder /app/migrations ./migrations

# Create non-root user
RUN addgroup -g 1000 app && \
    adduser -D -s /bin/sh -u 1000 -G app app && \
    chown -R app:app /app

USER app

# Expose port
EXPOSE 3000

# Set environment variables
ENV RUST_LOG=info
ENV SERVER_PORT=3000

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD wget --no-verbose --tries=1 --spider http://localhost:3000/ || exit 1

# Run the binary
CMD ["./url-shortener"]
