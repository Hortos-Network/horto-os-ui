#[path = "connection.rs"]
mod component;
mod state;

pub use component::{boot_connection, ConnectionPanel};
pub use state::ConnectionState;
