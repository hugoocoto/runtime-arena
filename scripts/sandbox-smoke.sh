#!/bin/bash
# Builds the images, compiles test programs in build-env and judges them in the
# server container started through compose (the same way a deployment runs it).
#
# SKIP_BUILD=1 uses already built images (CI builds them with a layer cache):
#   runtime-arena-build-env:dev and ghcr.io/hugoocoto/runtime-arena-server:latest
set -euo pipefail
cd "$(dirname "$0")/.."

if [ -z "${SKIP_BUILD:-}" ]; then
    docker build -f docker/Dockerfile --target build-env -t runtime-arena-build-env:dev .
    docker compose build arena
fi

rm -rf tests/sandbox/out
docker run --rm --user "$(id -u):$(id -g)" -v "$PWD/tests/sandbox:/tests" runtime-arena-build-env:dev /tests/build.sh
docker compose run --rm --no-deps -v "$PWD/tests/sandbox:/tests:ro" arena /tests/judge.sh
