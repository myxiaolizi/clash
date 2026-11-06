//! mihomo 内核管理：进程生命周期、API 客户端、端口分配、下载安装。

pub mod api;
pub mod fetch_core;
pub mod ports;
pub mod supervisor;

pub use api::{ApiFuture, HttpKernelApi, SelectorApi, SELECTOR_KIND};
pub use supervisor::{CoreLaunchSpec, KernelSupervisor};
