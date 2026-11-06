//! Clash Zhengl 库入口。

pub mod app;
pub mod commands;
pub mod daemon;
pub mod domain;
pub mod kernel;
pub mod scheduler;
pub mod speedtest;
pub mod store;
pub mod subscription;
pub mod system;
pub mod util;

/// 应用版本
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
