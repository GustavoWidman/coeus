pub mod client;
pub mod server;

pub use client::{ClientConfig, client_config};
pub use server::{ServerConfig, server_config};

const fn default_config_path() -> u16 {
    45729
}
