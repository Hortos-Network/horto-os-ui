//! High-level ops composed from kits and steps (backup, doctor, leases, runner, status).

pub mod backup;
pub mod catalog;
pub mod doctor;
/// DHCP lease parsing and export helpers.
pub mod leases;
/// Run Full/Minimal pipelines and individual steps with resume bookkeeping.
pub mod runner;
pub mod status;
