//! 持久化层（契约 C.3 / C.5）。

pub mod crypto;
pub mod rules_store;
pub mod settings_store;
pub mod switch_history;
pub mod traffic_store;

pub use settings_store::{SettingsStore, SubscriptionStore};
