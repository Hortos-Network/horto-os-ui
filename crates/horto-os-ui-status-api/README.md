# horto-os-ui-status-api

Binary `horto-os-ui-status-api`: box-local HTTP status and a single day-2 mutate
route (timestamped `/etc` backup). Setup and reinstall are not HTTP.

```bash
make api
# GET /health  GET /v1/status  POST /v1/backup/etc
```

Tool doc: [../../docs/tools/status-api.md](../../docs/tools/status-api.md).
