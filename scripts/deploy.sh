#!/bin/sh
# Update the checkout and images, validate the host, start Arena and wait for HTTPS.
set -eu
cd "$(dirname "$0")/.."

if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    git pull --ff-only
fi
scripts/host-preflight.sh
docker compose pull
docker compose up -d

domain=$(sed -n 's/^ARENA_DOMAIN=//p' .env | tail -n 1)
domain=${domain#\"}
domain=${domain%\"}
url="https://$domain/healthz"
curl_args="-fsS"
if [ "$domain" = "localhost" ]; then
    curl_args="$curl_args -k"
fi

i=0
while [ "$i" -lt 60 ]; do
    if curl $curl_args --max-time 5 "$url" >/dev/null 2>&1; then
        printf 'Arena is ready at https://%s\n' "$domain"
        exit 0
    fi
    i=$((i + 1))
    sleep 1
done

echo "Arena did not become ready at $url" >&2
docker compose ps
exit 1
