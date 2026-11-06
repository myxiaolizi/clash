# Clash Zhengl · 开发提示词套件（使用说明）

> 这是一套\*\*可直接投喂给 AI 编码代理\*\*（Claude Code / Cursor / Codex / Copilot Agent / 其他自主编码 Agent）的提示词集合，
> 用于从零开发 Windows x64 轻量级代理客户端 \*\*Clash Zhengl\*\*。
>
> 它不是软件源码，而是「把软件做出来的施工图 + 施工指令」。

\---

## 1\. 目标软件（一句话）

**Clash Zhengl**：一个 Windows x64 桌面客户端，用户填入一个或多个机场订阅地址后，
自动拉取 Clash/mihomo 订阅配置，展示**流量用量与到期日**，按用户设定周期**自动更新订阅**，
按用户设定周期**自动测速**，并在可用节点中**自动切换到延迟最低的节点**。
整体资源占用对标并优于 Clash Verge，内存与包体尽可能小。

参考项目：

* Clash Verge Rev 文档：[https://www.clashverge.dev/index.html](https://www.clashverge.dev/index.html)
* Clash Verge Rev 源码：[https://github.com/clash-verge-rev/clash-verge-rev](https://github.com/clash-verge-rev/clash-verge-rev)
* 内核 mihomo（原 Clash.Meta）：[https://github.com/MetaCubeX/mihomo](https://github.com/MetaCubeX/mihomo)
* mihomo 文档：[https://wiki.metacubex.one/](https://wiki.metacubex.one/)

> 若 GitHub 直连失败，用本机代理 `http://127.0.0.1:1886` 访问
> （`git -c http.proxy=http://127.0.0.1:1886 clone ...` 或设置 `HTTPS\_PROXY`）。

> \*\*契约版本\*\*：本套提示词统一对齐 \*\*C 章 v1.3\*\*（2026-11-06）。
> 所有章节的命令名、事件名、错误码、数据模型、路径均以 \[appendices/C-conventions.md](appendices/C-conventions.md) 为唯一真源；
> 章节正文与契约冲突时，\*\*一律以契约为准\*\*。契约变更须走 `CONTRACT-CHANGE-REQUEST` 流程（见 13 章 P9 卡），
> 并在 C 章顶部的变更日志表中登记。

\---

## 2\. 文档清单与投喂顺序

|顺序|文件|作用|何时用|
|-|-|-|-|
|①|[01-master-prompt.md](01-master-prompt.md)|总纲 + 角色 + 铁律|**每次新会话第一件事**贴入（或放 `AGENTS.md` / `CLAUDE.md`）|
|②|[appendices/C-conventions.md](appendices/C-conventions.md)|工程契约：目录、数据模型、IPC、错误码、性能预算|与 ① 一起作为常驻上下文|
|③|[02-tech-stack-and-architecture.md](02-tech-stack-and-architecture.md)|技术选型对比与架构定稿|开工前一次性确定|
|④|[03-subscription-fetch-and-parse.md](03-subscription-fetch-and-parse.md)|订阅拉取、解析、流量/到期提取|阶段 P2|
|⑤|[04-multi-subscription-merge.md](04-multi-subscription-merge.md)|多订阅去重合并成运行配置|阶段 P2|
|⑥|[05-scheduler-and-auto-update.md](05-scheduler-and-auto-update.md)|单一调度器、周期更新、退避|阶段 P3|
|⑦|[06-speedtest-engine.md](06-speedtest-engine.md)|基于内核 API 的并发延迟测速|阶段 P4|
|⑧|[07-auto-switch-policy.md](07-auto-switch-policy.md)|最低延迟自动切换 + 防抖策略|阶段 P4|
|⑨|[08-kernel-and-system-proxy.md](08-kernel-and-system-proxy.md)|mihomo 子进程管理、系统代理、托盘|阶段 P1|
|⑩|[09-ui-ux-and-tray.md](09-ui-ux-and-tray.md)|三页面的极简 UI 与交互细节|阶段 P5|
|⑪|[10-lightweight-and-performance.md](10-lightweight-and-performance.md)|轻量化硬指标与优化手段|贯穿全程 / P6|
|⑫|[11-packaging-and-release.md](11-packaging-and-release.md)|打包、图标、签名、Updater、许可证|阶段 P7|
|⑬|[12-testing-and-acceptance.md](12-testing-and-acceptance.md)|测试矩阵与验收清单|每阶段收尾|
|⑭|[13-iteration-task-cards.md](13-iteration-task-cards.md)|P0–P8 分阶段任务卡（可直接复制粘贴）|逐阶段执行|
|附录|[appendices/A-mihomo-api-and-config.md](appendices/A-mihomo-api-and-config.md)|内核 REST API 与必填配置字段|查表|
|附录|[appendices/B-subscription-userinfo.md](appendices/B-subscription-userinfo.md)|`subscription-userinfo` 规范与解析样本|查表|

\---

## 3\. 三种使用方式

### 方式 A：单文件模式（最省事）

一次性把全部章节拼成一个文件，作为一次超长上下文投喂：

```powershell
# 在仓库根目录执行，生成 prompt-pack.md
pwsh -File .\\scripts\\build-prompt-pack.ps1
```

然后用 `prompt-pack.md` 全文 + 一句「按 13 章任务卡从 P0 开始执行，每阶段结束停下来给我验收」即可。

### 方式 B：分阶段模式（推荐，稳定）

1. 新会话 → 贴 `01-master-prompt.md` 全文 + `appendices/C-conventions.md` 全文；
2. 再贴 `13-iteration-task-cards.md` 中**当前阶段**那一张卡；
3. 若该阶段涉及细节，追加对应专题章节（如 P4 追加 06、07 章）；
4. Agent 交付后，用 `12-testing-and-acceptance.md` 中该阶段的验收项逐条核对，通过再进下一阶段。

> 每个新会话都重复第 1 步。工程契约必须常驻，否则 Agent 会漂移。

### 方式 C：写入仓库（最佳）

把 `01` + `C-conventions` 落到代码仓库的 `AGENTS.md`，把专题章节放 `docs/dev-prompts/`，
让 Agent 在自己仓库里随时可读。此后只需一句「执行 docs/dev-prompts/13 里的 P3」。

\---

## 3.5 配套脚本（随提示词一起交付）

|脚本|用途|什么时候跑|
|-|-|-|
|[`scripts/build-prompt-pack.ps1`](../scripts/build-prompt-pack.ps1)|把 16 个章节按投喂顺序拼成单文件 `prompt-pack.md`，输出行数/字符数/预估 token；缺章直接报错|方式 A 投喂前|
|[`scripts/check-contract.ps1`](../scripts/check-contract.ps1)|**契约一致性校验**：从 C 章解析命令/事件/错误码白名单，扫描所有章节，报告"引用了契约未登记的名字"；同时检查章节齐全性、结构完整性、单文件包新鲜度|每次改完章节后；建议纳入 CI（13 章 P9 卡）|
|[`scripts/perf-probe.ps1`](../scripts/perf-probe.ps1)|只读采集 RSS(P95/峰值)/CPU/冷启动(5 次中位数)/磁盘/线程句柄，按 C.11 预算自动判定 PASS/FAIL，输出 `perf-report.md`|每个阶段收尾、P6 调优、P8 验收|

```powershell
# 典型用法（Windows PowerShell 5.1 或 PowerShell 7 均可）
powershell -NoProfile -ExecutionPolicy Bypass -File .\\scripts\\check-contract.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File .\\scripts\\build-prompt-pack.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File .\\scripts\\perf-probe.ps1 -AppName clash-zhengl
```

> `check-contract.ps1` 是这套提示词的\*\*防漂移装置\*\*。它把 C 章当唯一真源做交叉核对，
> 能拦住「某章自己发明了一个命令名」这类最难靠人眼发现、却必然导致代码对不上的错误。
> 退出码 0 = 一致，1 = 有硬错误。

\---

## 4\. 投喂纪律（重要）

1. **一次只做一个阶段**。让 Agent 一口气做完 P0–P8，结果一定是半成品。
2. **要求先给方案再写码**：每个阶段开头强制「先输出文件级改动清单与接口签名，我确认后再落盘」。
3. **每阶段结束跑门禁**：`cargo fmt --check` / `cargo clippy -D warnings` / `cargo test` / 构建通过，缺一不可。
4. **不要让它猜**：凡契约未定义处，要求它输出 `CONTRACT-CHANGE-REQUEST:` 而不是自行发明。
5. **不要让它联网抓内核/依赖当编译输入**：内核以运行时下载或捆绑方式处理，构建阶段不得依赖外网。
6. **性能指标必须实测**：拿 `scripts/perf-probe.ps1` 的数字说话，不接受「应该很轻」这类结论。

\---

## 5\. 一句话验收

> 装完后：双击 1 秒内出托盘；填 3 条订阅 10 秒内全部显示流量与到期日；
> 设 30 分钟自动更新 + 15 分钟自动测速后无人工干预持续工作 24 小时；
> 节点延迟变化后 1 个测速周期内自动切到最快节点且不抖动；
> 挂机 8 小时应用 RSS ≤ 90 MB、CPU 平均 ≤ 1.5%；卸载干净。

\---

## 6\. 套件规模与已知待定项

|项|数值|
|-|-|
|章节数|16（01 总纲 + 01 契约 + 11 专题 + 2 附录 + 本 README）|
|总行数|约 16,400 行|
|单文件包|约 16,260 行 / 约 39 万 token（中文粗估）|
|契约版本|C 章 v1.3|
|一致性校验|`check-contract.ps1` 全部通过（0 错误 0 告警）|

**已知待定项**：这些是产品决策而非技术缺陷，各章以 `TODO(待确认)` 显式标注，**未编造结论**：

|待定项|当前占位处理|影响面|处理方式|
|-|-|-|-|
|本软件自身许可证|暂按 MIT 占位|11 章 About 页、仓库 LICENSE|按MIT|
|是否购买代码签名证书|0.1.x 不签名|11 章 CI、C.11 的"签名后 setup.exe"口径|不签名|
|GitHub 仓库归属|未定|11 章 CI、内核下载源、updater|看GitHub信息|
|是否启用自动更新（updater）|MVP 不做（`update\\\_check` 不在 C.6）|11 章、13 章|不做|
|是否自绘标题栏|默认自绘|09 章|默认自绘|
|是否提供便携版|不做|11 章|是|
|最低 Windows 版本|Win10 1809|11 章、系统代理 API 行为|只需要支持win11即可|
|mihomo Release 是否附校验和文件|待实测|08 章下载校验、12 章测试|无需，只要确保软件可正常运行即可|
|`mihomo -t` 非法配置的退出码|待实测|04 章配置校验、12 章测试|无需，只要确保软件可正常运行即可|

> 开始开发前，建议先把上表里「影响面」较大的几项（许可证、签名、仓库归属）定下来，
> 否则 11 章的 CI 与发布流程需要返工。

