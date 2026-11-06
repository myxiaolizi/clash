//! 调度器：单一 tick 循环、周期更新、退避、抖动。

pub mod backoff;
pub mod clock;
pub mod due;
pub mod runner;

pub use runner::{Scheduler, SchedulerEvent};
