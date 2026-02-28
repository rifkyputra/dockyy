# ── Build dashboard ──
FROM node:22-alpine AS dashboard
WORKDIR /app/dashboard
COPY dashboard/package.json dashboard/package-lock.json* ./
RUN npm ci --production=false
COPY dashboard/ .
RUN npm run build

# ── Build server ──
FROM rust:1.88-alpine AS server
RUN apk add --no-cache musl-dev pkgconfig openssl-dev openssl-libs-static
WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
COPY crates/ crates/
COPY --from=dashboard /app/dashboard/dist/ dashboard/dist/
RUN cargo build --release

# ── Runtime ──
FROM alpine:3.20
RUN apk add --no-cache ca-certificates docker-cli
COPY --from=server /app/target/release/dockyy /usr/local/bin/dockyy
EXPOSE 3000
ENV DOCKYY_DATA_DIR=/data
VOLUME /data
ENTRYPOINT ["dockyy"]
