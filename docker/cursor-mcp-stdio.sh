#!/usr/bin/env bash
# Cursor stdio helper: run horto-os-ui-mcp in Docker with SSH mounts.
# Usage: ./docker/cursor-mcp-stdio.sh
# Env (required for day-2 / remote): HORTO_BOX_URL, HORTO_API_TOKEN, HORTO_REMOTE_HOST
set -euo pipefail

IMAGE="${HORTO_MCP_IMAGE:-horto-os-ui-mcp:local}"
SSH_DIR="${HORTO_SSH_DIR:-${HOME}/.ssh}"

args=(
  run --rm -i
  -e MCP_HTTP=false
  -e HORTO_MCP_MODE=pc
  -e "HORTO_BOX_URL=${HORTO_BOX_URL:-http://host.docker.internal:8787}"
  -e "HORTO_API_TOKEN=${HORTO_API_TOKEN:-}"
  -e "HORTO_MCP_TOKEN=${HORTO_MCP_TOKEN:-${HORTO_API_TOKEN:-}}"
  -e "HORTO_REMOTE_HOST=${HORTO_REMOTE_HOST:-}"
  -e "HORTO_RELEASE_TAG=${HORTO_RELEASE_TAG:-dev-preview}"
  -e "HORTO_INSTALL_SSH_KEY=${HORTO_INSTALL_SSH_KEY:-}"
  -v "${SSH_DIR}:/home/nonroot/.ssh:ro"
)

if [[ -n "${SSH_AUTH_SOCK:-}" && -S "${SSH_AUTH_SOCK}" ]]; then
  args+=(-v "${SSH_AUTH_SOCK}:/ssh-agent" -e SSH_AUTH_SOCK=/ssh-agent)
fi

if [[ -n "${HORTO_BIN_DIR:-}" ]]; then
  args+=(-e "HORTO_BIN_DIR=/horto-bins" -v "${HORTO_BIN_DIR}:/horto-bins:ro")
fi

# Extra host so container can reach a status-api on the PC / LAN gateway.
if [[ "$(uname -s)" == "Linux" ]]; then
  args+=(--add-host=host.docker.internal:host-gateway)
fi

exec docker "${args[@]}" "${IMAGE}"
