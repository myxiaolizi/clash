# 06 · 测速引擎（Speedtest Engine）

> **适用阶段**：P4（依赖 P1 内核 API、P2/P3 合并后的 runtime config）
> **必读前置**：[01-master-prompt.md](01-master-prompt.md)、[appendices/C-conventions.md](appendices/C-conventions.md)（C.5 `SpeedtestRun`/`ProbeResult`、C.6 `speedtest_*`、C.7 `speedtest:progress`、C.8 错误码、C.10 定时精度）
> **本文件覆盖**：`src-tauri/src/speedtest/{mod,engine,probe,rank}.rs`；`selector.rs` 见 [07-auto-switch-policy.md](07-auto-switch-policy.md)。

---

## 6.1 目标与硬约束

| # | 目标 | 约束来源 | 落地 |
| --- | --- | --- | --- |
| T1 | 并发上限默认 **8**，可配 **1..=16** | C.5 `speedtest_concurrency` | `Semaphore::new(conc)` + 入参 clamp |
| T2 | 单探测超时默认 **3000ms**，范围 1000..=10000 | C.5 `speedtest_timeout_ms` | 同时下发 `<timeout>` 给 mihomo + 本地 `tokio::time::timeout` 双保险 |
| T3 | 总时长上限 ≤ `ceil(N/并发) × 超时 × 重试系数` | 本任务验收 | `budget_ms` 预算 + 全局 `CancellationToken` |
| T4 | 可取消，取消后不留悬挂任务、不留 socket | C.13「async 任务必须有取消路径」 | `CancellationToken` + 丢弃 `FuturesUnordered` |
| T5 | 失败重试 **1 次** | 本任务验收 | 每个 target 最多 2 次尝试，第二次前 `80ms × attempt` 微退避 |
| T6 | 进度事件 **≤4 条/秒**，每 250ms 批量合并 | C.7 | 独立 flusher，仅在测试运行期存在 |
| T7 | 排除 `DIRECT`/`REJECT` 与「信息节点」 | C.5 `exclude_keywords` | `rank.rs` 纯函数 + 引擎前置过滤 |
| T8 | 与 mihomo 自身 `url-test` 组不打架 | 已知事实 / 本任务验收 | 本应用测速后**显式 PUT 覆盖**，并把相关组改为 `select`（见 §6.9） |
| T9 | 结果结构与 C.5 完全一致 | C.5 | 直接复用 `SpeedtestRun`/`ProbeResult`，不新增 DTO |
| T10 | 排序/排除可单测 | C.2 | `rank.rs` 零 IO，吃 `&[ProbeResult]` 出 `Vec<Candidate>` |

**非目标（明确不做）**：不做带宽测速（下载大文件）、不做 UDP/TUN 连通性判定、不做丢包率统计。
本产品只测「握手/首包延迟」，单位毫秒（C.10）。

---

## 6.2 mihomo REST 事实表（本章所有代码据此）

| 用途 | 请求 | 成功响应 | 失败响应 |
| --- | --- | --- | --- |
| 单节点延迟 | `GET /proxies/{name}/delay?url=<url>&timeout=<ms>&expected=204` | `{"delay":123}`（HTTP 200） | 4xx/5xx；超时返回 **504** + `{"message":"..."}` |
| 组内全测（**本应用不用**） | `GET /group/{group}/delay?url=&timeout=` | `{"节点名": ms, ...}` | 同上 |
| 切换选择 | `PUT /proxies/{group}` body `{"name":"节点名"}` | 204 无 body | 400/404 |
| 节点清单 | `GET /proxies` | `{"proxies":{name:{name,type,udp,alive,history:[{time,delay}],now,all,testUrl,...}}}` | — |

**三条必须记住的坑**

1. `/group/{group}/delay` 会**清空该自动策略组的固定选择**（`fixed` 被重置），
   导致用户刚手选的节点被内核自己改掉，且与 §6.9 的显式覆盖叠加产生双引擎抖动。→ **禁用该接口**。
2. `expected=204` 缺省时 mihomo 只校验「有响应」，机场劫持的 200 落地页也会被算成成功。
   必须带 `expected=204`，否则延迟数据不可信。
3. `timeout` 是**内核侧**超时，内核可能比我们更早返回 504；因此本地 `tokio::time::timeout`
   要设为 `timeout_ms + 500ms` 的富余，避免把内核正常返回误判为本地超时。

---

## 6.3 运行数据流与状态机

```
   IPC speedtest_start{trigger}
            │
            ▼
   ┌──────────────────┐  已有 run 在跑
   │  start(runId)    │──────────────▶ 直接返回同一个 runId（幂等，不报 E_BUSY）
   └────────┬─────────┘
            │ 新建
            ▼
   ┌──────────────────┐  GET /proxies 失败
   │  Inventory       │──────────────▶ E_CORE_NOT_FOUND / E_CORE_API_UNAUTH / E_NET_TIMEOUT
   └────────┬─────────┘
            │ 得节点清单
            ▼
   ┌──────────────────┐  全部被排除
   │  Filter (rank)   │──────────────▶ speedtest:done，total=0，best=None（不报 E_PARSE_EMPTY）
   └────────┬─────────┘
            ▼
   ┌──────────────────┐
   │  Probing         │◀── 每 250ms 发 speedtest:progress
   │  sem(8) 限流     │
   └───┬──────────┬───┘
       │ 全部完成  │ 全局预算耗尽 / speedtest_cancel
       ▼          ▼
   ┌──────────────────┐
   │  Aggregating     │  排序 + 排除（rank.rs），写入 best
   └────────┬─────────┘
            │
            ├──▶ speedtest:done { SpeedtestRun }
            │
            └──▶ 若 trigger != Manual 且 switch_policy.enabled
                    └─▶ selector::after_speedtest()（07 章）
                          ├─ ① update_failure_counters()   ← 必须先，否则阈值差一轮
                          └─ ② apply_best()                ← 返回 {primary, all}
```

> **与 07 章的衔接契约（不可改）**：测速引擎在 `trigger ≠ Manual` 时**直接调用**
> `selector::after_speedtest()`，不经调度器、不发任何额外 IPC、不等待 UI 确认。
> 调用顺序固定为「先累加失败计数 → 再判定切换」，反过来写 `failure_threshold` 永远差一轮。
> 切换失败**不影响**测速结果的有效性：只 `tracing::warn!` 记一条，`speedtest:done` 照常发出。
> 这段衔接的完整实现在 [07-auto-switch-policy.md](07-auto-switch-policy.md) §7.6。
>
> ⚠️ **签名冻结**：本章 `SpeedtestEngine::start` / `status` / `cancel` / `StartOutcome`，
> 以及 `after_speedtest` / `rank_candidates_with_priority` / `ExcludeRules::from_policy` /
> `CoreApi::proxy_delay` 的**完整签名以 [07 §7.16 跨章接口冻结表](07-auto-switch-policy.md)
> 为准**（当前冻结版本 `v1.4-freeze-1`）。本章正文里的签名若与冻结表不一致，**以冻结表为准**，
> 并回头修正本章——不得反向要求 07/08 章迁就。任何签名变更须同步 C.6.1 并升冻结版本号。

