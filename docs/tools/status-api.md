# Status API (`horto-os-ui-status-api`)

Box-local HTTP for PC clients. Read-mostly; no privileged install.

## Endpoints

| Method | Path | Auth |
| ------ | ---- | ---- |
| GET | `/health` | Always open |
| GET | `/v1/status` | Bearer when `HORTO_API_TOKEN` is set |

```bash
make api
# API_BIND=localhost:8787 by default via Make
curl -s http://localhost:8787/health
```

Env: `HORTO_API_BIND`, `HORTO_API_TOKEN`.
