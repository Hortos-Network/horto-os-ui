# CLI (`horto-os-ui`)

On-box (or laptop dry-run) command line over the shared engine.

## Commands

| Group | Examples |
| ----- | -------- |
| Setup | `setup status\|run\|step` with `--full` / `--minimal` |
| Doctor | `doctor` |
| Docker | `docker status\|init\|rebuild` |
| Net | `net leases`, `net export-leases` |
| Backup | `backup etc\|list\|disk-status\|disk\|shrink\|status` |

Global: `--dry-run`, `--skip-piper`. Apply needs root.

```bash
make status
make cli ARGS='setup step s1'
make doctor
```

Full operator recipes: [../README.md](../README.md) (this docs hub).
