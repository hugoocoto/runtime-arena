#!/bin/bash
# Runs in the build-env image: compiles the C test programs into /tests/out.
set -euo pipefail
mkdir -p /tests/out
for p in ok tle mle re; do
    gcc -O2 -o "/tests/out/$p" "/tests/$p.c"
done
cp /tests/ok.py /tests/net.py /tests/out/
chmod +x /tests/out/*