| 状态 | 前端可见 | 可取消 | 备注 |
| --- | --- | --- | --- |
| `Starting` | `speedtest_start` 已返回 runId | 是 | `GET /proxies` 阶段 |
| `Probing` | 进度条 0..100% | 是 | 主体阶段 |
| `Draining` | 进度 100% | 否 | 等 Semaphore 归还 + flush 末批 |
| `Done` | `speedtest:done` | 否 | 终态 |
| `Cancelled` | `speedtest:done`，`finished < total` | 否 | 终态，**不**报错 |

> 取消语义：`speedtest_cancel` 后仍然发 `speedtest:done`（`finished_at` 已填，`finished < total`），
> 前端据 `finished < total` 显示「已取消」。C.7 没有 `speedtest:cancelled` 事件，**不新增**。

### 6.3.1 三个 IPC 的精确行为（严格对齐 C.6）

| IPC 命令 | 入参 | 返回 | 本章实现要点 |
| --- | --- | --- | --- |
| `speedtest_start` | `{trigger?}` | `{runId}` | **幂等**：已在跑则返回当前 runId，不排队、不报 `E_BUSY`。`trigger` 缺省按 `Timer`（来自调度器）或 `Manual`（来自 UI 按钮）区分 |
| `speedtest_status` | `{runId}` | `SpeedtestRun` | 前端 1s 轮询兜底；`runId` 不存在返回 `E_UNSUPPORTED`（不新造错误码）。有事件通道时前端应优先用事件 |
| `speedtest_cancel` | `{runId}` | `()` | 只取消匹配 runId 的那一轮；不在跑则**幂等成功**，不报错 |

> **为什么 `speedtest_start` 不返回 `E_BUSY`**：用户连点两次「立即测速」是很常见的操作，
> 报错只会让用户困惑。返回同一个 `runId` 让前端继续显示同一条进度条，是更正确的语义。
> `E_BUSY` 留给**真正会互相破坏**的操作（见 07 章 `switch_apply_best` 的单飞锁）。

---

## 6.4 引擎实现（engine.rs）

```rust
// speedtest/mod.rs
pub mod engine;
pub mod probe;
pub mod rank;

pub use engine::{SpeedtestEngine, SpeedtestConfig, StartOutcome};
pub use probe::{ProbeTarget, ProbeOutcome};
pub use rank::{Candidate, ExcludeRules, rank_candidates};
```

```rust
// speedtest/engine.rs
use std::sync::Arc;
use std::time::Duration;

use futures::stream::FuturesUnordered;
use futures::StreamExt;
use tokio::sync::{Mutex, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::domain::errors::AppError;
use crate::domain::model::{ProbeResult, RunTrigger, SpeedtestRun, SwitchPolicy};
use crate::kernel::api::CoreApi;
use super::probe::{self, ProbeTarget};
use super::rank::{self, ExcludeRules};

/// 并发与超时的合法区间（C.5 注释即契约）
pub const CONC_MIN: u8 = 1;
pub const CONC_MAX: u8 = 16;
pub const TIMEOUT_MIN_MS: u32 = 1_000;
pub const TIMEOUT_MAX_MS: u32 = 10_000;
/// 单次运行的总时长硬上限（保护低配机，避免一次测速占满 4 分钟）
pub const HARD_BUDGET_MS: u64 = 240_000;
/// 进度合并窗口：250ms → 上限 4 条/秒（C.7）
pub const PROGRESS_FLUSH_MS: u64 = 250;

#[derive(Debug, Clone)]
pub struct SpeedtestConfig {
    pub probe_url: String,
    pub timeout_ms: u32,
    pub concurrency: u8,
    pub retries: u8, // 默认 1（= 额外重试 1 次）
}

impl SpeedtestConfig {
    /// 从 AppSettings 收敛：任何越界值被 clamp 而不是报错（用户改坏配置也要能跑）
    pub fn from_settings(s: &crate::domain::model::AppSettings) -> Self {
        Self {
            probe_url: s.speedtest_probe_url.clone(),
            timeout_ms: s.speedtest_timeout_ms.clamp(TIMEOUT_MIN_MS, TIMEOUT_MAX_MS),
            concurrency: s.speedtest_concurrency.clamp(CONC_MIN, CONC_MAX),
            retries: 1,
        }
    }

    /// 总时长预算（毫秒）。纯函数，可单测。
    /// waves = ceil(total / concurrency)；每波最多 timeout；重试系数 (1 + retries)；
    /// 再加 2s 调度余量；最后与硬上限取小。
    pub fn budget_ms(&self, total: u32) -> u64 {
        let waves = (total as u64).div_ceil(self.concurrency.max(1) as u64);
        let per_wave = self.timeout_ms as u64 + 500; // 本地超时富余
        let raw = waves
            .saturating_mul(per_wave)
            .saturating_mul(1 + self.retries as u64)
            .saturating_add(2_000);
        raw.min(HARD_BUDGET_MS)
    }
}

#[derive(Debug)]
pub enum StartOutcome {
    /// 新起一轮
    Started(String),
    /// 已有 run 在跑，返回它（C.6：speedtest_start 幂等）
    AlreadyRunning(String),
}

/// 运行期共享状态：runId → 取消句柄 + 快照
pub struct SpeedtestEngine {
    cfg: SpeedtestConfig,
    inflight: Mutex<Option<Inflight>>,
}

struct Inflight {
    run_id: String,
    cancel: CancellationToken,
    started_at: i64,
    snapshot: Arc<Mutex<SpeedtestRun>>,
}

impl SpeedtestEngine {
    pub fn new(cfg: SpeedtestConfig) -> Self {
        Self { cfg, inflight: Mutex::new(None) }
    }

    pub async fn current_run_id(&self) -> Option<String> {
        self.inflight.lock().await.as_ref().map(|i| i.run_id.clone())
    }

    pub async fn status(&self, run_id: &str) -> Result<SpeedtestRun, AppError> {
        let guard = self.inflight.lock().await;
        match guard.as_ref() {
            Some(i) if i.run_id == run_id => Ok(i.snapshot.lock().await.clone()),
            _ => Err(AppError::new("E_UNSUPPORTED", "该测速任务不存在或已结束").retryable(false)),
        }
    }

    /// 取消：只作用于当前 run；不在跑则幂等成功（C.6 `speedtest_cancel{runId}`）
    pub async fn cancel(&self, run_id: &str) {
        if let Some(i) = self.inflight.lock().await.as_ref() {
            if i.run_id == run_id {
                i.cancel.cancel();
            }
        }
    }

    /// 主入口：幂等启动或返回在跑的 run。
    ///
    /// 入参 `policy` / `selector` 由 `AppState` 注入：测速结束时（trigger≠Manual）
    /// 直接回调 07 章的 `after_speedtest`，这样「测完 → 切」是一个原子的连续动作，
    /// 不需要调度器或前端再触发一次 IPC。
    #[allow(clippy::too_many_arguments)]
    pub async fn start(
        self: &Arc<Self>,
        api: Arc<CoreApi>,
        inventory: Vec<ProbeTarget>,
        rules: ExcludeRules,
        region_priority: Vec<String>,
        policy: SwitchPolicy,
        selector: Arc<crate::speedtest::Selector>,
        trigger: RunTrigger,
        events: crate::event::EventSink,
    ) -> Result<StartOutcome, AppError> {
        let mut guard = self.inflight.lock().await;
        if let Some(cur) = guard.as_ref() {
            // 幂等：不排队、不报 E_BUSY，直接复用
            return Ok(StartOutcome::AlreadyRunning(cur.run_id.clone()));
        }

        let run_id = crate::util::time::new_run_id();
        let now = crate::util::time::now_unix();
        let run = SpeedtestRun {
            id: run_id.clone(),
            started_at: now,
            finished_at: None,
            probe_url: self.cfg.probe_url.clone(),
            timeout_ms: self.cfg.timeout_ms,
            total: 0, // 过滤后才确定
            finished: 0,
            results: Vec::new(),
            best: None,
            trigger,
        };
        let snapshot = Arc::new(Mutex::new(run));
        let cancel = CancellationToken::new();
        *guard = Some(Inflight {
            run_id: run_id.clone(),
            cancel: cancel.clone(),
            started_at: now,
            snapshot: snapshot.clone(),
        });
        drop(guard); // 不持锁跑网络

        // 后台跑，立刻返回 runId（C.6 返回 {runId}，不阻塞前端）
        let this = Arc::clone(self);
        let run_id_for_task = run_id.clone();
        let shutdown = crate::app::shutdown_token();
        tokio::spawn(async move {
            // 应用退出时强制取消，避免悬挂探测阻塞进程退出（C.13）
            let run_cancel = cancel.clone();
            let res = tokio::select! {
                biased;
                _ = shutdown.cancelled() => {
                    cancel.cancel();
                    Ok(())
                }
                r = this.execute(
                    run_id_for_task,
                    api,
                    inventory,
                    rules,
                    region_priority,
                    policy,
                    selector,
                    trigger,
                    run_cancel,
                    snapshot.clone(),
                    events.clone(),
                ) => r,
            };
            if let Err(e) = res {
                tracing::error!("[speedtest] run aborted code={} msg={}", e.code, e.message);
                // 收尾：写入终态并广播，前端不会永久转圈
                let mut snap = snapshot.lock().await;
                if snap.finished_at.is_none() {
                    snap.finished_at = Some(crate::util::time::now_unix());
                }
                let done = snap.clone();
                drop(snap);
                events.emit("speedtest:done", &done);
            }
            // 无论成败都释放 inflight，允许下一轮
            *this.inflight.lock().await = None;
        });

        Ok(StartOutcome::Started(run_id))
    }
}
```

