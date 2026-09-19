# Ops KPI (`horto-os-ui-kpi`)

GPUI **control-room dashboard** for operators: live charts against the status API and optional EVCC power. Not the homeowner product shell (`horto-os-ui-desktop`).

```bash
make kpi
make kpi HORTO_BOX_URL=http://192.168.1.10:8787
make kpi HORTO_KPI_PANELS=energy,fleet,readiness,network
# HORTO_API_TOKEN when the API requires it
# HORTO_KPI_POLL_SECS=2  HORTO_KPI_HISTORY=60
```

Panels (toggle via `HORTO_KPI_PANELS`):

- **Energy** - PV / grid / home / charge watts from EVCC `/api/state` (needs an EVCC service link)
- **Fleet** - containers up and services up over time
- **Readiness** - setup / doctor / container / service percentage bars
- **Network** - DHCP lease count trend

Charts update on a poll timer. No text dumps of container names or URLs.
