# horto-os-ui-status-api image

Build and run the box-local status API in a distroless nonroot container.

## Local

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

## GitHub Release artefact (works without GHCR)

Tip Pre-release (`dev-preview`) and stable `vX.Y.Z` Releases attach:

`horto-os-ui-status-api-{V}-amd64.docker.tar.gz` (+ `.sha256`)

Load and run:

```bash
gunzip -c horto-os-ui-status-api-0.1.0-amd64.docker.tar.gz | docker load
docker run --rm -p 8787:8787 \
  -e HORTO_API_TOKEN=secret \
  horto-os-ui-status-api:0.1.0
```

## GHCR (when org package create is enabled)

**Tip** (`:dev`):

```bash
docker pull ghcr.io/hortos-network/horto-os-ui-status-api:dev
```

**Stable:**

```bash
docker pull ghcr.io/hortos-network/horto-os-ui-status-api:0.1.0
docker pull ghcr.io/hortos-network/horto-os-ui-status-api:latest
```

GHCR push workflows are disabled until org package management is enabled (issue #22). Use the Release `docker.tar.gz` until then.

This image is a day-2 alternate host for the API. It does not replace remote SSH first-install (box tar.gz from GitHub Releases).