### 6.4.1 探测主循环（`execute`）

```rust
impl SpeedtestEngine {
    #[allow(clippy::too_many_arguments)]
    async fn execute(
        &self,
        run_id: String,
        api: Arc<CoreApi>,
        inventory: Vec<ProbeTarget>,
        rules: ExcludeRules,
        region_priority: Vec<String>,
        policy: SwitchPolicy,
        selector: Arc<crate::speedtest::Selector>,
        trigger: RunTrigger,
        cancel: CancellationToken,
        snapshot: Arc<Mutex<SpeedtestRun>>,
        events: crate::event::EventSink,
    ) -> Result<(), AppError> {
        // ① 前置过滤：排除 DIRECT/REJECT/信息节点/重复 key
        let targets = rank::filter_targets(inventory, &rules);
        let total = targets.len() as u32;
        {
            let mut snap = snapshot.lock().await;
            snap.total = total;
        }
        if total == 0 {
            tracing::info!("[speedtest] no target after exclusion run={}", run_id);
            self.finish(snapshot, events).await;
            return Ok(());
        }

        // ② 预算：纯函数算出总时长上限
        let budget = Duration::from_millis(self.cfg.budget_ms(total));
        let probe_cancel = cancel.child_token();
        // 进度定时器单独一个子 token：主循环结束即可停 flush，不必等 cancel
        let progress_cancel = CancellationToken::new();
        let engine_cfg = self.cfg.clone();

        // ③ 进度合并器（只在运行期存在的唯一额外定时器）
        let progress = ProgressCoalescer::new(
            events.clone(),
            run_id.clone(),
            total,
            Duration::from_millis(PROGRESS_FLUSH_MS),
        );
        // flush 循环与主循环并行；run 结束（含取消/超时）时随 cancel 退出
        let progress_task = {
            let p = Arc::new(progress);
            let p2 = Arc::clone(&p);
            let guard = progress_cancel.cancelled_owned();
            let h = tokio::spawn(async move {
                tokio::select! {
                    biased;
                    _ = guard => {}
                    _ = p2.run() => {}
                }
            });
            (p, h)
        };

        let sem = Arc::new(Semaphore::new(self.cfg.concurrency as usize));
        let mut tasks = FuturesUnordered::new();

        // ⚠️ 关键：permit 必须在**异步任务内部**获取，不能在 for 循环里 `await`。
        // 若在循环里 await acquire_owned，第 conc+1 次会永久阻塞——因为此时
        // FuturesUnordered 还没被 poll，前面的任务根本没跑，永远不会释放 permit。
        // 这是一个必踩的死锁，写在这里作为反例注释。
        for t in targets {
            let api = Arc::clone(&api);
            let cfg = engine_cfg.clone();
            let snap = snapshot.clone();
            let pc = progress.handle();
            let t_cancel = probe_cancel.clone();
            let sem2 = Arc::clone(&sem);
            tasks.push(async move {
                // 拿到 permit 才真正发起探测；`_permit` 持有到本 future 结束
                let _permit = match sem2.acquire_owned().await {
                    Ok(p) => p,
                    Err(_) => return, // Semaphore 已关闭（收尾阶段）
                };
                // 排队期间可能已被取消，不必再发请求
                if t_cancel.is_cancelled() {
                    return;
                }
                let out = probe::probe_with_retry(&api, &t, &cfg, cfg.retries, &t_cancel).await;
                if t_cancel.is_cancelled() && out.delay_ms.is_none() {
                    // 取消导致的中断不计入结果，避免污染统计
                    return;
                }
                let pr = out.into_probe_result(&t);
                // 逐条累加（锁粒度极小，只做 push + 计数）
                {
                    let mut s = snap.lock().await;
                    s.results.push(pr.clone());
                    s.finished += 1;
                }
                pc.push(pr).await;
            });
        }

        // ④ 事件循环：完成 / 取消 / 预算超时，三者竞速
        let deadline = tokio::time::sleep(budget);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    tracing::info!("[speedtest] cancelled finished_partially");
                    break;
                }
                _ = &mut deadline => {
                    tracing::warn!("[speedtest] budget exhausted budget_ms={}", self.cfg.budget_ms(total));
                    probe_cancel.cancel();
                    break;
                }
                Some(()) = tasks.next() => {}
                else => break,
            }
        }
        // 丢弃 FuturesUnordered 即取消所有未完成探测（futures 被 drop，reqwest future 随之 abort）
        drop(tasks);
        // 先停 flush 循环，再发末批：保证末批是最后一个 progress 事件，不会与 done 抢序
        progress_cancel.cancel();
        let _ = progress_task.1.await;
        progress_task.0.flush_and_stop().await;

        // ⑤ 聚合 + 排序（纯函数）。注意用 _with_priority：`prefer_region_priority` 是延迟并列时的
        //    tiebreak，漏传会导致「同延迟多节点」每轮随机选一个（真实抖动来源）。
        let results = { snapshot.lock().await.results.clone() };
        let ranked = rank::rank_candidates_with_priority(&results, &rules, &region_priority);
        let best = ranked.first().map(|c| c.node_key.clone());
        {
            let mut snap = snapshot.lock().await;
            snap.best = best.clone();
            snap.finished_at = Some(crate::util::time::now_unix());
        }
        tracing::info!(
            "[speedtest] done total={} ok={} best={:?} trigger={:?}",
            total,
            results.iter().filter(|r| r.delay_ms.is_some()).count(),
            best,
            trigger
        );
        // ⑥ 交给 07 章做切换判定（trigger=Manual 时由用户自己点「应用最优」，不自动切）
        if matches!(trigger, RunTrigger::Timer | RunTrigger::AfterUpdate) {
            let summary = crate::speedtest::selector::build_summary(
                &results,
                &rules,
                &region_priority,
            );
            let finished = { snapshot.lock().await.clone() };
            let now = crate::util::time::now_unix();
            // 失败不阻断：测速结果本身仍然有效，切换失败只记日志（详见 07 章）
            if let Err(e) = selector
                .after_speedtest(&api, &policy, &finished, &summary, events.clone(), now)
                .await
            {
                tracing::warn!("[speedtest] auto switch skipped code={} msg={}", e.code, e.message);
            }
        }
        self.finish(snapshot, events).await;
        Ok(())
    }

    async fn finish(&self, snapshot: Arc<Mutex<SpeedtestRun>>, events: crate::event::EventSink) {
        let mut snap = snapshot.lock().await;
        if snap.finished_at.is_none() {
            snap.finished_at = Some(crate::util::time::now_unix());
        }
        let done = snap.clone();
        drop(snap);
        events.emit("speedtest:done", &done);
    }
}
```

