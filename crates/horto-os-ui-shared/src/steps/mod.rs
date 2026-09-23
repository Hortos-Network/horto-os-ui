//! Versioned setup steps (s1–s7, m1, d0–d3).

pub mod compose_util;
pub mod d0_docker_engine;
pub mod d1_docker;
pub mod d2_start_stacks;
pub mod d3_optional_stacks;
pub mod m1_minimal;
pub mod s1_packages;
pub mod s2_env;
pub mod s3_backup;
pub mod s4_stage;
pub mod s5_apply;
pub mod s6_validate;
pub mod s7_activate;
