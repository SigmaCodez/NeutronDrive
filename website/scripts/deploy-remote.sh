#!/usr/bin/env bash
set -Eeuo pipefail
cd "$(dirname "$0")/.."
exec 9>/opt/neutrondrive/deploy.lock
flock -n 9 || { echo 'Another deployment is active.' >&2; exit 1; }
NETWORK=codezwerk-proxy_default
NAME=neutrondrive-web
PORT=31219
CONFIGURE_PROXY=false
[[ ${1:-} != --configure-proxy ]] || CONFIGURE_PROXY=true
TAG="neutrondrive-web:$(date -u +%Y%m%dT%H%M%SZ)"
PREVIOUS=$(docker inspect "$NAME" --format '{{.Config.Image}}' 2>/dev/null || true)
PREVIOUS_PORT=$(docker inspect "$NAME" --format '{{range $port, $_ := .HostConfig.PortBindings}}{{$port}}{{end}}' 2>/dev/null || true)
PREVIOUS_PORT=${PREVIOUS_PORT%/tcp}
docker network inspect "$NETWORK" >/dev/null
# Fail before build/replacement when the existing proxy credentials have expired.
if $CONFIGURE_PROXY; then python3 scripts/configure-proxy.py --check-auth; fi
# Refuse to replace another service already using the requested port.
if ss -H -ltn "sport = :$PORT" | grep . >/dev/null; then
  BOUND=$(docker inspect "$NAME" --format '{{range .HostConfig.PortBindings}}{{range .}}{{.HostPort}}{{end}}{{end}}' 2>/dev/null || true)
  [[ "$BOUND" == "$PORT" ]] || { echo "Port $PORT is already in use." >&2; exit 1; }
fi
python3 -m unittest discover -s tests -p 'test_*.py'
docker build --pull -t "$TAG" .
bash scripts/install-content-sync.sh
run_container() {
  local ports=()
  if [[ "$1" == "$NAME" ]]; then ports=(-p "${PORT}:${3:-31219}"); fi
  docker run -d "${ports[@]}" --name "$1" --network "$NETWORK" --restart unless-stopped \
    --read-only --tmpfs /tmp:rw,noexec,nosuid,size=32m \
    --mount type=bind,src=/var/lib/neutrondrive-content,dst=/usr/share/nginx/html/live,readonly \
    --cap-drop ALL --security-opt no-new-privileges:true \
    --memory 128m --cpus 1 --pids-limit 100 \
    --log-opt max-size=5m --log-opt max-file=2 "$2"
}
healthy() {
  for ((i=0;i<30;i++)); do
    if [[ $(docker inspect "$1" --format '{{.State.Health.Status}}') == healthy ]]; then return 0; fi
    sleep 2
  done
  return 1
}
# Test the built image before replacing the serving container.
CANDIDATE="neutrondrive-check-$$"
trap 'docker rm -f "$CANDIDATE" >/dev/null 2>&1 || true' EXIT
run_container "$CANDIDATE" "$TAG"
healthy "$CANDIDATE" || { echo 'Candidate failed its health check.' >&2; exit 1; }
docker rm -f "$CANDIDATE" >/dev/null
rollback() {
  local status=$?
  trap - ERR
  echo 'Deployment failed. Restoring previous application image if available.' >&2
  docker rm -f "$NAME" >/dev/null 2>&1 || true
  if [[ -n "$PREVIOUS" ]]; then
    run_container "$NAME" "$PREVIOUS" "${PREVIOUS_PORT:-31219}"
    healthy "$NAME" || true
    if $CONFIGURE_PROXY; then python3 scripts/configure-proxy.py || true; fi
  fi
  exit "$status"
}
trap rollback ERR
if [[ -n "$PREVIOUS" ]]; then docker rm -f "$NAME" >/dev/null; fi
run_container "$NAME" "$TAG"
healthy "$NAME"
curl --fail --silent --show-error "http://127.0.0.1:$PORT/health"
curl --fail --silent --show-error "http://127.0.0.1:$PORT/" | grep 'NeutronDrive' >/dev/null
if $CONFIGURE_PROXY; then
python3 scripts/configure-proxy.py
docker exec codezwerk_proxy nginx -t
curl --fail --silent --show-error --retry 5 --retry-delay 2 --retry-all-errors https://neutrondrive.sigmacodez.dev/health
curl --fail --silent --show-error https://neutrondrive.sigmacodez.dev/ | grep 'NeutronDrive' >/dev/null
fi
printf '%s\n' "$TAG" > /opt/neutrondrive/current-image
printf '%s\n' "$PREVIOUS" > /opt/neutrondrive/previous-image
trap - ERR
printf 'Deployed %s on host port %s. Proxy upstream: http://neutrondrive-web:%s\n' "$TAG" "$PORT" "$PORT"