---

## 6.5 单探测（probe.rs）

```rust
// speedtest/probe.rs
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use crate::domain::model::{ProbeResult, ProxyKind};
use crate::kernel::api::CoreApi;
use super::engine::SpeedtestConfig;

#[derive(Debug, Clone)]
pub struct ProbeTarget {
    /// 与 C.5 `ProbeResult.node_key` 完全一致：运行时配置里 `proxies` 的 key
    /// （冲突时已由 04 章加 `<sub>/` 前缀）
    pub node_key: String,
    /// 归属的策略组（优先 SwitchScope 指定的组，其次第一个含该节点的 select 组）
    pub group: String,
    pub kind: ProxyKind,
    /// mihomo 上报的存活标记（仅用于日志对照，不参与排除——它可能是陈旧的）
    pub alive_hint: bool,
}

#[derive(Debug)]
pub struct ProbeOutcome {
    pub delay_ms: Option<u32>,
    pub error: Option<String>,
}

impl ProbeOutcome {
    pub fn into_probe_result(self, t: &ProbeTarget) -> ProbeResult {
        ProbeResult {
            node_key: t.node_key.clone(),
            group: t.group.clone(),
            kind: t.kind,
            delay_ms: self.delay_ms,
            error: self.error,
            tested_at: crate::util::time::now_unix(),
        }
    }
}

/// 单次探测（不重试）。返回的 error 是**已脱敏**的短字符串。
pub async fn probe_once(
    api: &CoreApi,
    t: &ProbeTarget,
    cfg: &SpeedtestConfig,
    cancel: &CancellationToken,
) -> ProbeOutcome {
    // 本地超时 = 内核超时 + 500ms 富余，避免抢在内核 504 之前误判
    let local = Duration::from_millis(cfg.timeout_ms as u64 + 500);
    let fut = api.proxy_delay(&t.node_key, &cfg.probe_url, cfg.timeout_ms, 204);

    let out = tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            return ProbeOutcome { delay_ms: None, error: Some("cancelled".into()) };
        }
        r = tokio::time::timeout(local, fut) => r,
    };

    match out {
        Err(_elapsed) => ProbeOutcome { delay_ms: None, error: Some("timeout_local".into()) },
        Ok(Ok(ms)) => ProbeOutcome { delay_ms: Some(ms), error: None },
        Ok(Err(e)) => ProbeOutcome { delay_ms: None, error: Some(map_probe_err(&e)) },
    }
}

/// 重试策略：只在**可重试**错误上重试 1 次；4xx（节点不存在/组不存在）不重试。
pub async fn probe_with_retry(
    api: &CoreApi,
    t: &ProbeTarget,
    cfg: &SpeedtestConfig,
    retries: u8,
    cancel: &CancellationToken,
) -> ProbeOutcome {
    let mut last = probe_once(api, t, cfg, cancel).await;
    for attempt in 1..=retries {
        if last.delay_ms.is_some() || !is_retryable(last.error.as_deref()) {
            break;
        }
        // 微退避 80ms × attempt，且可被取消打断
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            _ = tokio::time::sleep(Duration::from_millis(80 * attempt as u64)) => {}
        }
        last = probe_once(api, t, cfg, cancel).await;
    }
    last
}

/// 只有「超时 / 连接失败 / 5xx / 504」值得重试；
/// 404（节点不存在）、401（secret 错）、超时配置错重试也没用。
fn is_retryable(err: Option<&str>) -> bool {
    matches!(
        err,
        Some("timeout_local")
            | Some("timeout_core")
            | Some("connect")
            | Some("http_5xx")
            // 未知但非协议类错误：给一次机会
            | Some("unknown")
    )
}

fn map_probe_err(e: &crate::kernel::api::ApiError) -> String {
    use crate::kernel::api::ApiError as A;
    match e {
        A::Timeout => "timeout_core".into(),
        A::Status(401) => "unauthorized".into(),
        A::Status(404) => "not_found".into(),
        A::Status(s) if *s >= 500 => "http_5xx".into(),
        A::Status(s) => format!("http_{s}"),
        A::Connect => "connect".into(),
        A::Decode => "bad_json".into(),
        A::Other(_) => "unknown".into(),
    }
}
```

### 6.5.1 `CoreApi::proxy_delay` 需要的返回约定

```rust
impl CoreApi {
    /// GET /proxies/{name}/delay?url=&timeout=&expected=
    /// 200 → {"delay": u32}（delay 可能为 0，表示 <1ms，C.10）
    /// 504 → ApiError::Timeout
    /// 401 → ApiError::Status(401)（→ E_CORE_API_UNAUTH）
    /// 其它非 2xx → ApiError::Status(code)
    pub async fn proxy_delay(
        &self,
        name: &str,
        url: &str,
        timeout_ms: u32,
        expected: u16,
    ) -> Result<u32, ApiError> { /* 见附录 A */ }
}
```

