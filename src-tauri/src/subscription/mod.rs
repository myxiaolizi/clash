//! 订阅管理编排层（P2）：CRUD、拉取、落盘、配置重建。

pub mod fetch;
pub mod generator;
pub mod manager;
pub mod parse;
pub mod userinfo;

pub use generator::{generate, GenerateInput, Generated, MAIN_GROUP_DEFAULT};
pub use manager::SubscriptionManager;
pub use parse::{parse_subscription, ParsedSubscription};
