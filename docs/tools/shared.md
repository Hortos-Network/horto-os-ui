# Engine (`horto-os-ui-shared`)

Library crate. Owns step registry, kits, resume, doctor, backup probes, and status DTOs.

## Public entry points

| Area | Symbols |
| ---- | ------- |
| Context | `HostContext`, `ApplyMode`, `require_root_for_apply` |
| Pipeline | `SetupKind`, `pipeline`, `lookup`, `setup_run`, `setup_step` |
| Status | `box_status`, `setup_status`, `doctor`, `backup_status` |
| Docker | `list_containers`, `docker_rebuild`, `docker_available` |
| Net | `read_leases`, `export_dhcp_leases` |
| Backup | `backup_etc_*`, `probe_disk_backup`, `backup_disk`, … |

## Tests

Most coverage should land here: dry-run pipelines against tempfile `HostPaths`, kit unit tests, lease parsers, resume round-trips.

```bash
cargo test -p horto-os-ui-shared
```