> **节点名必须 URL 编码**：`Hong Kong 01 | 5x` 这类名字含空格、`|`、中文，未编码会得到 400。
> 用 `url::form_urlencoded::byte_serialize` 或 `percent-encoding`，**不要**手写替换。

---

## 6.6 排除与排序（rank.rs，纯函数）

```rust
// speedtest/rank.rs
use crate::domain::model::{ProbeResult, ProxyKind};
use super::probe::ProbeTarget;

/// 来自 `SwitchPolicy.exclude_keywords`；引擎另外硬编码兜底几条。
#[derive(Debug, Clone)]
pub struct ExcludeRules {
    pub keywords: Vec<String>,
}

impl ExcludeRules {
    pub fn from_policy(p: &crate::domain::model::SwitchPolicy) -> Self {
        let mut kw: Vec<String> = p
            .exclude_keywords
            .iter()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        // 兜底：即使用户把 exclude_keywords 清空，这几类也永不参与切换
        for extra in ["direct", "reject", "traffic", "expire", "官网", "套餐"] {
            if !kw.iter().any(|k| k == extra) {
                kw.push(extra.to_string());
            }
        }
        Self { keywords: kw }
    }
}

/// 内置策略名（mihomo 保留），大小写不敏感比较
const BUILTIN: [&str; 6] = ["DIRECT", "REJECT", "PASS", "REJECT-DROP", "COMPATIBLE", "GLOBAL"];

/// 纯函数：判断一个节点是否参与测速/切换。
/// 返回 Some(reason) 表示排除，reason 用于日志与 UI 解释。
pub fn exclude_reason(node_key: &str, kind: ProxyKind, rules: &ExcludeRules) -> Option<&'static str> {
    if BUILTIN.iter().any(|b| b.eq_ignore_ascii_case(node_key)) {
        return Some("builtin");
    }
    // 组类型（Selector/URLTest/Fallback/LoadBalance）本身不是节点，kind 应为 Other 且通常带 all 字段；
    // 引擎在 Inventory 阶段已剔除，这里再兜一层类型白名单。
    if matches!(kind, ProxyKind::Other) {
        return Some("not_a_proxy");
    }
    let lower = node_key.to_lowercase();
    if rules.keywords.iter().any(|k| lower.contains(k.as_str())) {
        return Some("info_node");
    }
    None
}

/// 前置过滤：去重 + 排除。保持输入顺序稳定（便于快照测试）。
pub fn filter_targets(input: Vec<ProbeTarget>, rules: &ExcludeRules) -> Vec<ProbeTarget> {
    let mut seen = std::collections::HashSet::new();
    input
        .into_iter()
        .filter(|t| exclude_reason(&t.node_key, t.kind, rules).is_none())
        .filter(|t| seen.insert(t.node_key.clone()))
        .collect()
}

/// 排序后的候选（07 章的 selector 直接消费这个）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub node_key: String,
    pub group: String,
    pub delay_ms: u32,
    pub kind: ProxyKind,
}

/// 排序键（从小到大）：
///   1) delay_ms 升序（唯一的「好坏」主键）
///   2) 失败次数升序（本轮无失败=0，重试过=1；由 error 是否曾出现推导，见 failure_count）
///   3) 地区偏好下标升序（prefer_region_priority 未命中记 usize::MAX）
///   4) node_key 字节序升序（**稳定排序，保证同延迟时结果确定可复现**）
///
/// 排除的：内置策略、非节点、信息节点、delay 为 None 的失败项。
pub fn rank_candidates(results: &[ProbeResult], rules: &ExcludeRules) -> Vec<Candidate> {
    rank_candidates_with_priority(results, rules, &[])
}

pub fn rank_candidates_with_priority(
    results: &[ProbeResult],
    rules: &ExcludeRules,
    region_priority: &[String],
) -> Vec<Candidate> {
    let mut list: Vec<Candidate> = results
        .iter()
        .filter(|r| exclude_reason(&r.node_key, r.kind, rules).is_none())
        .filter_map(|r| {
            r.delay_ms.map(|d| Candidate {
                node_key: r.node_key.clone(),
                group: r.group.clone(),
                delay_ms: d,
                kind: r.kind,
            })
        })
        .collect();

    list.sort_by(|a, b| {
        a.delay_ms
            .cmp(&b.delay_ms)
            .then_with(|| region_rank(&a.node_key, region_priority).cmp(&region_rank(&b.node_key, region_priority)))
            .then_with(|| a.node_key.as_bytes().cmp(b.node_key.as_bytes()))
    });
    list
}

/// 地区命中下标；未命中返回 usize::MAX（排最后）
pub fn region_rank(node_key: &str, priority: &[String]) -> usize {
    if priority.is_empty() {
        return 0; // 不参与排序，全部同权
    }
    let lower = node_key.to_lowercase();
    priority
        .iter()
        .position(|p| lower.contains(&p.to_lowercase()))
        .unwrap_or(usize::MAX)
}
```

> **为什么延迟并列要按名称字节序**：一次测速里 3 个节点都是 42ms 很常见（同一机房）。
> 若不加最终 tiebreak，`sort_by` 的稳定性会让结果随探测完成顺序漂移，
> 每次测速都「切换」到名字不同的 42ms 节点 → 用户看到的是无意义的抖动。
> 加名称序后，同延迟永远选同一个，**这就是最便宜的一层防抖**。

---

## 6.7 进度事件合并（≤4 条/秒）

