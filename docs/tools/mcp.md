# MCP (`horto-os-ui-mcp`)

One AI tool surface for Horto: day-2 status/backup via the status API, and
privileged setup / doctor / docker / backup listing via OpenSSH (PC) or the
in-process shared engine (box). Same tool catalog on both hosts.

Status-api stays the thin day-2 HTTP contract. MCP is the adapter that calls
the API and/or SSH / embedded ops. Install and reinstall are **not** added to
the status API.

## Transports

Both hosts support **stdio** and **HTTP**. The same binary switches with
`MCP_HTTP=false` (default) → stdio, or `MCP_HTTP=true` / `--http` → Streamable HTTP
on `:8790`. Surface probes report Docker image, host binary, HTTP reachability,
and unit/process on **PC and box**.

| Host | Common client use     | Transport details                                       |
| ---- | --------------------- | ------------------------------------------------------- |
| PC   | Cursor                | stdio via `docker run -i … horto-os-ui-mcp` (or binary) |
| PC   | Local HTTP (loopback) | Streamable HTTP `http://127.0.0.1:8790/mcp`             |
| Box  | On-box model          | Streamable HTTP `http://<box-lan>:8790/mcp`             |
| Box  | Cursor from PC        | `mcp-remote` to the LAN URL + bearer (or box stdio)     |

Runtime on either host may be the Docker image (`HORTO_MCP_IMAGE`, default
`horto-os-ui-mcp:local`) and/or the `horto-os-ui-mcp` binary.

## Mode

| Env              | Value | Day-2                                | Privileged tools       |
| ---------------- | ----- | ------------------------------------ | ---------------------- |
| `HORTO_MCP_MODE` | `pc`  | HTTP client → `HORTO_STATUS_API_URL` | OpenSSH remote runner  |
| `HORTO_MCP_MODE` | `box` | HTTP client → loopback API           | Embedded shared engine |

## Security

Same class as the status API:

- HTTP mode binds `0.0.0.0:8790` on the box by default, but **rejects non-LAN peers**
  (loopback / RFC1918 / IPv6 ULA / link-local).
- Every MCP HTTP request needs `Authorization: Bearer <token>`
  (`HORTO_MCP_TOKEN`, or `HORTO_API_TOKEN` when MCP token is unset).
- PC Docker HTTP (if used): bind `127.0.0.1` only; do not publish MCP on the WAN.
- Destructive tools require a confirm string: `backup-etc`, `docker-rebuild`.
- Day-1 on PC: OpenSSH only (password / askpass / agent). No second password UI.
- Host firewall stays required; never expose MCP or the status API on the public Internet.

## Tools

**Day-2 (always via status-api HTTP):**

| Tool         | Notes                                     |
| ------------ | ----------------------------------------- |
| `health`     | `GET /health`                             |
| `get_status` | `GET /v1/status` (bearer when configured) |
| `backup_etc` | confirm must be `backup-etc`              |

**Privileged (PC = SSH; box = embedded):**

| Tool                 | Notes                                |
| -------------------- | ------------------------------------ |
| `setup_status`       | `apply` (default false) / `full`     |
| `setup_run`          | `apply` default false; `skip_piper`  |
| `setup_step`         | step id (`s1`, `m1`, `d1`, …)        |
| `doctor`             | readiness report                     |
| `docker_status`      | container list                       |
| `docker_rebuild`     | confirm must be `docker-rebuild`     |
| `backup_list`        | timestamped `/etc` backups           |
| `backup_disk_status` | disk backup probe                    |
| `remote_probe`       | PC only; box arch via SSH            |

## Cursor on a PC (stdio + Docker)

```bash
make docker-build-mcp
export HORTO_STATUS_API_URL=http://192.168.0.242:8787
export HORTO_API_TOKEN=…          # from /etc/horto-os-ui/api.env on the box
export HORTO_REMOTE_HOST=horto    # OpenSSH Host alias or user@host
./docker/cursor-mcp-stdio.sh
```

Cursor `mcp.json` example:

```json
{
  "mcpServers": {
    "horto": {
      "command": "/path/to/horto-os-ui/docker/cursor-mcp-stdio.sh"
    }
  }
}
```

Or inline Docker:

```json
{
  "mcpServers": {
    "horto": {
      "command": "docker",
      "args": [
        "run",
        "--rm",
        "-i",
        "--add-host=host.docker.internal:host-gateway",
        "-e",
        "MCP_HTTP=false",
        "-e",
        "HORTO_MCP_MODE=pc",
        "-e",
        "HORTO_STATUS_API_URL",
        "-e",
        "HORTO_API_TOKEN",
        "-e",
        "HORTO_REMOTE_HOST",
        "-e",
        "HORTO_RELEASE_TAG=dev-preview",
        "-v",
        "${HOME}/.ssh:/home/nonroot/.ssh:ro",
        "horto-os-ui-mcp:local"
      ]
    }
  }
}
```

Mount `SSH_AUTH_SOCK` when using an agent. Tip Release assets: `HORTO_RELEASE_TAG=dev-preview`.

Release attaches `horto-os-ui-mcp-{V}-amd64.docker.tar.gz` (same GHCR pause as status-api until issue #22). Local: `make docker-build-mcp`.

## Box (HTTP for on-box model)

Install the binary next to the status API, then enable the unit:

```bash
sudo install -m 755 target/release/horto-os-ui-mcp /usr/local/bin/
sudo cp crates/horto-os-ui-mcp/packaging/horto-os-ui-mcp.service /etc/systemd/system/
# Optional: echo HORTO_MCP_TOKEN=… | sudo tee /etc/horto-os-ui/mcp.env && sudo chmod 600 …
sudo systemctl daemon-reload
sudo systemctl enable --now horto-os-ui-mcp.service
```

The unit reads `/etc/horto-os-ui/api.env` (and optional `mcp.env`), listens on
`0.0.0.0:8790`, uses `HORTO_MCP_MODE=box`, and points day-2 at loopback status-api.

Compose smoke (API + MCP containers):

```bash
export HORTO_API_TOKEN=secret
docker compose -f docker/compose.box-mcp.yaml up -d --build
```

Client: `http://<box-lan>:8790/mcp` with bearer token.

## Env reference

| Variable                | Role                                          |
| ----------------------- | --------------------------------------------- |
| `HORTO_MCP_MODE`        | `pc` (default) or `box`                       |
| `HORTO_STATUS_API_URL`  | Status API base URL                           |
| `HORTO_API_TOKEN`       | Bearer for status-api day-2                   |
| `HORTO_MCP_TOKEN`       | Bearer for MCP HTTP (falls back to API token) |
| `HORTO_MCP_ADDR`        | HTTP bind (`--listen`)                        |
| `MCP_HTTP`              | `true` → Streamable HTTP                      |
| `HORTO_REMOTE_HOST`     | PC mode: OpenSSH Host / `user@host`           |
| `HORTO_RELEASE_TAG`     | Remote binary source tag (e.g. `dev-preview`) |
| `HORTO_BIN_DIR`         | Local bins instead of GitHub Release download |
| `HORTO_INSTALL_SSH_KEY` | Opt-in `ssh-copy-id` (`1`/`true`)             |

See also [modes.md](modes.md) and [status-api.md](status-api.md).
