# 性能实测记录（契约 C.11）

> 环境：Windows 11 / 16 逻辑核 / 实测日期 2026-11-06
> 被测二进制：`src-tauri/target/release/clash-zhengl.exe`（7.27 MB，含静态链接的 Tauri 运行时）
> 采样工具：`scripts/perf-probe.ps1`、`scripts/mem-diag.ps1`

## 汇总

| 指标 | 预算 | 实测 | 结果 |
| --- | --- | --- | --- |
| 冷启动到窗口可用 | ≤ 1.2 s | **29 ms**（3 次：46 / 20 / 20） | ✅ 达标（余量 41×） |
| 应用进程 RSS（主进程） | ≤ 90 MB | **45.0 MB** | ✅ 达标（50%） |
| 空闲 CPU（16 核归一化） | ≤ 1.5% | **0.05%**（峰值 0.20%） | ✅ 达标（3%） |
| 二进制体积 | — | 7.27 MB | ✅ |
| 安装包体积 | ≤ 12 MB | 见下文打包结果 | ⏳ |
| 前端产物（JS + CSS） | < 300 KB | **55.5 KB**（gzip 19.1 KB） | ✅ 达标（18%） |

## WebView2 内存明细（信息性，不计入契约预算）

契约 C.11 的口径是「**应用进程** RSS」，即 `clash-zhengl.exe` 本身，实测 45 MB。
WebView2 由 Windows 系统提供、独立进程承载，明细如下：

| 进程 | Working Set | Private |
| --- | --- | --- |
| clash-zhengl.exe（应用） | 45.0 MB | 16.5 MB |
| msedgewebview2.exe ×6（子进程） | 334.5 MB | 150.0 MB |
| **合计** | 379.4 MB | 166.5 MB |

**为什么 Working Set 高但实际占用低**：WebView2 的 6 个进程共享同一份 `msedge.dll` 映像，
Working Set 把共享页重复计入每个进程；Private Working Set 才反映真实增量，合计 150 MB。
这部分是**所有 WebView2 宿主（含 Tauri / Electron / 任何套壳应用）的固有成本**，
不随我们前端代码量变化——我们的页面 JS 只有 42 KB。

已做的优化（`tauri.conf.json` → `additionalBrowserArgs`）：

| 参数 | 作用 |
| --- | --- |
| `--renderer-process-limit=1` | 单窗口只需 1 个渲染进程，防止多开 |
| `--js-flags=--max-old-space-size=128` | 限制 V8 堆（前端 < 1MB JS，128MB 绰绰有余） |
| `--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection` | 关掉用不到的 PDF/Office/SmartScreen 组件 |
| `--disable-gpu-shader-disk-cache` | 不缓存着色器，省磁盘与 GPU 进程内存 |
| `--disable-dev-shm-usage` | 避免共享内存文件膨胀 |

优化前后 Working Set 变化不明显（378 → 379 MB），说明瓶颈在 WebView2 基础开销而非配置。
进一步的处置按 C.11 的仲裁规则执行，见下节。

## 口径争议处置（按 C.11 仲裁条款）

C.11 原文：「若实测在预算的 100%–120% 之间，允许保留并记入 `docs/perf-log.md`，
但下一阶段必须给出改进项；超过 120% 视为不达标，必须立即处置。」

本例的判定：

- **应用进程口径**（契约字面）：45 MB / 90 MB = **50%**，达标，无争议。
- **进程树合计口径**（更严格的自我要求）：379 MB / 90 MB = 421%，超标。

结论：按契约口径判为**达标**，并把进程树数字如实记录在此，避免读者误以为"总共只用 45MB"。

如果不接受 WebView2 的成本，唯一替代方案是放弃 WebView、改用原生 Win32 绘制
（如 `native-windows-gui` 或直接 `windows-sys` 自绘），代价是：

1. 全部 UI 需重写为 Rust（约 3000+ 行），失去 HTML/CSS 的表达力
2. 富文本、滚动、输入法、高 DPI 缩放全部要自己处理
3. 开发周期从「已完成」回到「重新开始」

以本项目当前规模（4 个视图、约 2000 行 TS），重写收益远低于成本，因此保留 WebView2，
并把「应用进程 RSS」作为实际监控指标（这是用户能感知的部分：任务管理器里
`Clash Zhengl` 那一行显示的就是 45 MB）。

## 复现方式

```powershell
# 预算判定（会自动对照 C.11 打 PASS/FAIL）
powershell -ExecutionPolicy Bypass -File .\scripts\perf-probe.ps1 -Samples 10

# 内存明细诊断（拆分主进程 / WebView2 / Private）
powershell -ExecutionPolicy Bypass -File .\scripts\mem-diag.ps1 -WarmupSec 12 -Samples 6
```

## 历史

| 日期 | 变更 | 冷启动 | 应用 RSS | 备注 |
| --- | --- | --- | --- | --- |
| 2026-11-06 | 首次达标 | 29 ms | 45.0 MB | WebView2 参数已加，见上表 |
