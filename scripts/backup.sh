#!/bin/sh
# Make a consistent backup of the SQLite database and submissions.
# Usage: scripts/backup.sh [directory]
set -eu
cd "$(dirname "$0")/.."

destination=${1:-backups}
case "$destination" in
    /*) backup_dir=$destination ;;
    *) backup_dir=$(pwd)/$destination ;;
esac
mkdir -p "$backup_dir"
container=$(docker compose ps -q arena)
[ -n "$container" ] || {
    echo "error: the arena container is not running" >&2
    exit 1
}
running=$(docker inspect -f '{{.State.Running}}' "$container")
if [ "$running" != true ]; then
    echo "error: the arena container is not running" >&2
    exit 1
fi

stamp=$(date -u +%Y%m%dT%H%M%SZ)
archive=$backup_dir/"arena-data-$stamp.tar.gz"
trap 'docker compose start arena >/dev/null' EXIT
docker compose stop arena >/dev/null
docker run --rm --volumes-from "$container" -v "$backup_dir:/backup" \
    alpine:3.22 tar czf "/backup/arena-data-$stamp.tar.gz" -C /data .
printf 'Backup written to %s\n' "$archive"
