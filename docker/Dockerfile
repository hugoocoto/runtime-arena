#
# Targets:
#   server     the arena server with the isolate sandbox (run privileged, see compose.yaml)
#   build-env  the environment participants build their submissions in
#
# Both derive from `base`, so a binary built in build-env links against the same
# libraries the judge runs it with, and shebang scripts find the same interpreters.
#
# Base image tags are literal (not ARGs) so Dependabot can update them. The Rust
# toolchain actually used is the one pinned in rust-toolchain.toml; keep the
# cargo-chef image on the same Rust version to avoid a toolchain download.

FROM debian:trixie-slim AS base
# Interpreters available to shebang submissions (#!/usr/bin/env python3, ...).
RUN apt-get update \
 && apt-get install -y --no-install-recommends python3 ca-certificates \
 && rm -rf /var/lib/apt/lists/*

FROM debian:trixie-slim AS isolate
ARG ISOLATE_REF=v2.7
ARG ISOLATE_COMMIT=8f185bb37f3f23e29b33b0c7727c91c13429abe3
RUN apt-get update \
 && apt-get install -y --no-install-recommends gcc libc6-dev make pkg-config git ca-certificates \
      libcap-dev libseccomp-dev \
 && rm -rf /var/lib/apt/lists/*
RUN git clone --depth 1 --branch "${ISOLATE_REF}" https://github.com/ioi/isolate.git /src
WORKDIR /src
# Refuse a tag that was moved after we reviewed it.
RUN test "$(git rev-parse HEAD)" = "${ISOLATE_COMMIT}"
RUN make isolate default.cf

# cargo-chef builds the dependencies in their own layer, so a source change only
# recompiles the workspace crates. Works with the classic builder (no BuildKit).
FROM lukemathwalker/cargo-chef:0.1.78-rust-1.99.1-slim-trixie AS chef
WORKDIR /src

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS rust
COPY rust-toolchain.toml ./
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --locked -p arena-server --recipe-path recipe.json
COPY . .
RUN cargo build --release --locked -p arena-server \
 && cp target/release/arena-server /arena-server

FROM base AS build-env
RUN apt-get update \
 && apt-get install -y --no-install-recommends gcc g++ libc6-dev make git \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /work
CMD ["bash"]

FROM base AS server
RUN apt-get update \
 && apt-get install -y --no-install-recommends libcap2 libseccomp2 \
 && rm -rf /var/lib/apt/lists/*
COPY --from=isolate /src/isolate /usr/local/bin/isolate
COPY --from=isolate /src/default.cf /usr/local/etc/isolate.default
COPY --from=rust /arena-server /usr/local/bin/arena-server
COPY docker/entrypoint.sh /usr/local/bin/entrypoint.sh
ENV ARENA_LISTEN=0.0.0.0:8080 \
    ARENA_DATA=/data
VOLUME /data
EXPOSE 8080
HEALTHCHECK --interval=10s --timeout=3s CMD python3 -c "import urllib.request,sys; sys.exit(urllib.request.urlopen('http://127.0.0.1:8080/healthz').status != 200)"
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["arena-server"]
