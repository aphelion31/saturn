# Multi-stage production build for Saturn (static React Router SPAs + Rust binary)

FROM oven/bun:1.4.4-slim AS bun-bin

FROM node:22-bookworm-slim AS web-builder
WORKDIR /app
COPY --from=bun-bin /usr/local/bin/bun /usr/local/bin/bun
RUN ln -sf /usr/local/bin/bun /usr/local/bin/bunx
COPY scripts/build-web.sh scripts/build-web.sh
COPY web web
RUN ./scripts/build-web.sh

FROM rust:1.98.1-bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs && cargo build --release --locked --bin saturn && rm -rf src
COPY src src
COPY migrations migrations
RUN touch src/main.rs && cargo build --release --locked --bin saturn

FROM debian:bookworm-slim AS runtime
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 1001 saturn \
    && useradd --uid 1001 --gid saturn --create-home saturn \
    && install -d -o saturn -g saturn -m 750 /data/storage
WORKDIR /srv/saturn
COPY --from=builder /app/target/release/saturn /usr/local/bin/saturn
COPY --from=web-builder /app/web/apps/web/build/client /srv/saturn/web/web/build/client
COPY --from=web-builder /app/web/apps/admin/build/client /srv/saturn/web/admin/build/client
COPY --from=web-builder /app/web/apps/space/build/client /srv/saturn/web/space/build/client
RUN chown -R saturn:saturn /srv/saturn
ENV BIND=0.0.0.0:3000 \
    WEB_DIR=/srv/saturn/web \
    STORAGE_DIR=/data/storage \
    RUST_LOG=info,tower_http=warn \
    HOME=/home/saturn
USER saturn
EXPOSE 3000
HEALTHCHECK --interval=15s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -sf http://127.0.0.1:3000/ready >/dev/null || exit 1
ENTRYPOINT ["/usr/local/bin/saturn"]
