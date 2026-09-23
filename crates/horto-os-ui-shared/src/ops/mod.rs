//! High-level ops composed from kits and steps (backup, doctor, leases, runner, status).

pub mod backup;
pub mod catalog;
pub mod doctor;
/// Local CPU / disk / OS / apt sensors for status overview.
pub mod host_metrics;
/// DHCP lease parsing and export helpers.
pub mod leases;
/// Run Full/Minimal pipelines and individual steps with resume bookkeeping.
pub mod runner;
/// Full service link catalog (name + default port).
pub mod service_catalog;
pub mod status;