```rust
// speedtest/engine.rs（同文件内）
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

pub struct ProgressCoalescer {
    events: crate::event::EventSink,
    run_id: String,
    total: u32,
    window: Duration,
    batch: Arc<Mutex<Vec<crate::domain::model::ProbeResult>>>,
    finished: Arc<AtomicU32>,
    stopped: Arc<AtomicBool>,
}

#[derive(Clone)]
pub struct ProgressHandle {
    batch: Arc<Mutex<Vec<crate::domain::model::ProbeResult>>>,
    finished: Arc<AtomicU32>,
}

impl ProgressHandle {
    /// 由探测任务在完成后调用（已持有 snapshot 锁外调用，避免锁序反转）
    pub async fn push(&self, r: crate::domain::model::ProbeResult) {
        self.finished.fetch_add(1, Ordering::Relaxed);
        let mut buf = self.batch.lock().await;
        // 上限保护：窗口内最多保留 32 条，防止 1 万个节点时缓冲膨胀
        if buf.len() < 32 {
            buf.push(r);
        } else if let Some(slot) = buf.last_mut() {
            *slot = r; // 始终保留最新一条作为 latest
        }
    }
}

impl ProgressCoalescer {
    pub fn new(
        events: crate::event::EventSink,
        run_id: String,
        total: u32,
        window: Duration,
    ) -> Self {
        Self {
            events,
            run_id,
            total,
            window,
            batch: Arc::new(Mutex::new(Vec::new())),
            finished: Arc::new(AtomicU32::new(0)),
            stopped: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn handle(&self) -> ProgressHandle {
        ProgressHandle { batch: self.batch.clone(), finished: self.finished.clone() }
    }

    /// 唯一的进度定时器：只在测速运行期存在，运行结束即退出（空闲期零定时器，满足 C.11）
    pub async fn run(&self) {
        let mut ticker = tokio::time::interval(self.window);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if self.stopped.load(Ordering::Relaxed) {
                return;
            }
            let mut buf = self.batch.lock().await;
            if buf.is_empty() {
                continue;
            }
            // 取最后一条作为 latest（C.7 负载只有单条 latest，故此处语义是「本窗口最新完成的一条」）
            let latest = match buf.pop() {
                Some(x) => x,
                None => continue,
            };
            buf.clear();
            drop(buf);
            let payload = serde_json::json!({
                "runId": self.run_id.clone(),
                "finished": self.finished.load(Ordering::Relaxed),
                "total": self.total,
                "latest": latest,
            });
            self.events.emit_json("speedtest:progress", payload);
        }
    }

    pub async fn flush_and_stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        let mut buf = self.batch.lock().await;
        if let Some(latest) = buf.pop() {
            buf.clear();
            drop(buf);
            let payload = serde_json::json!({
                "runId": self.run_id,
                "finished": self.finished.load(Ordering::Relaxed),
                "total": self.total,
                "latest": latest,
            });
            self.events.emit_json("speedtest:progress", payload);
        }
    }
}
```

> **两个定时器不冲突**：`ProgressCoalescer::run()` 与主循环里的 `deadline` 生命周期都被约束在
> 单次 run 内（run 结束即 drop），因此空闲期（无测速）应用内**零**额外定时器，满足 C.11
> 「后台唤醒」预算。运行期的 4 次/秒唤醒持续 ≤ 5s，落在 C.11「测速期间」预算内。

**合并规则表**

| 场景 | 原始事件量 | 合并后 | 说明 |
| --- | --- | --- | --- |
| 8 并发 × 典型 40ms | ~200/s | 4/s | 每窗口只发 1 条，`finished` 用累计值 |
| 128 节点全部超时（3s） | 最多 8 个/秒（受并发限） | 4/s | 窗口内丢中间条目，只保 `latest` |
| 单条 40ms 的极小订阅 | 5 条总量 | 5 条（自然低于阈值） | 无需强合并 |
| 引擎取消 | 残余缓冲 | 立刻 flush 1 条 + done | 前端进度不卡住 |
| 前端未挂载（窗口隐藏） | — | 仍然发（后端不知情） | 由 09 章前端丢弃渲染，后端不特殊处理 |

> **取 `buf.pop()` 而不是 `buf.remove(0)`**：C.7 的 `latest` 是「最新一条」，
> 取尾部即最新；且 `pop()` 是 O(1)。`buf.clear()` 保证内存不随订阅规模增长。

---

## 6.8 取消与资源回收

| 取消源 | 触发 | 行为 |
| --- | --- | --- |
| 用户点「取消」 | `speedtest_cancel{runId}` → `cancel.cancel()` | 未开始的探测不再排队；在飞的 reqwest 请求随 future drop 而 abort |
| 总预算超时 | `deadline` 分支 | `probe_cancel.cancel()`（子 token），已得结果保留 |
| 内核重启 | 引擎收到内核 Down | 取消整轮，`speedtest:done`，`finished<total`，**不**入退避 |
| 应用退出 | 全局 `shutdown` token | 同上，且不再 emit 事件 |
| 新一轮 start | 幂等：直接返回旧 runId | 不取消、不并发 |

**取消安全自查清单（code review 必看）**

1. `Semaphore::acquire_owned()` 在 `select!` 里被取消时，不会泄漏 permit（tokio 保证）。
2. `FuturesUnordered` 被 drop 后，所有 future drop，`_permit` 随之释放。
3. `snapshot` 的锁**从不跨 await 持有网络请求**（只覆盖 push/计数）。
4. 取消后 `inflight` 必须置 `None`，否则下一次 `speedtest_start` 永远返回旧 runId（僵尸态）。
5. `tokio::spawn` 的 Future 必须包一层「无论什么路径都清理」的结构（已在 §6.4 的 spawn 尾部处理）。

---

## 6.9 与 mihomo `url-test` / `fallback` 组的关系

这是本项目最容易出**双引擎抖动**的地方，必须显式设计。

| 组类型 | 内核自身行为 | 本应用策略 |
| --- | --- | --- |
| `select` | 不自动选，只能 PUT 指定 | **主战场**：显式 `PUT /proxies/{group}`，见 07 章 |
| `url-test` | 内核按自己的 interval 自动选最快，**并会覆盖固定选择** | 从生成的 runtime config 中**改写为 `select`**（04 章职责），或保留但**加入排除名单**不参与本应用切换 |
| `fallback` | 内核按顺序取第一个可用 | 可保留；本应用不改它 |
| `load-balance` | 内核轮询 | 可保留；本应用不改它 |

**为什么不直接依赖内核的 `url-test`**

1. 内核的 `url-test` 用自己的 `test-url` 与 `interval`（默认 300s），与本应用的 `speedtest_probe_url`
   / `speedtest_interval_min` 不一致 → 两套节奏，节点来回跳；
2. 内核不感知 `exclude_keywords`（信息节点会参与），也不感知 `tolerance_ms`（延迟抖动 5ms 就切）；
3. 内核切换无 `SwitchRecord`，用户无法回答「为什么切了」——违反 01 §3-7「可解释」。

**因此：本应用对同一组只允许一个决策者。**

```text
规则 R6.9（写入 04 章「groups 生成」与本章「切换」两侧）：
  若 switch_policy.enabled == true 且 scope == Group(G):
    生成的 runtime config 中 G 必须是 type: select；
    不得生成同名 url-test 组；
  若 switch_policy.scope == Global：
    全部参与自动切换的组都是 select，由 07 章统一决策并发 PUT。
```

**显式覆盖时序**

```text
speedtest run 完成
  └─ selector::decide_and_apply(run, snapshot_of_now)
        ├─ 只对 scope 内、type==Selector 的组发 PUT
        ├─ 每个 PUT 成功后 GET /proxies/{group} 校验 now == 目标节点
        │     （内核可能因 profile.store-selected 异步回写，需校验后再落 SwitchRecord）
        └─ 失败 → 回滚到 from_node（见 07 章 §7.7）
```

---

## 6.10 错误码映射（不得自造）

