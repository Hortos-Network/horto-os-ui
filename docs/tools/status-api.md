# Status API (`horto-os-ui-status-api`)

Box-local HTTP for PC clients. Read status over GET; the only mutate route is a
timestamped `/etc` backup. Setup, reinstall, disk image, and docker rebuild stay
on SSH / embedded CLI/TUI (not HTTP).

## Endpoints

| Method | Path             | Auth                                                                                                          |
| ------ | ---------------- | ------------------------------------------------------------------------------------------------------------- |
| GET    | `/health`        | Always open (still local-network peers only)                                                                  |
| GET    | `/v1/status`     | Bearer when `HORTO_API_TOKEN` is set                                                                          |
| POST   | `/v1/backup/etc` | **Always** requires bearer; disabled (503) if token unset. Also requires header `X-Horto-Confirm: backup-etc` |

```bash
make api
# API_BIND=0.0.0.0:8787 by default via Make (all interfaces)
curl -s http://localhost:8787/health
curl -s http://$(hostname):8787/health

# Mutate (token required):
curl -sS -X POST http://localhost:8787/v1/backup/etc \
  -H "Authorization: Bearer $HORTO_API_TOKEN" \
  -H "X-Horto-Confirm: backup-etc"
```

Env: `HORTO_API_BIND` (default all interfaces), `HORTO_API_TOKEN`.

Remote or embedded **full** apply writes `/etc/horto-os-ui/api.env` (mode 0600) with a random token and
wires it into the systemd unit via `EnvironmentFile=`. Paste that value into Desktop
Connection. Dev `make api` without a token still allows GET; POST mutate stays off.

Bind `0.0.0.0` means listen on every local NIC (so `deb` / LAN IP work). It does not by itself publish the port on the public Internet (that needs a public address, port forward, or open WAN firewall). As defense in depth, the API rejects peer IPs that are not loopback, RFC1918, IPv6 ULA, or link-local (`403`). Keep a host firewall anyway.

Service links in `/v1/status` come from embedded `assets/config/service_links.env`, overridden by `/srv/active_setup/service_links.env` when present (`SCHEME`, `HOST`, `LINKS=Name:port,...`). Empty `HOST` uses the box hostname (else `localhost`). After a successful loopback fetch, the web UI switches the Status API URL to the box hostname when that host also answers; otherwise it keeps loopback and shows an explicit error if a later fetch fails.

No Trunk proxy is required: the browser talks to the status API on `:8787`. CORS allows localhost / Tauri UI origins for GET and POST. A `Failed to fetch` to the box hostname with a loopback-only bind is a listen-address problem, not CORS.

AI tools that need setup / doctor / docker use MCP ([mcp.md](mcp.md)), not new status-api routes.
