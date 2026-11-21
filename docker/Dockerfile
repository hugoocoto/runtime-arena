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
#
# Stage order matters for the classic (non-BuildKit) builder, which builds every
# stage before the requested target: build-env comes first so it builds alone,
# and the judge-test stage comes last so building the server skips it.

FROM debian:trixie-slim AS base
# Interpreters available to shebang submissions (#!/usr/bin/env python3, ...).
RUN apt-get update \
 && apt-get install -y --no-install-recommends python3 ca-certificates \
 && rm -rf /var/lib/apt/lists/*

FROM base AS build-env
RUN apt-get update \
 && apt-get install -y --no-install-recommends gcc g++ libc6-dev make git \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /work
CMD ["bash"]

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
FROM lukemathwalker/cargo-chef:0.1.78-rust-1.100.0-slim-trixie AS chef
WORKDIR /src
# sqlx checks queries against the committed .sqlx/ data instead of a database.
ENV SQLX_OFFLINE=true

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS rust
# Unset in releases. CI sets them to compile faster: its sandbox tests don't run
# the server binary, they only need the image to build.
ARG CARGO_PROFILE_RELEASE_LTO
ARG CARGO_PROFILE_RELEASE_OPT_LEVEL
ARG CARGO_PROFILE_RELEASE_CODEGEN_UNITS
COPY rust-toolchain.toml ./
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --locked -p arena-server --recipe-path recipe.json
COPY . .
RUN cargo build --release --locked -p arena-server \
 && cp target/release/arena-server /arena-server

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

# Runs the judge crate's sandbox tests against a real isolate. Run privileged
# with a private cgroup namespace (scripts/judge-tests.sh). Not published.
FROM chef AS judge-test
RUN apt-get update \
 && apt-get install -y --no-install-recommends libcap2 libseccomp2 python3 \
 && rm -rf /var/lib/apt/lists/*
COPY --from=isolate /src/isolate /usr/local/bin/isolate
COPY --from=isolate /src/default.cf /usr/local/etc/isolate.default
COPY docker/entrypoint.sh /usr/local/bin/entrypoint.sh
COPY rust-toolchain.toml ./
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --tests --locked -p arena-judge --recipe-path recipe.json
COPY . .
RUN cargo test --no-run --locked -p arena-judge
ENV ARENA_ISOLATE_TESTS=1
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["cargo", "test", "--locked", "-p", "arena-judge"]