| 现象 | 内核/网络层 | `AppError.code` | 用户文案 | 是否重试 |
| --- | --- | --- | --- | --- |
| 401 | `ApiError::Status(401)` | `E_CORE_API_UNAUTH` | 内核 API 鉴权失败（secret 不一致） | 否 |
| 内核进程不在 | 连接被拒 | `E_CORE_NOT_FOUND` | 未安装内核，请先下载 | 否 |
| 内核在但端口对不上 | 连接被拒（不同端口） | `E_CORE_START_FAILED` | 内核启动失败：{detail} | 否（由 08 章重启） |
| 内核根本没起来（端口耗尽） | 无 `api_endpoint` 可连 | `E_CORE_PORT_EXHAUSTED` | 无可用本地端口 | 否（用户需释放端口或改 `api_port`） |
| 504 | `ApiError::Timeout` | 计入 `ProbeResult.error="timeout_core"` | 不弹窗 | 是（1 次） |
| 本地超时 | `Elapsed` | `"timeout_local"` | 不弹窗 | 是（1 次） |
| 404 | 节点名不存在 | `"not_found"` | 不弹窗 | 否 |
| 5xx | 内核内部错误 | `"http_5xx"` | 不弹窗 | 是（1 次） |
| 400 | 名称未编码 | `"http_400"` | 不弹窗（属代码 bug，应被单测拦住） | 否 |
| 全部节点失败 | —— | **不是错误**：`best=None`，`done` 正常发 | 「本轮全部节点不可用」 | — |
| 当前已有 run | —— | **不报错**，返回旧 runId | —— | — |

> `E_BUSY` 在本章的**唯一**用法：`switch_apply_best` 被调用时，`runId` 对应的 run 尚未
> `finished_at`（前端抢跑）。该判定在 07 章，本章不产生 `E_BUSY`。
> 单条探测失败**绝不**冒泡成 `AppError` 弹窗——否则 128 个节点同时超时会弹 128 个通知。

---

## 6.11 单元测试用例表（≥12 行，全部离线可跑）

> 测试文件：`src-tauri/tests/speedtest_rank.rs`、`src-tauri/tests/speedtest_engine.rs`。
> 引擎测试用 `FakeCoreApi`（实现与 `CoreApi` 相同的 trait，按节点名返回预设延迟/错误，不碰网络）。

| # | 用例 | 输入 | 期望 | 类型 |
| --- | --- | --- | --- | --- |
| 1 | 基本排序 | 3 节点 42/118/64 | 顺序 `42,64,118` | rank 纯函数 |
| 2 | 延迟并列稳定 | `HK-01=42`,`HK-02=42`,`HK-03=42` | 顺序按字节序 `HK-01,HK-02,HK-03`，多次调用一致 | rank 纯函数 |
| 3 | 排除 DIRECT/REJECT | 含 `DIRECT`、`REJECT`、`reject-drop` | 全部被 `exclude_reason=builtin` | rank 纯函数 |
| 4 | 排除信息节点 | `剩余流量：100GB`、`到期：2026-12-01`、`官网 example.com` | 全部 `info_node` 被剔除 | rank 纯函数 |
| 5 | 关键词大小写 | 节点名 `TRAFFIC-01`，keywords 含 `traffic` | 被排除（compare 前 lowercase） | rank 纯函数 |
| 6 | 用户清空 keywords | `exclude_keywords=[]` | 仍排除 direct/reject/官网/套餐（兜底生效） | rank 纯函数 |
| 7 | 失败项不进候选 | 一节点 `delay_ms=None, error="timeout_core"` | 不在 `rank_candidates` 结果中 | rank 纯函数 |
| 8 | 全部失败 | 所有 `delay_ms=None` | 返回空 Vec，`best=None`，`finished==total` | 引擎集成 |
| 9 | 全被排除 | 全部为信息节点 | `total=0`，秒发 `speedtest:done`，无探测请求 | 引擎集成 |
| 10 | 并发上限 | 32 节点，concurrency=8，FakeApi 记录同时在飞峰值 | 峰值 ≤ 8 | 引擎集成 |
| 11 | 重试一次成功 | FakeApi 首次 `Timeout`，第二次 42ms | 结果 `42`，总请求数 2 | 引擎集成 |
| 12 | 4xx 不重试 | FakeApi 返回 404 | 结果 `not_found`，请求数 1 | 引擎集成 |
| 13 | 重试仍失败 | 两次都 `Timeout` | `delay=None, error="timeout_core"`，请求数 2 | 引擎集成 |
| 14 | 取消 | 64 节点，50ms 后 `cancel()` | `finished < total`，`done` 只发 1 次，无 panic | 引擎集成 |
| 15 | 进度节流 | 100 节点 × 10ms，统计 1s 内事件数 | ≤ 4 条（250ms 窗口） | 引擎集成 |
| 16 | 预算耗尽 | timeout=1000，50 节点，FakeApi 全部 hang 到 2s | 总耗时 ≤ `budget_ms`，`done` 正常发 | 引擎集成 |
| 17 | 幂等 start | 连续两次 `start` | 第二次返回 `AlreadyRunning(同一 runId)`，不产生第二轮探测 | 引擎集成 |
| 18 | 预算公式 | `budget_ms(total=100, conc=8, timeout=3000)` | `ceil(100/8)=13` 波 → `13×3500×2+2000=93000ms` | 纯函数 |
| 19 | 预算封顶 | `budget_ms(total=10000, conc=1, timeout=10000)` | 被 `HARD_BUDGET_MS=240000` 截断 | 纯函数 |
| 20 | 配置越界 | `concurrency=0`，`timeout=20000` | clamp 到 `1` 与 `10000` | 纯函数 |
| 21 | 地区偏好 | priority `["HK","JP"]`，延迟 `JP=40,HK=45` | 仍返回 `JP`（延迟优先于地区） | rank 纯函数 |
| 22 | 地区偏好-并列 | `HK=40,JP=40`，priority `["HK"]` | 返回 `HK`（地区后置 tiebreak） | rank 纯函数 |
| 23 | 节点去重 | 同一 `node_key` 出现在两个组 | `filter_targets` 只保留 1 条 | rank 纯函数 |
| 24 | 取消后重启 | 取消 → 等 done → 再次 start | 新 runId ≠ 旧 runId，`inflight` 已释放 | 引擎集成 |

**FakeCoreApi 骨架**

```rust
struct FakeCoreApi {
    delays: std::collections::HashMap<String, Vec<Result<u32, ApiError>>>, // 按调用次数出队
    inflight: std::sync::atomic::AtomicUsize,
    peak: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl crate::kernel::api::DelayProbe for FakeCoreApi {
    async fn proxy_delay(&self, name: &str, _u: &str, _t: u32, _e: u16) -> Result<u32, ApiError> {
        let cur = self.inflight.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        self.peak.fetch_max(cur, std::sync::atomic::Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        self.inflight.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        self.delays.lock().get_mut(name).and_then(|v| if v.len() > 1 { Some(v.remove(0)) } else { v.first().cloned() })
            .unwrap_or(Err(ApiError::Status(404)))
    }
}
```

---

## 6.12 验收与观测

