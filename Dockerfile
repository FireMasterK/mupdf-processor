FROM rust:slim AS build

WORKDIR /app

RUN --mount=type=cache,target=/var/cache/apt \
    apt-get update && \
    apt-get install -y --no-install-recommends \
    build-essential \
    clang \
    libclang-dev \
    libfontconfig1-dev \
    libfreetype6-dev \
    pkg-config \
    ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY testdata ./testdata

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release && \
    cp target/release/mupdf-processor /app/mupdf-processor

FROM debian:stable-slim

RUN --mount=type=cache,target=/var/cache/apt \
    apt-get update && \
    apt-get install -y --no-install-recommends \
    ca-certificates \
    libfontconfig1 \
    libfreetype6 && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=build /app/mupdf-processor /app/mupdf-processor

EXPOSE 8080

CMD ["/app/mupdf-processor"]
