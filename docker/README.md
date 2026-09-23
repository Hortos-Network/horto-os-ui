# horto-os-ui Docker images

## status-api

Build and run the box-local status API in a distroless nonroot container.

### Local

```bash
make docker-build
make docker-run
# http://127.0.0.1:8787/health
```

Pass a mutate token when needed:

```bash
docker run --rm -p 8787:8787 \
  -e HORTO_API_TOKEN=secret \
  horto-os-ui-status-api:local
```

### GitHub Release artefact

Tip Pre-release (`dev-preview`) and stable `vX.Y.Z` Releases attach:

`horto-os-ui-status-api-{V}-amd64.docker.tar.gz` (+ `.sha256`)

Load and run:

```bash
gunzip -c horto-os-ui-status-api-0.1.0-amd64.docker.tar.gz | docker load
docker run --rm -p 8787:8787 \
  -e HORTO_API_TOKEN=secret \
  horto-os-ui-status-api:0.1.0
```

This image is an alternate host for the status API. It does not replace remote SSH first install (box tar.gz from GitHub Releases).

## MCP

Cursor on a PC uses stdio via Docker (OpenSSH client included for remote tools):

```bash
make docker-build-mcp
export HORTO_STATUS_API_URL=http://192.168.0.242:8787
export HORTO_API_TOKEN=…
export HORTO_REMOTE_HOST=horto
./docker/cursor-mcp-stdio.sh
```

### GitHub Release artefact

Tip Pre-release (`dev-preview`) and stable `vX.Y.Z` Releases attach:

`horto-os-ui-mcp-{V}-amd64.docker.tar.gz` (+ `.sha256`)

```bash
gunzip -c horto-os-ui-mcp-0.1.0-amd64.docker.tar.gz | docker load
docker run --rm -i -e MCP_HTTP=false -e HORTO_MCP_MODE=pc \
  -e HORTO_STATUS_API_URL -e HORTO_API_TOKEN -e HORTO_REMOTE_HOST \
  -v "$HOME/.ssh:/home/nonroot/.ssh:ro" \
  horto-os-ui-mcp:0.1.0
```

Box smoke (status-api + MCP HTTP on LAN):

```bash
export HORTO_API_TOKEN=secret
docker compose -f docker/compose.box-mcp.yaml up -d --build
```

See [docs/tools/mcp.md](../docs/tools/mcp.md).
