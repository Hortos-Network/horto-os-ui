# Ops KPI (`horto-os-ui-kpi`)

GPUI desktop viewer for operators. View-only against the status API. Not the homeowner product shell (that is `horto-os-ui-desktop`).

```bash
make kpi
make kpi HORTO_BOX_URL=http://192.168.1.10:8787
# HORTO_API_TOKEN when the API requires it
```

Shows a grid of numeric KPI tiles derived from `/health` and `/v1/status` (API health, containers up/total, services up/total, setup progress, doctor checks, DHCP leases, backup snaps / disk / initial). No container or service text dumps.
