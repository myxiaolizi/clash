# Clash Zhengl

Windows 桌面代理客户端，基于 **Tauri 2 + Rust + TypeScript**。
软件本身不提供任何代理服务，只作为 mihomo 内核的图形界面 —— 你需要自备合法订阅。

## 功能

| 模块 | 说明 |
|---|---|
| 订阅管理 | 多订阅并行拉取、失败重试退避、按周期自动更新（下限 15 分钟） |
| 流量统计 | 解析 `Subscription-Userinfo`（流量/到期时间），多订阅聚合 |
| 节点与测速 | 读取内核节点组，全量并发测速（下限 5 分钟），延迟分档着色 |
| 自动切换 | 按「失败救援 → 最短驻留 → 已最优 → 优势阈值」决策，抖动 0–30s |
| 手动覆盖 | 手动选节点后进入覆盖期，期间自动切换暂停 |
| 内核管理 | 一键下载安装 mihomo（自动识别官方 zip 内的平台后缀文件名）、启停、崩溃重启 |
| 系统代理 | 写入注册表并回读校验，崩溃后按 journal 快照自动还原 |
| 电源与托盘 | 开箱即用托盘菜单、关闭到托盘、静默启动 |

## 快速开始

前置：Rust stable、Node.js 18+、Windows 11。

```powershell
npm install
npm run tauri dev          # 开发
npm run tauri build        # 打包（产出 NSIS 安装包）
```

> 构建必须用 `npm run tauri build`，不要用 `cargo build --release`。
> 后者不会注入前端资源，二进制会带着 `devUrl` 去找 `127.0.0.1:5273`，启动后只会看到
> `ERR_CONNECTION_REFUSED` 白屏。

## 目录结构

```
src/                    前端（原生 DOM，无框架）
  ipc/client.ts         37 个 IPC 命令的类型化封装 + 事件订阅
  ui/views/             4 个页面：订阅 / 流量 / 节点 / 设置
src-tauri/src/
  domain/               数据模型、错误码、切换决策（纯函数，可单测）
  subscription/         订阅解析、用户信息、配置生成
  kernel/               mihomo 下载/启动/健康检查/热重载
  speedtest/            测速引擎与排名
  scheduler/            周期任务（抖动 + 退避）
  store/                设置、订阅元数据、切换历史；DPAPI 加密
  system/               路径、原子写、系统代理
prompts/                本项目的开发契约与提示词套件（契约 C 章为唯一权威）
docs/perf-log.md        性能实测记录（C.11 预算对照）
scripts/verify-all.ps1  9 步一键验收
```

## 验收

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\verify-all.ps1
```

9 项全部通过：`cargo check`、单元测试、`tsc`、前端构建、契约自检、
Selector 命名空间陷阱扫描、配置硬约束、性能实测、NSIS 打包。

## 性能（实测）

| 指标 | 预算 | 实测 |
|---|---|---|
| 冷启动 | ≤ 1.2 s | 29–71 ms |
| 应用进程 RSS | ≤ 90 MB | 35.4–46.1 MB |
| 空闲 CPU | ≤ 1.5 % | 0.00–0.05 % |
| 安装包 | ≤ 12 MB | 2.44 MB |

详见 [docs/perf-log.md](docs/perf-log.md)。

## 许可证

MIT
