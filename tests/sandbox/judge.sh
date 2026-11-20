#!/bin/bash
# Runs in the server image: judges each program in /tests/out with isolate and
# checks the verdict. Exits non-zero on the first unexpected result.
set -uo pipefail

fail=0
isolate --cg --box-id=0 --cleanup >/dev/null 2>&1
isolate --cg --box-id=0 --init >/dev/null || { echo "FAIL: isolate --init"; exit 1; }
box=/var/local/lib/isolate/0/box

run() {
    cp "/tests/out/$1" "$box/sol"
    echo "3 4" > "$box/in"
    rm -f "$box/out" /tmp/meta
    isolate --cg --box-id=0 --cg-mem=262144 --time=1 --wall-time=3 --processes=1 \
        --dir=/usr --stdin=in --stdout=out --stderr=err --meta=/tmp/meta \
        --run -- ./sol >/dev/null 2>&1
    meta=$(tr '\n' ' ' < /tmp/meta)
    out=$(cat "$box/out" 2>/dev/null || true)
}

check() {  # name, description, condition
    if eval "$3"; then echo "ok    $1: $2"; else echo "FAIL  $1: $2"; echo "      meta: $meta"; echo "      out:  $out"; fail=1; fi
}

run ok;     check ok     "accepted, prints 7"        '[[ "$out" == 7 && "$meta" == *"exitcode:0"* && "$meta" != *status:* ]]'
run ok.py;  check ok.py  "shebang script runs"       '[[ "$out" == 7 && "$meta" == *"exitcode:0"* ]]'
run tle;    check tle    "time limit exceeded"       '[[ "$meta" == *"status:TO"* ]]'
run mle;    check mle    "killed by memory limit"    '[[ "$meta" == *"cg-oom-killed:1"* ]]'
run re;     check re     "segfault reported"         '[[ "$meta" == *"status:SG"* && "$meta" == *"exitsig:11"* ]]'
run net.py; check net.py "network unreachable"       '[[ "$out" != *"NETWORK OPEN"* && "$meta" == *"status:RE"* ]]'

isolate --cg --box-id=0 --cleanup >/dev/null
exit $fail
