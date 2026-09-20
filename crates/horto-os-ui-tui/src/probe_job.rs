//! Background remote surface probe for the TUI event loop.

use horto_os_ui_shared::{
    probe_surfaces, remote_box_snapshot, RemoteBoxSnapshot, RemoteOptions, SurfaceProbeReport,
    SystemProcessRunner,
};

/// Successful remote probe payload (surfaces + optional setup snapshot).
pub struct RemoteProbeOk {
    /// SSH/CLI/API/MCP surface report.
    pub report: SurfaceProbeReport,
    /// Setup/doctor snapshot when the box CLI is current; `Err` if that fetch failed.
    pub snapshot: Option<Result<RemoteBoxSnapshot, String>>,
}

/// Result of a background remote probe job.
pub struct RemoteProbeOutcome {
    /// OpenSSH Host that was probed.
    pub host: String,
    /// Surfaces + snapshot, or a probe-level error string.
    pub result: Result<RemoteProbeOk, String>,
}

/// Run `probe_surfaces` then optional `remote_box_snapshot` (same work as the old blocking refresh).
pub fn run_remote_probe(opts: RemoteOptions, full: bool) -> RemoteProbeOutcome {
    let host = opts.host.clone();
    match probe_surfaces(&SystemProcessRunner, &opts, false) {
        Ok(report) => {
            let snapshot = if report.cli.current {
                Some(
                    remote_box_snapshot(&SystemProcessRunner, opts, full)
                        .map_err(|e| e.to_string()),
                )
            } else {
                None
            };
            RemoteProbeOutcome {
                host,
                result: Ok(RemoteProbeOk { report, snapshot }),
            }
        }
        Err(e) => RemoteProbeOutcome {
            host,
            result: Err(e.to_string()),
        },
    }
}
