#!/bin/bash
# Mock contest and stress test against a running server, using the `arena` CLI:
# uploads problems/sum, creates participants, starts a round, and has every
# participant submit several solutions at the same time.
#
#   ARENA_SERVER=http://localhost:8080 ARENA_ADMIN_TOKEN=... ARENA_JOIN_CODE=... scripts/mock-contest.sh [users] [rounds-of-submissions]
#
# Needs Docker (to build the solutions in the build-env image) and cargo.
set -euo pipefail
cd "$(dirname "$0")/.."

users=${1:-20}
repeat=${2:-3}
: "${ARENA_SERVER:=http://localhost:8080}"
: "${ARENA_ADMIN_TOKEN:?set ARENA_ADMIN_TOKEN}"
: "${ARENA_JOIN_CODE:?set ARENA_JOIN_CODE}"
export ARENA_SERVER ARENA_ADMIN_TOKEN ARENA_JOIN_CODE
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
# Keep the user's own CLI config untouched.
export ARENA_CONFIG="$work/config.toml"

cargo build -q --release -p arena-cli
arena=target/release/arena
build_image=${BUILD_IMAGE:-runtime-arena-build-env:dev}
docker image inspect "$build_image" >/dev/null 2>&1 ||
    docker build -q -f docker/Dockerfile --target build-env -t "$build_image" . >/dev/null

echo "== problem"
[ -f problems/sum/tests/04.in ] || python3 -I problems/sum/gen.py
"$arena" admin problem add problems/sum --slug mock-sum

echo "== solutions"
docker run --rm --user "$(id -u):$(id -g)" -v "$PWD/problems/sum/solutions:/src:ro" -v "$work:/out" \
    "$build_image" sh -c 'gcc -O2 -o /out/fast /src/fast.c && gcc -O2 -o /out/simple /src/simple.c'
cp problems/sum/solutions/wrong.py "$work/wrong"
solutions=(fast simple wrong)

echo "== $users participants"
run=$(date +%s)
tokens=()
for i in $(seq 1 "$users"); do
    # Each participant gets its own config, and the token is read back from it.
    config="$work/user-$i.toml"
    printf 'mock-password\n' | ARENA_CONFIG="$config" ARENA_TOKEN='' "$arena" signup "$ARENA_SERVER" \
        --username "mock$run-$i" --display-name "Mock $i" --password-stdin >/dev/null
    tokens+=("$(sed -n 's/^token = "\(.*\)"$/\1/p' "$config")")
done

echo "== round"
round=$("$arena" admin round create --name "Mock $run" --minutes 30 --cooldown 0 mock-sum |
    sed -n 's/^Round \([0-9]*\):.*/\1/p')
"$arena" admin round start "$round" >/dev/null

echo "== $((users * repeat)) submissions at once"
start=$(date +%s.%N)
pids=()
for r in $(seq 1 "$repeat"); do
    for i in $(seq 0 $((users - 1))); do
        sol=${solutions[$(((i + r) % 3))]}
        ARENA_TOKEN=${tokens[$i]} "$arena" submit mock-sum "$work/$sol" \
            --src "problems/sum/solutions" >"$work/out-$r-$i" 2>&1 &
        pids+=($!)
    done
done
ok=0 rejected=0 errors=0
for pid in "${pids[@]}"; do
    if wait "$pid"; then ok=$((ok + 1)); else
        case $? in 1) rejected=$((rejected + 1)) ;; *) errors=$((errors + 1)) ;; esac
    fi
done
elapsed=$(echo "$(date +%s.%N) - $start" | awk '{printf "%.1f", $1 - $3}' 2>/dev/null || true)
echo "accepted $ok, rejected $rejected, CLI errors $errors, in ${elapsed:-?} s"
if [ "$errors" -gt 0 ]; then
    echo "-- first errors:"
    grep -l '^error' "$work"/out-* | head -3 | xargs -r cat
fi

echo "== verdicts"
"$arena" admin submissions --round "$round" | awk '{v=$4" "$5; if ($4 ~ /^(accepted|pending)$/) v=$4; c[v]++} END {for (k in c) print "  " c[k], k}'
"$arena" admin round stop "$round" >/dev/null
[ "$errors" -eq 0 ]
