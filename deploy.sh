#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"
name=url-shortener
image="${IMAGE_NAME:-url-shortener}:$(git rev-parse --short HEAD)"
network="${DOCKER_NETWORK:-lou2-net}"
env_file="${ENV_FILE:-.env}"
previous="${name}-previous"
saved=false
started=false

rollback() {
  if "$started"; then
    docker logs --tail 50 "$name" >&2 || true
    docker rm -f "$name" >/dev/null || true
  fi
  if "$saved"; then
    if docker rename "$previous" "$name" && docker start "$name" >/dev/null; then
      echo "Restored the previous container. Check database migration compatibility." >&2
    else
      echo "Could not restore the previous container." >&2
    fi
  fi
}

ready() {
  docker exec "$name" sh -ec '
    wget -q -T 2 -O /dev/null http://127.0.0.1:3000/health
    wget -q -T 3 -O /dev/null http://127.0.0.1:3000/ready
  ' >/dev/null 2>&1
}

docker network inspect "$network" >/dev/null
test -r "$env_file"
docker build -t "$image" .
trap rollback ERR
if docker container inspect "$name" >/dev/null 2>&1; then
  if docker container inspect "$previous" >/dev/null 2>&1; then
    docker rm "$previous" >/dev/null
  fi
  docker rename "$name" "$previous"
  saved=true
  docker stop -t 25 "$previous" >/dev/null
fi

started=true
docker run -d --name "$name" --network "$network" --env-file "$env_file" \
  --cpus=1 --memory=512m --restart unless-stopped --stop-timeout 25 "$image" >/dev/null

deadline=$((SECONDS + 60))
until ready; do
  if (( SECONDS >= deadline )); then
    echo "Replacement container failed liveness or readiness checks." >&2
    rollback
    exit 1
  fi
  sleep 1
done

trap - ERR
echo "$name is ready on $network."
