#!/bin/bash
# Builds the images, compiles test programs in build-env and judges them in the
# server container started through compose (the same way a deployment runs it).
#
# SKIP_BUILD=1 uses what CI already built with a layer cache: the programs in
# tests/sandbox/out and ghcr.io/hugoocoto/runtime-arena-server:latest.
set -euo pipefail
cd "$(dirname "$0")/.."
# compose.yaml requires an admin token; the smoke test never uses the API.
export ARENA_ADMIN_TOKEN="${ARENA_ADMIN_TOKEN:-sandbox-smoke-test-token-not-secret}"

if [ -z "${SKIP_BUILD:-}" ]; then
    docker build -f docker/Dockerfile --target build-env -t runtime-arena-build-env:dev .
    docker compose build arena
    rm -rf tests/sandbox/out
    docker run --rm --user "$(id -u):$(id -g)" -v "$PWD/tests/sandbox:/tests" runtime-arena-build-env:dev /tests/build.sh
fi

docker compose run --rm --no-deps -v "$PWD/tests/sandbox:/tests:ro" arena /tests/judge.sh
