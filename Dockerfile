# syntax=docker/dockerfile:1.7

FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p aviary-cli -p aviary-telegram -p aviary-discord \
    && mkdir -p /out \
    && cp target/release/aviary target/release/aviary-telegram target/release/aviary-discord /out/

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 1000 aviary \
    && mkdir -p /data \
    && chown aviary:aviary /data
COPY --from=build /out/ /usr/local/bin/
ENV COLIBRI_BASE_URL=http://colibri:8000/v1 \
    DATABASE_URL=sqlite:///data/aviary.db \
    RUST_LOG=info
VOLUME ["/data"]
WORKDIR /data
USER aviary
CMD ["aviary", "status"]
