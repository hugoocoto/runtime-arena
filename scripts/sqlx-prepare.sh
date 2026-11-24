#!/bin/bash
# Regenerates .sqlx/, the saved query metadata that lets sqlx check queries at
# compile time without a database (CI and Docker build with SQLX_OFFLINE=true).
# Run after changing a query or a migration. With --check (CI), fails if .sqlx/
# is out of date instead of rewriting it. Needs sqlx-cli:
#   cargo install --locked sqlx-cli --no-default-features --features sqlite
set -euo pipefail
cd "$(dirname "$0")/.."

db="$PWD/target/sqlx-prepare.db"
mkdir -p target
rm -f "$db"
export DATABASE_URL="sqlite:$db"
export SQLX_OFFLINE=false
sqlx database create
sqlx migrate run --source crates/server/migrations
cargo sqlx prepare --workspace "$@" -- --all-targets
