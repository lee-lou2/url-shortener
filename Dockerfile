FROM rust:1.88-alpine3.21 AS builder

RUN apk add --no-cache musl-dev pkgconf
WORKDIR /build
COPY Cargo.toml Cargo.lock askama.toml ./
COPY src/ ./src/
COPY views/ ./views/
COPY migrations/ ./migrations/
RUN cargo build --locked --release

FROM alpine:3.21

RUN apk add --no-cache ca-certificates \
    && addgroup -S app \
    && adduser -S -G app app
COPY --from=builder /build/target/release/url-shortener /usr/local/bin/url-shortener
USER app

ENV RUST_LOG=info SERVER_PORT=3000
EXPOSE 3000
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD wget -q -T 2 -O /dev/null http://127.0.0.1:3000/health || exit 1

ENTRYPOINT ["url-shortener"]
