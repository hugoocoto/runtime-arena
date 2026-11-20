#!/bin/bash
# Reduces timing noise on the machine that hosts a contest, following the checks
# of isolate-check-environment. Changes last until reboot.
#
#   scripts/host-tuning.sh           print what would be changed
#   sudo scripts/host-tuning.sh --apply
#
# Not run automatically. Disabling address space randomisation weakens the
# host's security, so only do this on a machine dedicated to the contest.
set -euo pipefail

apply=
[ "${1:-}" = "--apply" ] && apply=1

set_value() {  # file value
    [ -e "$1" ] || return 0
    if [ -n "$apply" ]; then
        echo "$2" > "$1" && echo "set   $1 = $2"
    else
        echo "would set $1 = $2 (now: $(cat "$1" 2>/dev/null | head -c 60))"
    fi
}

for gov in /sys/devices/system/cpu/cpufreq/policy*/scaling_governor; do
    set_value "$gov" performance
done
set_value /sys/devices/system/cpu/intel_pstate/no_turbo 1
set_value /sys/devices/system/cpu/cpufreq/boost 0
set_value /sys/devices/system/cpu/smt/control off
set_value /proc/sys/kernel/randomize_va_space 0
set_value /sys/kernel/mm/transparent_hugepage/enabled never
set_value /sys/kernel/mm/transparent_hugepage/defrag never
set_value /sys/kernel/mm/transparent_hugepage/khugepaged/defrag 0
set_value /proc/sys/kernel/core_pattern core

if [ -z "$apply" ]; then
    echo
    echo "Swap: memory limits hold with cgroup swap accounting, but swapping adds"
    echo "timing noise. Consider 'swapoff -a' for the duration of the contest."
fi
