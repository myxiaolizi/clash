//! 测速引擎与自动切换。

pub mod engine;
pub mod rank;

pub use engine::{ProbeConfig, SpeedtestEngine, SpeedtestEvent, SwitchEvent};
pub use rank::{build_view, RankOutcome, RankRow};
