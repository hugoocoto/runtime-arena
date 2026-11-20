#!/bin/bash
# Runs the judge crate's tests, including those that need a real isolate sandbox,
# in a privileged container.
#
# SKIP_BUILD=1 uses an already built runtime-arena-judge-test:dev image (CI).
set -euo pipefail
cd "$(dirname "$0")/.."

if [ -z "${SKIP_BUILD:-}" ]; then
    docker build -f docker/Dockerfile --target judge-test -t runtime-arena-judge-test:dev .
fi
docker run --rm --privileged --cgroupns=private runtime-arena-judge-test:dev "$@"
