# Ops KPI (`horto-os-ui-kpi`)

GPUI **KPI board**: a fixed 3x3 grid of live charts against the status API, plus optional EVCC power.

```bash
make kpi
make kpi HORTO_STATUS_API_URL=http://192.168.1.10:8787
make kpi HORTO_EVCC_URL=http://192.168.1.10:7070
# HORTO_API_TOKEN when the API requires it
# HORTO_KPI_POLL_SECS=2  HORTO_KPI_HISTORY=60
```

Tiles (equal rectangles):

|            |               |             |
| ---------- | ------------- | ----------- |
| PV (W)     | Grid (W)      | Home (W)    |
| Charge (W) | Containers up | Services up |
| Setup %    | Doctor %      | DHCP leases |

Energy tiles need a real EVCC `/api/state`. If the linked EVCC port is only a static proxy, set `HORTO_EVCC_URL` to the actual EVCC base, or the energy cells show the error and stay empty until power samples arrive.

**Demo mode (default):** synthetic animated series so charts look alive without EVCC. Turn off with `--demo false` or `HORTO_KPI_DEMO=0`.