| 项 | 命令/方法 | 通过标准 |
| --- | --- | --- |
| 单测 | `cargo test --test speedtest_rank --test speedtest_engine` | 24 条全绿 |
| Clippy | `cargo clippy --all-targets -- -D warnings` | 无警告，无 `unwrap()` |
| 真机并发 | 128 节点，`concurrency=8`，看 mihomo 日志 | 峰值并发 8，总量 128 |
| CPU 预算 | 测速期间 `perf-probe.ps1` | ≤ 25% 单核且持续 ≤ 5s（C.11） |
| 事件频率 | 前端 `listen("speedtest:progress")` 计数 | ≤ 4 条/秒 |
| 取消 | 测速中关窗口再打开 | 不重复跑、不卡进度条 |
| 与 url-test 共存 | 用带 `url-test` 组的订阅跑 10 轮 | 组内 `now` 只被本应用改变（对比 `SwitchRecord`） |

**日志样例（可解释性）**

```
2026-11-06T16:02:03.101 INFO  [speedtest] start run=7f3a9c trigger=Timer total_candidate=57
2026-11-06T16:02:03.104 INFO  [speedtest] excluded count=6 reasons={info_node:4,builtin:2}
2026-11-06T16:02:03.212 WARN  [speedtest] probe node=SG-02 attempt=1/2 code=timeout_core
2026-11-06T16:02:04.880 INFO  [speedtest] done total=51 ok=48 best=HK-01 delay=42ms elapsed=1776ms
```

---

## 6.13 常见坑

| 坑 | 现象 | 对策 |
| --- | --- | --- |
| 用 `/group/{g}/delay` | 用户手选的节点被清空 | 只用 `/proxies/{name}/delay` |
| 漏 `expected=204` | 机场劫持页返回 200 被算作 12ms | 强制带 `expected=204` |
| 节点名未 URL 编码 | 中文/空格节点全部 400 | 统一 `percent-encoding`，见 §6.5 |
| 本地超时 < 内核超时 | 大量「本地超时」假失败 | 本地 = 内核 + 500ms |
| 无限重试 | 机场故障时每轮打 2×N 请求 | 固定重试 1 次，且 4xx 不重试 |
| 进度事件直接 emit | 事件风暴，前端掉帧 | `ProgressCoalescer` 250ms |
| 排序无 tiebreak | 同延迟节点轮流被选，用户觉得抖动 | 名称字节序兜底 |
| 忽略 `alive=false` | （反例）用它排除节点 → mihomo 的 alive 可能陈旧，误杀好节点 | 只测不看，`alive_hint` 仅入日志 |
| `inflight` 未释放 | 第二轮 start 永远拿到僵尸 runId | `spawn` 尾部无条件置 `None` |
| 每节点开 `tokio::spawn` | 128 个任务同时抢连接，CPU 尖峰 | `Semaphore` 限流 + `FuturesUnordered` |
| **在 `for` 循环里 `await acquire_owned()`** | 死锁：第 9 个请求永久阻塞，因前 8 个 future 还没被 poll | permit 必须在 future **内部**获取（见 §6.4.1 注释） |
| 在「组内」用 `FuturesUnordered` 但不 pin | 编译错或运行 panic | `FuturesUnordered` 是 `Unpin`，直接 `tasks.next()` 即可，不需要 `pin!` |
| 用 `JoinSet` 而非 `FuturesUnordered` | 每个节点一个 task，128 节点 = 128 个 task 调度开销 | 探测是轻量 IO，`FuturesUnordered` 更省；`JoinSet` 留给需要 `abort()` 的 CPU 任务 |

---

## 6.14 本章自检清单（Agent 必须逐条确认）

> 交付任何代码前逐条回答「是/否」，任一「否」不得进入下一章。

- [ ] 1. 探测是否只调 `GET /proxies/{name}/delay?url=&timeout=&expected=204`？是否**完全没有**用 `GET /group/{group}/delay`（会清空用户手选）？
- [ ] 2. 是否强制带 `expected=204`？漏带会让机场劫持页的 `200` 被算成极低延迟。
- [ ] 3. 节点名是否做了 URL 编码（`percent-encoding`）？中文/空格/emoji 节点名不编码会 400。
- [ ] 4. 本地超时是否 = 内核 `timeout` + 500ms？反过来写会产生大量「本地超时」假失败。
- [ ] 5. 并发是否用 `Semaphore`（默认 8，clamp 1..=16）且 permit 在 **future 内部**获取？在 `for` 循环里 `await acquire_owned()` 会死锁（§6.4.1）。
- [ ] 6. 重试是否固定 1 次且 4xx **不重试**？无限重试会让机场故障时每轮打 2×N 请求。
- [ ] 7. `speedtest_start` 是否**幂等**（已在跑则返回同一 `runId`，不报 `E_BUSY`）？`speedtest_status` / `speedtest_cancel` 是否也幂等？
- [ ] 8. 是否**没有**新增 `speedtest:cancelled` 之类事件？取消仍发 `speedtest:done`，前端靠 `finished < total` 判断（C.7）。
- [ ] 9. 进度事件是否经 `ProgressCoalescer` 合并（`PROGRESS_FLUSH_MS = 250`）？是否 ≤ 4 条/秒（C.11）？
- [ ] 10. 收尾顺序是否为「停 flush 循环 → flush 末批 → 发 `speedtest:done`」？先发 done 会导致进度条停在 98%。
- [ ] 11. 取消/预算超时后，未完成的探测结果是否**不计入** `results`？计入会污染「失败率」统计。
- [ ] 12. 排序是否用 `rank_candidates_with_priority`（带地区 tiebreak），且 tiebreak 按节点名**字节序**兜底？否则同延迟节点每轮轮流被选。
- [ ] 13. 排除规则（`DIRECT`/`REJECT`/信息节点关键词）是否只来自 `ExcludeRules::from_policy`，与 C.5 `exclude_keywords` 同源？
- [ ] 14. 是否**不看** `alive` 字段？mihomo 的 `alive` 可能陈旧（延迟探测已被裁剪），用它排除会误杀好节点。
- [ ] 15. `switch_policy.scope` 内的组是否由 04 章生成为 `type: select`？（C.5 组类型约束；否则内核 url-test 与本应用抢节点）
- [ ] 16. 测速完成后（`trigger ≠ Manual`）是否**直接**调 `selector::after_speedtest()`，顺序为「先 `update_failure_counters` 再 `apply_best`」？
- [ ] 17. 切换失败是否**不影响**测速结果？`speedtest:done` 是否照常发出（只记 Warn）？
- [ ] 18. 所有错误码是否取自 C.8（`E_CORE_API_UNAUTH` / `E_CORE_NOT_FOUND` / `E_CORE_START_FAILED` / `E_CORE_PORT_EXHAUSTED` / `E_UNSUPPORTED` / `E_BUSY`）？有无自造？
- [ ] 19. `budget_ms()` / `rank` / `ProgressCoalescer` 等纯函数是否有单测？§6.12 的 24 条用例是否全绿？
- [ ] 20. `probe` 层是否有 `unwrap()` / `expect()` / `panic!()`？超时、取消、5xx 是否都走 `Result`？
- [ ] 21. 应用退出时（`shutdown_token`）是否强制取消在跑的探测，不阻塞进程退出（C.13）？
- [ ] 22. 本章是否**没有**修改 `01-master-prompt.md`、`appendices/C-conventions.md` 或其它章节文件？
