//! 纯逻辑层：数据模型、错误码、策略纯函数。**本模块不得有 IO**。

pub mod errors;
pub mod model;
pub mod policy;

pub use errors::{AppError, AppResult, ErrorCode};
