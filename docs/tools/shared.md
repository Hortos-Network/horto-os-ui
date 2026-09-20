# Engine (`horto-os-ui-shared`)

Library crate. Owns step registry, kits, resume, doctor, backup probes, status DTOs, and the **remote OpenSSH runner**.

## Public entry points

| Area | Symbols |
| ---- | ------- |
| Context | `HostContext`, `ApplyMode`, `require_root_for_apply` |
| Pipeline | `SetupKind`, `pipeline`, `lookup`, `setup_run`, `setup_step` |
| Status | `box_status`, `setup_status`, `doctor`, `backup_status` |
| Docker | `list_containers`, `docker_rebuild`, `docker_available` |
| Net | `read_leases`, `export_dhcp_leases` |
| Backup | `backup_etc_*`, `probe_disk_backup`, `backup_disk`, … |
| Remote | `remote_run_cli` / `remote_setup_run` → `RemoteRunOutcome`, `offer_save_api_token`, `RemoteOptions`, `SystemProcessRunner` |

Remote modes and auth: [modes.md](modes.md).

## Tests

Most coverage should land here: dry-run pipelines against tempfile `HostPaths`, kit unit tests, lease parsers, resume round-trips, remote runner unit tests (scripted OpenSSH).

```bash
cargo test -p horto-os-ui-shared
make test-remote-docker   # live Docker SSH fixture (ignored by default)
```
