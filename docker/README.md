# horto-os-ui-status-api image (GHCR only)

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

## GHCR

Tip image (after CI push):

```bash
docker pull ghcr.io/hortos-network/horto-os-ui-status-api:dev
docker run --rm -p 8787:8787 \
  -e HORTO_API_TOKEN=secret \
  ghcr.io/hortos-network/horto-os-ui-status-api:dev
```

This image is a day-2 alternate host for the API. It does not replace remote SSH first-install (box tar.gz from GitHub Releases).
