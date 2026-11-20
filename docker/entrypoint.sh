#!/bin/bash
# Prepares the container's cgroup tree for isolate, writes the isolate config,
# then execs the given command.
#
# Requires `privileged: true` and a private cgroup namespace (compose.yaml).
set -euo pipefail

CG=/sys/fs/cgroup

if [ ! -f "$CG/cgroup.subtree_control" ]; then
    echo "entrypoint: $CG is not a cgroup v2 mount; isolate cannot run" >&2
    exit 1
fi

# cgroup v2 forbids processes in a cgroup that delegates controllers to children,
# so move everything out of the namespace root first.
mkdir -p "$CG/init"
for pid in $(cat "$CG/cgroup.procs"); do
    echo "$pid" > "$CG/init/cgroup.procs" 2>/dev/null || true
done
echo "+cpuset +memory" > "$CG/cgroup.subtree_control"
mkdir -p "$CG/isolate"
echo "+cpuset +memory" > "$CG/isolate/cgroup.subtree_control"

# JUDGE_CPUS: comma-separated CPU ids, one sandbox box per CPU (box i -> i-th CPU).
# Empty means boxes are not pinned.
JUDGE_CPUS="${JUDGE_CPUS:-}"
{
    sed -e 's|^cg_root.*|cg_root = /sys/fs/cgroup/isolate|' \
        -e 's|^subid_user.*|first_uid = 60000\nfirst_gid = 60000\nnum_boxes = 1000|' \
        /usr/local/etc/isolate.default
    if [ -n "$JUDGE_CPUS" ]; then
        i=0
        IFS=',' read -ra cpus <<< "$JUDGE_CPUS"
        for cpu in "${cpus[@]}"; do
            echo "box$i.cpus = $cpu"
            echo "box$i.mems = 0"
            i=$((i + 1))
        done
    fi
} > /usr/local/etc/isolate

mkdir -p /var/local/lib/isolate "${ARENA_DATA:-/data}"
exec "$@"
