//! 持久化层（契约 C.3 / C.5）。

pub mod crypto;
pub mod settings_store;
pub mod switch_history;

pub use settings_store::{SettingsStore, SubscriptionStore};
