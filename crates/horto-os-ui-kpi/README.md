# horto-os-ui-kpi

Binary `horto-os-ui-kpi`: GPUI control-room dashboard (live charts) against the status API and optional EVCC.

```bash
make kpi
make kpi HORTO_BOX_URL=http://192.168.1.10:8787
make kpi HORTO_KPI_PANELS=energy,fleet
```

Tool doc: [../../docs/tools/kpi.md](../../docs/tools/kpi.md).
