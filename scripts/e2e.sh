#!/bin/bash
# Boots the server image in a throwaway container, separate from any Compose
# deployment, and plays a short contest through its API with the real judge.
#
# SKIP_BUILD=1 uses what CI already built: $ARENA_IMAGE and tests/sandbox/out.
set -euo pipefail
cd "$(dirname "$0")/.."

image=${ARENA_IMAGE:-runtime-arena-e2e:dev}
admin=e2e-admin-token-not-secret-0123456789
join_code=e2e-join-code

if [ -z "${SKIP_BUILD:-}" ]; then
    docker build -f docker/Dockerfile --target server -t "$image" .
    docker build -f docker/Dockerfile --target build-env -t runtime-arena-build-env:dev .
    rm -rf tests/sandbox/out
    docker run --rm --user "$(id -u):$(id -g)" -v "$PWD/tests/sandbox:/tests" runtime-arena-build-env:dev /tests/build.sh
fi

container=$(docker run -d --privileged --cgroupns=private -p 127.0.0.1::8080 \
    -e ARENA_ADMIN_TOKEN="$admin" -e ARENA_JOIN_CODE="$join_code" \
    -e ARENA_RATE_LIMIT=0 -e ARENA_UPDATE_CHECK=off "$image")
cleanup() {
    status=$?
    if [ "$status" -ne 0 ]; then
        docker logs --tail 40 "$container" >&2 || true
    fi
    docker rm -fv "$container" >/dev/null
    exit "$status"
}
trap cleanup EXIT

port=$(docker port "$container" 8080/tcp | head -n1 | sed 's/.*://')
python3 -I tests/e2e/run.py "http://127.0.0.1:$port" "$admin" "$join_code"
