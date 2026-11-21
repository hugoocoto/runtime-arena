#!/bin/sh
# Check the host prerequisites before starting a contest.
set -eu

errors=0

ok() {
    printf 'ok: %s\n' "$1"
}

fail() {
    printf 'error: %s\n' "$1" >&2
    errors=$((errors + 1))
}

if [ "$(uname -s)" = Linux ]; then
    ok "Linux host"
else
    fail "the judge host must run Linux"
fi

if [ -f /sys/fs/cgroup/cgroup.controllers ]; then
    ok "cgroup v2"
else
    fail "cgroup v2 is not mounted at /sys/fs/cgroup"
fi

if command -v docker >/dev/null 2>&1; then
    if docker info >/dev/null 2>&1; then
        ok "Docker is running"
    else
        fail "Docker is installed but the daemon is unavailable"
    fi
    if docker compose version >/dev/null 2>&1; then
        ok "Docker Compose"
    else
        fail "Docker Compose is unavailable"
    fi
else
    fail "Docker is not installed"
fi

if command -v curl >/dev/null 2>&1; then
    ok "curl"
else
    fail "curl is required for the HTTPS readiness check"
fi

if [ ! -f .env ]; then
    fail "missing .env; copy .env.example to .env first"
else
    admin=$(sed -n 's/^ARENA_ADMIN_TOKEN=//p' .env | tail -n 1)
    admin=${admin#\"}
    admin=${admin%\"}
    if [ "${#admin}" -ge 24 ]; then
        ok "admin token is configured"
    else
        fail "ARENA_ADMIN_TOKEN must be at least 24 characters in .env"
    fi

    join=$(sed -n 's/^ARENA_JOIN_CODE=//p' .env | tail -n 1)
    join=${join#\"}
    join=${join%\"}
    if [ -z "$join" ] || [ "${#join}" -ge 6 ]; then
        ok "join code configuration"
    else
        fail "ARENA_JOIN_CODE must be empty or at least 6 characters"
    fi

    domain=$(sed -n 's/^ARENA_DOMAIN=//p' .env | tail -n 1)
    domain=${domain#\"}
    domain=${domain%\"}
    if [ -n "$domain" ] && [ "$domain" != "arena.example.com" ]; then
        ok "HTTPS domain configured ($domain)"
    else
        fail "ARENA_DOMAIN must be replaced with the public HTTPS domain"
    fi
fi

if command -v docker >/dev/null 2>&1 && [ -f .env ]; then
    if docker compose config --quiet >/dev/null 2>&1; then
        ok "Compose configuration"
    else
        fail "Compose configuration is invalid; check .env"
    fi
fi

if [ "$errors" -eq 0 ]; then
    printf 'Host is ready. Run: scripts/deploy.sh\n'
    printf 'Participants connect at: https://%s\n' "$domain"
else
    printf '%s prerequisite(s) need attention.\n' "$errors" >&2
    exit 1
fi
