# 09 · 界面、交互与托盘（UI/UX & Tray）

> **适用阶段**：P5（UI 完整化）。但**骨架在 P1 就要能跑**，否则无法验收内核与代理。
> **前置阅读**：`01-master-prompt.md`、`appendices/C-conventions.md`（下称 C 章）、`08-kernel-and-system-proxy.md`
> **技术约束**：**原生 TypeScript + DOM，不引入 React / Vue / Svelte / Solid / 任何虚拟 DOM 运行时**
> （01 §4、§2.2）。打包后前端产物必须 ≤ 60 KB（gzip 前），否则视为超预算。
> **本章职责**：信息架构、组件树、状态管理、每页线框与交互细节、虚拟滚动、托盘菜单、通知、主题、
> DPI、快捷键。所有 IPC 名/事件名/字段严格取自 C.6/C.7/C.5，**不得发明**。

---

## 09.0 前端目录（对齐 C.2 的 `src/`）

```
src/
├─ main.ts                      # 入口：bootstrap → 挂载路由 → 订阅事件
├─ ipc/
│  ├─ types.ts                  # C.5 数据模型的手写 TS 镜像（唯一来源，禁止 any）
│  ├─ client.ts                 # invoke 封装 + AppError 归一
│  └─ events.ts                 # listen 封装 + 事件合并/节流
├─ store/
│  ├─ store.ts                  # 极简可订阅 store（无依赖，~80 行）
│  ├─ selectors.ts              # 派生数据（已用/剩余/天数/延迟色阶）
│  └─ actions.ts                # 所有写操作（唯一调 client 的地方）
├─ ui/
│  ├─ shell.ts                  # 侧栏 + 内容区 + 状态栏骨架
│  ├─ router.ts                 # hash 路由（3 页 + 设置）
│  ├─ components/
│  │  ├─ card.ts                # 卡片容器
│  │  ├─ button.ts              # 按钮（含 loading/禁用）
│  │  ├─ modal.ts               # 模态框（焦点陷阱）
│  │  ├─ toast.ts               # 轻提示
│  │  ├─ progress.ts            # 进度条（流量/测速/下载）
│  │  ├─ switch.ts              # 开关
│  │  ├─ empty.ts               # 空态
│  │  ├─ error-box.ts           # 错误态 + 重试
│  │  └─ virtual-list.ts        # 虚拟滚动（09.6）
│  ├─ pages/
│  │  ├─ subscriptions.ts       # 订阅页
│  │  ├─ traffic.ts             # 流量与到期页
│  │  ├─ nodes.ts               # 节点与测速页
│  │  └─ settings.ts            # 设置页
│  └─ styles/
│     ├─ tokens.css             # CSS 变量（主题）
│     ├─ base.css               # reset + 排版
│     └─ components.css         # 组件样式
└─ i18n/zh-CN.json              # C.1：i18n 键值化，预留 en-US
```

> **禁止**引入 UI 库（Element/Ant/Naive 等）与图标字体：包体预算（≤12 MB 安装包）与
> 启动预算（≤1.2s）都不允许。图标用**内联 SVG symbol**（单文件 `<svg><symbol>`，约 3 KB）。

---

## 09.1 信息架构

| 页面 | 路由 | 一句话职责 | 主要 IPC | 主要事件 |
| --- | --- | --- | --- | --- |
| 订阅 | `#/subs` | 管理多个订阅：增删改、启用、周期、立即更新 | `subscription_*` | `subscription:updated`、`subscription:progress` |
| 流量 | `#/traffic` | 看每个订阅用了多少、还剩多少、什么时候到期 | `traffic_summary` | `subscription:updated` |
| 节点 | `#/nodes` | 看节点延迟、手动选、发起测速、看自动切换记录 | `speedtest_*`、`switch_*`、`core_status` | `speedtest:progress`、`speedtest:done`、`switch:applied` |
| 设置 | `#/settings` | 全局参数（周期/测速/策略/内核/代理/关于） | `settings_*`、`core_*`、`sysproxy_set` | `core:state` |

**为什么只有三页 + 设置**：目标用户是「有 1~5 个订阅的普通用户」。Clash Verge 的
配置/规则/日志/连接四页对他们是负担。日志与诊断降级为**设置页底部 + 一个弹窗**。

### 09.1.1 顶层布局线框

```
┌───────────────────────────────────────────────────────────────────────┐
│  ● Clash Zhengl            [系统代理: 开 ▣]   [内核: 运行中]   [—][□][×]│  ← 顶栏(48px)
├──────────┬────────────────────────────────────────────────────────────┤
│ 订阅  3  │                                                            │
│ 流量     │                    内容区（虚拟滚动）                       │
│ 节点 128 │                                                            │
│ ──────── │                                                            │
│ 设置     │                                                            │
│          │                                                            │
│ ──────── │                                                            │
│ ↑ 1.2MB/s│                                                            │
│ ↓ 8.4MB/s│                                                            │
└──────────┴────────────────────────────────────────────────────────────┘
   侧栏 168px                                   状态栏（底部 28px，可选）
```

| 区域 | 高度 | 内容 | 刷新频率 |
| --- | --- | --- | --- |
| 顶栏 | 48px | Logo、系统代理开关、内核状态灯、窗口按钮 | 事件驱动 + 流量 2s |
| 侧栏 | 168px 固定 | 4 个导航项（含计数徽标）、底部实时速率 | `traffic:tick` 2s（隐藏时 10s） |
| 内容区 | flex | 按路由切换 | — |
| 状态栏 | 28px | 最近一次更新时间、订阅状态摘要 | 30s |

> 窗口默认 880×620，最小 720×520（`minWidth`/`minHeight` 写在 `tauri.conf.json`，见 11 章）。
> 顶栏右侧不使用系统标题栏（`decorations: false` + 自绘），因为要放代理开关；**必须**自绘
> 最小化/最大化/关闭三键并调用 `getCurrentWindow().minimize()` 等 API。

### 09.1.2 顶栏代理开关与内核灯的初值（v1.1）

两者都**不得**在前端维护独立状态，一律从 `app_bootstrap` 的 `runtime` 读（C.6 首屏一次性拉取）：

| 控件 | 初值来源 | 后续更新 | 点击 |
| --- | --- | --- | --- |
| 系统代理开关 | `runtime.sysproxy.enabled` | `sysproxy:state` 事件 | `sysproxy_set`，用**返回值**回填（C.6：返回 `SysProxyState`） |
| 开关文案 | `runtime.sysproxy.managed_by_us` | 同上 | 见 09.8.1 四态表 |
| 内核状态灯 | `runtime.core` | `core:state` 事件 | 点击 → 日志弹窗 |

```ts
// src/store/actions.ts
export function bindSysproxy() {
  // 初值：bootstrap 已放进 store，这里只渲染
  renderSysproxy(ui.get("runtime")?.sysproxy);

  // 变化：后端主动推（含外部程序改动被探测到，C.7）
  listen<SysProxyState>("sysproxy:state", (st) => {
    const rt = ui.get("runtime");
    if (rt) ui.set({ runtime: { ...rt, sysproxy: st } });
    else renderSysproxy(st);
  });
}

export function renderSysproxy(st: SysProxyState | undefined) {
  const el = document.querySelector<HTMLButtonElement>("#toggle-proxy")!;
  if (!st) { el.disabled = true; el.textContent = "系统代理"; return; }
  el.setAttribute("aria-checked", String(st.enabled));   // role="switch"
  el.textContent = st.enabled
    ? (st.managed_by_us ? "系统代理：开" : "系统代理：开（外部）")
    : "系统代理：关";
}

export async function toggleSysproxy() {
  const cur = ui.get("runtime")?.sysproxy?.enabled ?? false;
  // 只把「开/关」意图交给后端，实际结果以返回值为准（可能被拒绝）
  const st = await call<SysProxyState>("sysproxy_set", { enabled: !cur });
  const rt = ui.get("runtime");
  if (rt) ui.set({ runtime: { ...rt, sysproxy: st } });
  else renderSysproxy(st);
}
```

---

## 09.2 状态管理：极简 store（约 80 行，零依赖）

### 09.2.1 设计取舍

| 方案 | 包体 | 学习成本 | 是否采用 |
| --- | --- | --- | --- |
| 引入 Zustand/Redux | +12~30 KB | 中 | ❌ 违反"不引入运行时框架" |
| 原生 DOM + 事件委托 + 全量重渲染 | 0 | 低 | ❌ 列表 300+ 节点会卡 |
| **可订阅 store + 按 key 精准更新** | 0 | 低 | ✅ |

核心思想：**一个 store 持有全部状态；组件订阅自己关心的 key；只有 key 变化才重渲染**。

```ts
// src/store/store.ts
export type Listener<T> = (v: T, prev: T) => void;

export class Store<S extends Record<string, unknown>> {
  private state: S;
  private subs = new Map<keyof S, Set<Listener<unknown>>>();
  private raf = 0;
  /** 本帧待派发的 key 集合；跨多次 set() 累积，避免丢失先前变更 */
  private pending = new Set<keyof S>();
  /** 上一次派发出去的值，供 Listener 的 prev 参数使用 */
  private prev = new Map<keyof S, unknown>();

  constructor(initial: S) { this.state = initial; }

  get<K extends keyof S>(key: K): S[K] { return this.state[key]; }

  /** 支持批量 set：所有 key 合并到一帧内派发，避免事件风暴重渲染 */
  set(patch: Partial<S>): void {
    for (const k of Object.keys(patch) as Array<keyof S>) {
      const next = patch[k] as S[keyof S];
      if (Object.is(this.state[k], next)) continue;
      this.state[k] = next;
      this.pending.add(k);            // 累积，不覆盖
    }
    if (this.pending.size === 0 || this.raf) return;
    this.raf = requestAnimationFrame(() => {
      this.raf = 0;
      const keys = [...this.pending];
      this.pending.clear();
      for (const k of keys) this.emit(k);
    });
  }

  subscribe<K extends keyof S>(key: K, fn: Listener<S[K]>): () => void {
    let set = this.subs.get(key);
    if (!set) { set = new Set(); this.subs.set(key, set); }
    set.add(fn as Listener<unknown>);
    this.prev.set(key, this.state[key]);
    fn(this.state[key], this.state[key]);   // 立即回调一次，省去初始化样板
    return () => { set!.delete(fn as Listener<unknown>); };
  }

  private emit(k: keyof S): void {
    const set = this.subs.get(k);
    if (!set) return;
    // prev 是上一次派发出去的值，组件可据此做 diff（列表项不必关心）
    const prev = this.prev.get(k) as S[typeof k];
    this.prev.set(k, this.state[k]);
    for (const fn of set) {
      try { fn(this.state[k], prev as never); } catch (e) { console.error(e); }
    }
  }
}
```

```ts
// src/store/store.ts —— 全局单例
import type { AppSettings, Subscription, RuntimeState, SpeedtestRun, TrafficCounter } from "../ipc/types";

export interface UiState {
  settings: AppSettings | null;
  subs: Subscription[];
  runtime: RuntimeState | null;
  traffic: TrafficCounter | null;
  run: SpeedtestRun | null;         // 当前/最近一次测速
  delays: Map<string, number | null>; // nodeKey -> ms（null=超时）
  route: "subs" | "traffic" | "nodes" | "settings";
  busy: Set<string>;                 // 正在进行的操作（按钮 loading）
  toasts: Array<{ id: number; level: "info" | "warn" | "error"; text: string }>;
}

export const ui = new Store<UiState>({
  settings: null, subs: [], runtime: null, traffic: null, run: null,
  delays: new Map(), route: "subs", busy: new Set(), toasts: [],
});
```

### 09.2.2 渲染契约（禁止踩的坑）

| 规则 | 原因 |
| --- | --- |
| 组件 mount 时挂 `subscribe`，unmount 时调返回的取消函数 | 防内存泄漏；SPA 里切页必须解绑 |
| **列表项不订阅 store**，只订阅「列表数据」这一个 key | 否则 300 个节点 = 300 个订阅者 |
| `speedtest:progress` 只更新 `delays` Map，**不重排列表** | C.7 要求：不得因事件风暴重渲染 |
| 任何 `<input>` 的即时值不进 store | 每次按键都派发会让 IME 中文输入掉字 |
| `busy` 用 `Set` 且**克隆后**再 set | `Object.is` 判断引用，原地 mutate 不会触发 |

---

## 09.3 IPC 客户端与错误归一

```ts
// src/ipc/client.ts
import { invoke } from "@tauri-apps/api/core";

export interface AppError { code: string; message: string; detail?: string; retryable: boolean }

/** Rust 侧 AppError 序列化后由 Tauri 抛出，这里统一归一，视图层永远只处理 AppError */
export async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (raw) {
    const e = normalize(raw);
    if (e.code === "E_BUSY") return Promise.reject(e);  // 由调用方决定是否静默
    throw e;
  }
}

function normalize(raw: unknown): AppError {
  if (typeof raw === "object" && raw !== null && "code" in raw) {
    const r = raw as Partial<AppError>;
    return { code: String(r.code), message: String(r.message ?? "未知错误"),
             detail: r.detail, retryable: Boolean(r.retryable) };
  }
  // Tauri 的 invoke 失败可能是字符串（例如命令不存在）
  return { code: "E_UNSUPPORTED", message: String(raw ?? "调用失败"), retryable: false };
}
```

**错误展示规则（C.8 文案必须原样使用）**：

| code | 展示位置 | 附加操作 |
| --- | --- | --- |
| `E_NET_*` / `E_HTTP_STATUS` | 行内 `error-box` | 「重试」按钮 |
| `E_PARSE_YAML` / `E_PARSE_EMPTY` | 行内 | 「查看原始配置」（`profile_preview`） |
| `E_CORE_NOT_FOUND` | 全屏引导 | 「下载内核」（`core_install`） |
| `E_CORE_START_FAILED` | 顶栏红点 + toast | 「重启内核」+「查看日志」 |
| `E_CORE_PORT_EXHAUSTED` | toast | 「改为自动分配」（跳设置） |
| `E_STORE_DECRYPT` | 订阅行内 | 「重新填写地址」 |
| `E_PERM_ADMIN` | modal | 「以管理员重启」 |
| `E_BUSY` | 静默（按钮保持 loading） | 无 |

---

## 09.4 订阅页

### 09.4.1 线框

```
┌──────────────────────────────────────────────────────────────────────────┐
│  订阅        自动更新每 6 小时   [立即全部更新]      [+ 添加订阅]         │
├──────────────────────────────────────────────────────────────────────────┤
│ ┌──────────────────────────────────────────────────────────────────────┐ │
│ │ ● 机场A                    128 节点   6 小时   上次: 12 分钟前  [⋮]  │ │
│ │   https://api.example.com/sub?token=******        [显示]  ▣ 启用      │ │
│ │   状态: 正常          已用 42.3 GB / 200 GB      到期 2027-03-14      │ │
│ └──────────────────────────────────────────────────────────────────────┘ │
│ ┌──────────────────────────────────────────────────────────────────────┐ │
│ │ ○ 机场B  ⚠ 更新失败                 56 节点   12 小时  [⋮]           │ │
│ │   https://b.example.net/api/v1/client/subscribe?token=******          │ │
│ │   失败: 连接订阅服务器超时                    [重试]                  │ │
│ └──────────────────────────────────────────────────────────────────────┘ │
│                                                                          │
│   （空态时整块替换为引导卡，见 09.4.4）                                    │
└──────────────────────────────────────────────────────────────────────────┘
```

### 09.4.2 控件树

```
page-subscriptions
├─ header
│  ├─ h1 "订阅"
│  ├─ span.summary "自动更新每 {n} 小时"
│  ├─ button#refresh-all "立即全部更新"      → subscription_refresh_all
│  └─ button#add-sub "＋ 添加订阅"           → openAddModal()
├─ ul#sub-list                              → virtual-list（>20 条时启用）
│  └─ li.sub-card[data-uid]
│     ├─ div.sub-head
│     │  ├─ span.status-dot.ok|fail|never
│     │  ├─ span.sub-name （双击进入重命名）
│     │  ├─ span.sub-nodes "{nodeCount} 节点"
│     │  ├─ span.sub-interval "{interval} 分钟/小时"
│     │  ├─ span.sub-last "{相对时间}"
│     │  └─ button.menu-trigger "⋮"
│     ├─ div.sub-url
│     │  ├─ code.url-hint                    （url_hint，来自后端，已脱敏）
│     │  └─ button#reveal "显示"/"隐藏"      → subscription_reveal_url
│     ├─ div.sub-meta
│     │  ├─ span.used "{已用} / {总量}"
│     │  ├─ progress.usage-bar
│     │  └─ span.expire "{yyyy-MM-dd} · 剩 {n} 天"
│     ├─ label.switch  "启用"                → subscription_set_enabled
│     └─ div.sub-error （仅 Failed 时）       + button "重试"
└─ div.sub-empty （空态）
```

`⋮` 菜单项（**原生 `<div role="menu">`，不用第三方 popover**）：

| 菜单项 | IPC | 说明 |
| --- | --- | --- |
| 立即更新 | `subscription_refresh` | |
| 重命名 | `subscription_rename` | 行内变输入框 |
| 修改地址 | `subscription_update_url` | 弹窗，URL 空则只改元信息 |
| 更新周期 | `subscription_set_interval` | 下拉：15/30/60/180/360/720/1440 分钟 |
| 查看配置 | `profile_preview` | 只读 modal，节点凭据脱敏 |
| 删除 | `subscription_remove` | **二次确认**（09.4.5） |

### 09.4.3 添加订阅弹窗与校验

```
┌─ 添加订阅 ─────────────────────────────── ✕ ┐
│ 名称  [ 机场A                            ]  │
│ 地址  [ https://api.example.com/sub?tok… ]  │  ← type=password 外观 + 眼睛切换
│ 更新周期 [ 每 6 小时 ▾ ]                     │
│ ⓘ 地址含你的专属 token，仅以 DPAPI 加密保存在本机 │
│                              [取消] [添加]  │
└─────────────────────────────────────────────┘
```

前端校验（**必须在提交前做，减少一次无效往返**；后端仍须重复校验）：

```ts
// src/ui/pages/subscriptions.ts
export function validateUrl(raw: string): string | null {
  const s = raw.trim();
  if (!s) return "请填写订阅地址";
  let u: URL;
  try { u = new URL(s); } catch { return "地址格式不正确，需以 http:// 或 https:// 开头"; }
  if (u.protocol !== "https:" && u.protocol !== "http:") return "仅支持 http/https 订阅地址";
  if (u.hostname === "localhost" || u.hostname === "127.0.0.1") {
    return "不能是本地地址";                        // 防呆：本地没有机场
  }
  if (s.length > 4096) return "地址过长";
  return null;                                      // 通过
}

export async function submitAdd(name: string, url: string, intervalMin?: number) {
  const err = validateUrl(url);
  if (err) return showFieldError("url", err);
  if (!name.trim()) name = new URL(url).hostname;    // 名称留空 → 用域名兜底
  try {
    const sub = await call<Subscription>("subscription_add", { name: name.trim(), url: url.trim(), intervalMin });
    ui.set({ subs: [...ui.get("subs"), sub] });
    closeModal();
    // 后端规范：添加时立即拉取一次，失败也登记并标记 Failed（C.6 明文）
    if (sub.lastUpdateStatus === "Failed") {
      toast("warn", `订阅已添加，但首次拉取失败：${sub.lastError ?? "未知原因"}`);
    }
  } catch (e) {
    showModalError((e as AppError).message);         // 失败留在弹窗内，不关闭
  }
}
```

| 校验点 | 规则 | 失败文案 |
| --- | --- | --- |
| 非空 | 必填 | 请填写订阅地址 |
| 协议 | http/https | 仅支持 http/https 订阅地址 |
| 可解析 | `new URL()` 成功 | 地址格式不正确 |
| 长度 | ≤ 4096 | 地址过长 |
| 周期 | 15 ~ 10080 分钟（C.6 下限 15） | 更新周期不能小于 15 分钟 |
| 重名 | 允许（用 uid 区分） | — |

### 09.4.4 空态 / 加载态

```
┌────────────────────────────────────────────────────┐
│                       ( 图标 )                      │
│              还没有添加任何订阅                      │
│   订阅地址由你的机场提供，通常以 http(s):// 开头，    │
│   添加后会自动拉取节点，并显示剩余流量与到期日。      │
│                                                     │
│              [ ＋ 添加第一个订阅 ]                  │
│                                                     │
│   ⓘ 本软件不提供任何节点或代理服务。                 │
└────────────────────────────────────────────────────┘
```

| 状态 | 表现 |
| --- | --- |
| 首次加载 | 骨架屏（3 个灰色卡片，无 spinner，避免闪烁） |
| 无订阅 | 上面的空态卡（含合规提示，C.12.6） |
| 全部停用 | 「所有订阅已停用，节点不会被合并」+ 一键全部启用 |
| 刷新中 | 该行右侧显示细进度条 + 数字百分比（`subscription:progress`） |
| 刷新失败 | 行内红条 + 错误码文案 + 「重试」 |

> **合规提示的显示条件**（v1.1，C.5 `compliance_accepted_at`）：首次启动且
> `settings.compliance_accepted_at == null` 时，在空态卡**之前**弹一次不可跳过的模态
> （文案与流程图见 11 章 11.4）；接受后写回该字段，之后不再显示。
> 空态卡里的那句「本软件不提供任何节点或代理服务」是常驻提示，与合规模态是两件事，
> 不要因为「已经接受过」就把常驻提示也去掉。

### 09.4.5 删除二次确认

```
┌─ 删除订阅「机场A」？ ──────────────────────── ✕ ┐
│ 将删除该订阅的本地配置与流量记录，此操作不可恢复。 │
│ 已下载的 1 个配置文件将从本机移除。               │
│                                    [取消] [删除] │
└────────────────────────────────────────────────┘
```

销毁性按钮默认**不是**焦点元素；`Enter` 触发"取消"，`Esc` 关闭。
`subscription_remove` 执行期间禁用按钮（防连点），成功后从列表移除 + toast「已删除」。

---

## 09.5 流量页（流量与到期）

### 09.5.1 线框

```
┌──────────────────────────────────────────────────────────────────────────┐
│  流量与到期                              [刷新]   更新于 15:42            │
├──────────────────────────────────────────────────────────────────────────┤
│  ┌─ 汇总 ─────────────────────────────────────────────────────────────┐  │
│  │  已用 128.4 GB            剩余 71.6 GB           总量 200 GB       │  │
│  │  ████████████████████████░░░░░░░░░░░  64.2%                        │  │
│  │  最近到期 2026-12-01（25 天）      3 个订阅 · 2 个有效              │  │
│  └────────────────────────────────────────────────────────────────────┘  │
│                                                                          │
│  按订阅                                                                  │
│  ┌────────────────────────────────────────────────────────────────────┐  │
│  │ 机场A                                        2027-03-14 · 剩 129 天 │  │
│  │ ▓▓▓▓▓▓▓▓▓▓░░░░░░░░░░  42.3 / 200 GB                    21.2%      │  │
│  ├────────────────────────────────────────────────────────────────────┤  │
│  │ 机场B  ⚠ 流量信息不可用                    无限期                  │  │
│  │ ░░░░░░░░░░░░░░░░░░░░  — / —                                      │  │
│  │ "该订阅未提供流量信息"                        [查看网页]            │  │
│  ├────────────────────────────────────────────────────────────────────┤  │
│  │ 机场C                                        已过期 3 天（2026-11-03）│  │
│  │ ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓  500.0 / 500 GB  █ 已用完              100%  │  │
│  └────────────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────────────┘
```

### 09.5.2 控件树

```
page-traffic
├─ header
│  ├─ h1 "流量与到期"
│  ├─ button#traffic-refresh "刷新"        → traffic_summary → TrafficSummary
│  └─ span.updated-at "更新于 {HH:mm}"     ← TrafficSummary.at
├─ section.summary-card
│  ├─ div.kpi × 3   (已用 / 剩余 / 总量)
│  ├─ div.progress.usage-bar (role="progressbar" aria-valuenow)
│  └─ div.summary-foot (最近到期 / 订阅统计)
└─ section.per-sub
   ├─ h2 "按订阅"
   └─ ul#traffic-list
      └─ li.traffic-row[data-uid]
         ├─ div.row-head  (名称 + 到期徽标)
         ├─ div.progress.usage-bar[data-level]
         └─ div.row-foot (已用/总量 · 百分比 · 操作)
```

### 09.5.3 字段来源（**后端已算好，前端只格式化**，C.5 明文）

> **v1.1 变更**：C.5 已定义 `TrafficSummary` / `SubscriptionTraffic` / `AggregateTraffic`，
> 其中 `used` / `used_percent` / `days_left` 是**后端派生态**。前端**不得**再自行 `upload + download`，
> 否则与后端算法分叉（尤其 `total == 0` 与缺失字段的处理）。下表「来源」列说明每个数字取自哪里。

| 展示项 | 取值 | 来源 |
| --- | --- | --- |
| 单订阅已用 | `item.used` | `SubscriptionTraffic.used`（任一缺失即 `null`） |
| 单订阅百分比 | `item.used_percent` | `SubscriptionTraffic.used_percent`（`total > 0` 才有值） |
| 单订阅剩余天数 | `item.days_left` | `SubscriptionTraffic.days_left`（可为负 = 已过期） |
| 单订阅到期日 | `item.expire_at` | 只做日期格式化 |
| 汇总已用 | `agg.used` | `AggregateTraffic.used` |
| 汇总总量 | `agg.total` | `None` = 全部不限量 → 显示「不限量」 |
| 汇总最近到期 | `agg.expire_at` | 最早的非 `None` |
| 汇总百分比 | `agg.used_percent` | `None` 时不显示百分比 |
| 数据可能过期 | `item.stale` | `true` → 该行加 ⏱ 角标 |
| 聚合为下限 | `agg.has_missing` | `true` → 汇总卡注明「实际用量可能更高」 |

```ts
// src/store/selectors.ts —— 只做「格式化」与「文案映射」，不做数值推导
import type { SubscriptionTraffic, AggregateTraffic } from "../ipc/types";

export function fmtBytes(b: number | null | undefined): string {
  if (b == null || b < 0) return "—";
  if (b === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0, v = b;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${v.toFixed(2)} ${units[i]}`;      // C.10：1024 进制，2 位小数
}

/** 剩余字节：语义由后端 total 表达（0=不限量，None=未提供），这里只负责出文案 */
export function fmtRemain(item: SubscriptionTraffic): string {
  if (item.total == null) return "—";
  if (item.total === 0) return "不限量";                       // total===0 语义
  if (item.used == null) return "—";
  return fmtBytes(Math.max(0, item.total - item.used));
}

/** 到期文案：直接消费后端 days_left，不再自算 */
export function fmtExpireText(item: SubscriptionTraffic): {
  text: string; level: "normal" | "warn" | "bad";
} {
  if (item.expire_at == null) return { text: "无限期", level: "normal" };
  const date = fmtDate(item.expire_at);
  const d = item.days_left;
  if (d == null) return { text: date, level: "normal" };
  if (d < 0) return { text: `已过期 ${-d} 天（${date}）`, level: "bad" };
  if (d <= 3) return { text: `${date} · 剩 ${d} 天`, level: "bad" };
  if (d <= 7) return { text: `${date} · 剩 ${d} 天`, level: "warn" };
  return { text: `${date} · 剩 ${d} 天`, level: "normal" };
}

export function fmtDate(ts: number): string {
  const dt = new Date(ts * 1000);
  return `${dt.getFullYear()}-${String(dt.getMonth() + 1).padStart(2, "0")}-${String(dt.getDate()).padStart(2, "0")}`;
}

export function fmtPercent(p: number | null | undefined): string {
  return p == null ? "" : `${p.toFixed(1)}%`;                  // 无值就不显示，不写 0%
}

/** 聚合卡「总量」列：None 表示全部不限量 */
export function fmtAggTotal(agg: AggregateTraffic): string {
  return agg.total == null ? "不限量" : fmtBytes(agg.total);
}
```

| 情形 | 展示文案 | 进度条 |
| --- | --- | --- |
| `item.total === 0` | 已用 `item.used` / **不限量** | 满格斜纹 + 无百分比 |
| `item.total == null` | 「该订阅未提供流量信息」 | 置灰空条 |
| `item.used == null`（缺 upload 或 download） | `—` / `total` | 置灰 |
| `item.expire_at == null` | **无限期** | — |
| `item.days_left < 0` | `已过期 N 天`（红） | — |
| `item.days_left <= 3` | `剩 N 天`（橙）+ 通知（`notify_on_expire`） | — |
| `item.days_left <= 7` | `剩 N 天`（黄） | — |
| `item.used_percent >= traffic_warn_percent` | 进度条变橙/红 + 通知 | 变色 |
| `item.stale === true` | 行首 ⏱ 角标 + `title="数据距今已超过 2 个更新周期"` | 不置灰 |
| `agg.has_missing === true` | 汇总卡下方：「部分订阅未提供流量信息，实际用量可能更高」 | — |

### 09.5.4 空态与错误态

| 状态 | 文案 | 操作 |
| --- | --- | --- |
| 状态 | 文案 | 操作 |
| --- | --- | --- |
| 无订阅（`items` 为空） | 「还没有订阅，无法统计流量」 | 「去添加订阅」→ `#/subs` |
| 全部无 userinfo（所有 `items[].total/source` 均缺失） | 「这些订阅都没有提供流量信息，无法统计」+ 说明机场差异 | 「查看原始响应」（诊断弹窗） |
| `agg.has_missing` | 顶部黄条：「部分订阅未提供流量信息，汇总数字仅为下限」 | 「查看详情」 |
| 存在 `stale` 行 | 该行 ⏱ 角标 + 顶部提示「部分数据已过期」 | 「全部更新」 |
| 拉取失败 | 保留上一份数据 + 顶部黄条「数据可能不是最新」 | 「重试」 |
| 首次加载 | 骨架屏 | — |

---

## 09.6 节点与测速页

### 09.6.1 线框

```
┌──────────────────────────────────────────────────────────────────────────┐
│  节点与测速    128 个节点 · 121 可用     [开始测速]  ▣ 自动切换   [⚙ 策略]│
├──────────────────────────────────────────────────────────────────────────┤
│  测速中 68/128  ██████████████░░░░░░░░░░░░░░░░  53%      [取消]          │
├──────────────────────────────────────────────────────────────────────────┤
│  当前: 香港 01  ·  42 ms  ·  组「自动选择」                        (3 min)│
├──────────────────────────────────────────────────────────────────────────┤
│  🔒 已手动锁定「自动选择」，{n} 分钟内不会被自动切换        [立即解除锁定]  │
├──────────────────────────────────────────────────────────────────────────┤
│  ┃ 节点名称                    类型      组        延迟      操作         │
│  ────────────────────────────────────────────────────────────────────────│
│  ┃ 香港 01                     SS        自动选择   42 ms    [使用] ✓     │
│  ┃ 香港 02                     SS        自动选择   58 ms    [使用]       │
│  ┃ 日本 03                     VMess     自动选择  128 ms    [使用]       │
│  ┃ 新加坡 01                   Trojan    自动选择  356 ms    [使用]       │
│  ┃ 美国 07                     VLESS     自动选择  超时      [使用]       │
│  ┋ （虚拟滚动：只渲染可视区 + 上下各 5 行缓冲）                           │
└──────────────────────────────────────────────────────────────────────────┘
```

> 🟡 黄色「🔒 已手动锁定」条只在覆盖期内出现（C.5 `manual_override_min`），
> 见 09.6.9。

### 09.6.2 控件树

```
page-nodes
├─ header
│  ├─ h1 "节点与测速"
│  ├─ span.summary "{total} 个节点 · {ok} 可用"
│  ├─ button#speedtest-start "开始测速"      → speedtest_start
│  ├─ button#speedtest-cancel "取消"         → speedtest_cancel（测速中替换开始按钮）
│  ├─ label.switch "自动切换"                → settings_save(switch_policy.enabled)
│  └─ button#policy "策略"                   → 策略弹窗
├─ div.speedtest-bar
│  ├─ progress (finished/total)
│  ├─ span "测速中 {finished}/{total}"
│  └─ span.eta "约剩 {n}s"
├─ div.current-node
│  ├─ span "当前: {node}"
│  ├─ span.delay-badge[data-level]
│  └─ span.group "组「{group}」"
├─ div.manual-lock-bar   （仅覆盖期内存在，见 09.6.9）
│  ├─ span "🔒 已手动锁定「{group}」，{n} 分钟内不会被自动切换"
│  └─ button.unlock "立即解除锁定"
├─ div.list-head （列标题，sticky）
└─ div#node-list （virtual-list，itemHeight=44px）
   └─ div.node-row[data-key]
      ├─ span.name
      ├─ span.kind        (SS/VMess/Trojan/HY2/TUIC/WG)
      ├─ span.group
      ├─ span.delay-badge[data-level="good|ok|warn|bad|timeout"]
      └─ button.use "使用"  → switch_apply_node
```

### 09.6.3 延迟色阶（硬规定）

```ts
// src/store/selectors.ts
export type DelayLevel = "good" | "ok" | "warn" | "bad" | "timeout";

export function delayLevel(ms: number | null): DelayLevel {
  if (ms == null) return "timeout";
  if (ms <= 100) return "good";
  if (ms <= 200) return "ok";
  if (ms <= 400) return "warn";
  return "bad";
}
```

| 区间 | level | 文字色 | 背景色 | 文案 |
| --- | --- | --- | --- | --- |
| 0–100 ms | `good` | `var(--ok-fg)` `#0f9d58` | `var(--ok-bg)` `#e6f4ea` | `42 ms` |
| 100–200 ms | `ok` | `var(--info-fg)` `#1a73e8` | `#e8f0fe` | `128 ms` |
| 200–400 ms | `warn` | `var(--warn-fg)` `#b06000` | `#fef7e0` | `356 ms` |
| > 400 ms | `bad` | `var(--err-fg)` `#c5221f` | `#fce8e6` | `512 ms` |
| 超时/失败 | `timeout` | `#5f6368` | `#f1f3f4` | `超时` |
| 未测 | `timeout`（同灰） | `#5f6368` | 透明 | `未测` |

> 深色模式下**必须**换一套背景色（不透明度更低的同色相），见 09.9。
> 色觉障碍：`good/bad` 同时用「✓ / ✕」图标前缀，不只靠颜色区分（无障碍要求）。

### 09.6.4 测速进度合并策略（C.7 限流要求）

后端已把 `speedtest:progress` 合并到 ≤4 条/秒，前端仍须二次防护：

```ts
// src/ipc/events.ts
let pending = new Map<string, number | null>();
let flushTimer = 0;

export function initSpeedtestEvents() {
  listen<{ runId: string; finished: number; total: number; latest: ProbeResult }>(
    "speedtest:progress", ({ latest, finished, total }) => {
      pending.set(latest.nodeKey, latest.delayMs);
      flushTimer ||= window.setTimeout(() => {
        flushTimer = 0;
        const delays = new Map(ui.get("delays"));
        for (const [k, v] of pending) delays.set(k, v);
        pending = new Map();
        const run = ui.get("run");
        if (run) ui.set({ delays, run: { ...run, finished, total } });
        else ui.set({ delays });
        // 注意：只更新 Map 与计数，**不重排列表、不重建行**
      }, 250);
    });

  listen<SpeedtestRun>("speedtest:done", (run) => {
    ui.set({ run });
    toast("info", `测速完成：${run.results.filter(r => r.delayMs != null).length}/${run.total} 可用`);
    // 测速完成后按策略自动应用（是否真的切由后端判定；这里只负责触发）
    void applyBest(run.id);
  });
}

/**
 * v1.2：`switch_apply_best` 返回 `{primary, all}`（C.6）。
 * primary = 主组（用于提示文案），all = 全部实际生效的组（scope=Group 时长度 1）。
 * 前端**不**自行推断切了哪些组，一律以后端返回为准。
 */
async function applyBest(runId: string) {
  const policy = ui.get("settings")?.switchPolicy;
  if (!policy?.enabled) return;                       // 总开关关闭则不自动应用
  const { primary, all } = await call<{ primary: SwitchRecord; all: SwitchRecord[] }>(
    "switch_apply_best", { runId },
  );
  if (all.length === 0) {
    toast("info", "当前已是最优节点，未切换");
  } else if (all.length === 1) {
    toast("info", `已切换到 ${primary.to_node}`);
  } else {
    // 多组同时切换时只报主组，其余写日志（避免 toast 刷屏）
    toast("info", `已切换 ${all.length} 个策略组（主组：${primary.to_node}）`);
  }
  // 列表与「当前节点」的刷新走 switch:applied 事件，这里不手改 DOM
}
```

> **`primary` 与 `all` 的用途分工**：`primary` 用于 toast 文案（用户只需要知道"切到哪了"）；
> `all` 用于「切换影响面」类展示与日志（例如策略弹窗调试视图列出所有被改的组）。
> `scope = Group(name)` 时 `all.len() == 1`，此时 UI **不显示**"N 个策略组"字样。

### 09.6.5 虚拟滚动实现（>300 节点，纯手写 ~70 行）

固定行高 44px，只渲染可视区 ± 5 行。**不使用 IntersectionObserver**（节点多时回调成本更高）。

```ts
// src/ui/components/virtual-list.ts
export interface VirtualListOptions<T> {
  container: HTMLElement;          // 固定高度、overflow:auto 的滚动容器
  itemHeight: number;              // 44
  overscan?: number;               // 5
  render: (item: T, index: number) => HTMLElement;
  keyOf: (item: T) => string;
}

export class VirtualList<T> {
  private items: T[] = [];
  private scroller: HTMLElement;   // 内部撑高的层
  private viewport: HTMLElement;   // 绝对定位的行容器
  private ro: ResizeObserver;
  private raf = 0;

  constructor(private opts: VirtualListOptions<T>) {
    const { container, itemHeight } = opts;
    container.style.overflowY = "auto";
    container.style.position = "relative";
    container.style.contain = "strict";        // 隔离布局/绘制，帧率更稳

    this.scroller = document.createElement("div");
    this.scroller.style.position = "relative";
    this.scroller.style.height = "0px";        // 由 setItems 撑开

    this.viewport = document.createElement("div");
    this.viewport.style.position = "absolute";
    this.viewport.style.inset = "0 0 auto 0";
    this.scroller.appendChild(this.viewport);
    container.appendChild(this.scroller);

    container.addEventListener("scroll", () => this.schedule(), { passive: true });
    this.ro = new ResizeObserver(() => this.schedule());
    this.ro.observe(container);
    void itemHeight;
  }

  setItems(items: T[]): void {
    this.items = items;
    this.scroller.style.height = `${items.length * this.opts.itemHeight}px`;
    this.renderWindow(true);
  }

  /** 局部更新某一行（测速结果回来时用，避免整表重绘） */
  updateItem(key: string): void {
    const el = this.viewport.querySelector<HTMLElement>(`[data-key="${CSS.escape(key)}"]`);
    if (!el) return;                             // 不在可视区，无需处理
    const idx = this.items.findIndex((it) => this.opts.keyOf(it) === key);
    if (idx < 0) return;
    const fresh = this.opts.render(this.items[idx], idx);
    fresh.dataset.key = key;
    el.replaceWith(fresh);
  }

  private schedule(): void {
    if (this.raf) return;
    this.raf = requestAnimationFrame(() => { this.raf = 0; this.renderWindow(false); });
  }

  private renderWindow(force: boolean): void {
    const { itemHeight, overscan = 5 } = this.opts;
    const scrollTop = this.opts.container.scrollTop;
    const viewH = this.opts.container.clientHeight || 400;
    const start = Math.max(0, Math.floor(scrollTop / itemHeight) - overscan);
    const end = Math.min(this.items.length, Math.ceil((scrollTop + viewH) / itemHeight) + overscan);

    // 快速路径：滚动不足一行时不重建
    if (!force && this.viewport.dataset.range === `${start}-${end}`) {
      this.viewport.style.transform = `translateY(${start * itemHeight}px)`;
      return;
    }
    this.viewport.dataset.range = `${start}-${end}`;
    this.viewport.style.transform = `translateY(${start * itemHeight}px)`;

    const frag = document.createDocumentFragment();
    for (let i = start; i < end; i++) {
      const el = this.opts.render(this.items[i], i);
      el.dataset.key = this.opts.keyOf(this.items[i]);
      el.style.height = `${itemHeight}px`;
      frag.appendChild(el);
    }
    this.viewport.replaceChildren(frag);
  }

  destroy(): void { this.ro.disconnect(); }
}
```

| 参数 | 值 | 依据 |
| --- | --- | --- |
| `itemHeight` | 44 px | 行内两行文本 + 徽标 |
| `overscan` | 5 | 快速滚动时留白不露底 |
| 启用阈值 | 节点数 > 40 | 40 以下直接全量渲染更简单 |
| 单帧成本 | 约 20 行 DOM 创建 | `contain: strict` 后实测 <2 ms |

**列表排序规则**（后端 `speedtest/rank.rs` 已排序，前端**不重排**，仅在无 `delays` 时按原序）：
可用（按延迟升序）→ 未测 → 超时。切换策略细节见 07 章。

### 09.6.6 空态 / 错误态

| 状态 | 文案 | 操作 |
| --- | --- | --- |
| 无内核 | 「内核未运行，无法测速」 | 「启动内核」→ `core_start` |
| 无节点 | 「订阅中没有可用节点」 | 「更新订阅」 |
| 全部超时 | 「全部节点探测超时，请检查网络或订阅」 | 「重新测速」 |
| 测速中 | 行内逐行刷新 + 顶部汇总 | 「取消」 |
| 自动切换关闭 | 顶部黄条「自动切换已关闭，仅显示延迟」 | 「开启」 |

### 09.6.7 最近切换（`switch_history`，v1.1 新增命令）

节点页底部一个可折叠区，展示最近 N 次切换。数据源是 **C.6 的 `switch_history`**
（入参 `{limit?}`，默认 50、上限 200），不再用 `logs_tail` 兜底。

```
┌─ 最近切换 ────────────────────────────────────────── [展开/收起] ┐
│ 15:42  自动选择   香港 02 → 香港 01   58ms → 42ms   Best        │
│ 15:12  自动选择   日本 03 → 香港 02  128ms → 58ms   Best        │
│ 14:58  自动选择   香港 01 → 日本 03   超时 → 128ms  CurrentFailed│
└──────────────────────────────────────────────────────────────────┘
```

```ts
// src/ui/pages/nodes.ts
import type { SwitchRecord } from "../ipc/types";

export async function loadSwitchHistory(limit = 50): Promise<SwitchRecord[]> {
  const rows = await call<SwitchRecord[]>("switch_history", { limit: Math.min(limit, 200) });
  return rows.slice().sort((a, b) => b.at - a.at);   // 契约已保证倒序，这里防御性再排一次
}

/** reason → 中文（C.5 SwitchReason 四值，一一对应，不得自增） */
const REASON_TEXT: Record<SwitchRecord["reason"], string> = {
  Best: "延迟更优",
  CurrentFailed: "当前节点失败",
  Manual: "手动切换",
  GroupMissing: "原组已失效",
};

/** 延迟列：两端都可能是 null，一律显示 — 或「超时」，不得写 0ms（C.5 to_delay 注释） */
export function fmtDelayPair(rec: SwitchRecord): string {
  const f = rec.from_delay == null ? "—" : `${rec.from_delay}ms`;
  const t = rec.to_delay == null ? "—" : `${rec.to_delay}ms`;   // v1.2：to_delay 变 Option
  return `${f} → ${t}`;
}
```

> **v1.2 变更：`SwitchRecord.to_delay` 改为 `Option<u32>`**。原因：`reason: Manual`
> 的切换（用户点「使用」）**可能根本没有新鲜测速数据**，此时延迟未知。
> 契约明文「不得用 0 冒充」，因此渲染时必须显示 `—`，不能显示 `0ms`——
> 否则用户会以为这个节点测出来是 0 延迟。

| 字段 | 可能为 `null` | 渲染 |
| --- | --- | --- |
| `from_node` | 是（首次选择时无前任） | `—` |
| `from_delay` | 是 | `—` |
| `to_delay` | **是**（v1.2，尤其 `Manual`） | `—` |
| `reason` | 否（四值枚举） | `REASON_TEXT[reason]` |

| 规则 | 说明 |
| --- | --- |
| 刷新时机 | 页面进入 + 收到 `switch:applied` 事件后**插入首行**（不整表重拉） |
| 落盘 | 后端写 `%LOCALAPPDATA%\ClashZhengl\runtime\switch-history.json`（C.3，最近 200 条滚动丢弃） |
| `Manual` 记录 | 用户点「使用」也产生一条，`reason: Manual`；因此历史里既有自动也有手动 |
| 空态 | 「还没有切换记录」+ 说明「开启自动切换后，这里会显示每次切换的原因」 |
| 可解释性 | 01 §3.7 要求「为什么切、从哪切到哪、当时延迟多少」——这张表就是答案，**四列必须全显示**（缺失显 `—`） |

### 09.6.8 哪些组会被自动切换（C.5 组类型约束）

只有 `type: select` 的组可被本应用自动切换（`PUT /proxies/{group}` 只对 selector 有意义）。
04 章生成合并配置时**必须**把 `switch_policy.scope` 指向的组生成为 `select`，
禁止同时生成同名 `url-test` / `fallback` 组。

| 规则 | 说明 |
| --- | --- |
| 可自动切换的组 | `switch_policy.scope` 指定的那个组（`Group(name)` 或全局主组） |
| 不可自动切换 | mihomo 内置 `url-test` / `fallback` 组由内核自己选，本应用不干预 |
| 节点页「使用」按钮 | 只对 `select` 组成员的节点可用；其它组内节点灰掉并提示「该组由内核自动选择」 |
| 若配置不合规 | 04 章已阻止生成；万一运行时发现组不是 `select`，记 `WARN` 并在节点页顶部提示一次，不尝试强改 |

> 这条约束的实际意义：如果同时存在 `select` 与同名 `url-test`，内核的健康检查会与
> 本应用的 `rank.rs` 打架——两边都在切，用户看到节点频繁跳动且历史记录混乱。

### 09.6.9 手动覆盖期提示（`manual_override_min`，v1.2 新增 UI）

用户点「使用」手动选节点后，该组在 `settings.switch_policy.manual_override_min` 分钟内
**不会被自动切换**（C.5 手动覆盖语义）。这是本版唯一需要新 UI 的地方。

```ts
// src/ui/pages/nodes.ts —— 覆盖期完全由前端本地计时，后端不发状态
export interface ManualLock { group: string; lockedAt: number; ttlMin: number }

let lock: ManualLock | null = null;
let lockTimer = 0;

/** switch_apply_node 成功后调用（这是唯一进入覆盖期的入口） */
export function onManualSelect(group: string) {
  const ttlMin = ui.get("settings")?.switchPolicy.manualOverrideMin ?? 60;
  if (ttlMin === 0) return;                     // 0 = 不锁定，也不显示提示条
  lock = { group, lockedAt: Date.now() / 1000, ttlMin };
  renderLockBar();
  scheduleLockTick();
}

function remainSec(): number {
  if (!lock) return 0;
  return Math.max(0, lock.lockedAt + lock.ttlMin * 60 - Date.now() / 1000);
}

function renderLockBar() {
  const bar = document.querySelector<HTMLElement>(".manual-lock-bar");
  if (!bar) return;
  const s = remainSec();
  if (!lock || s <= 0) { bar.remove(); lock = null; return; }
  const n = Math.ceil(s / 60);
  // 只改文本，不重建节点，避免打断滚动
  bar.querySelector(".lock-text")!.textContent =
    `🔒 已手动锁定「${lock.group}」，${n} 分钟内不会被自动切换`;
}

function scheduleLockTick() {
  clearTimeout(lockTimer);
  // 粒度 1s 只为「解除瞬间隐藏」，不触发列表重绘
  lockTimer = window.setTimeout(() => {
    if (!lock) return;
    if (remainSec() <= 0) { lock = null; renderLockBar(); return; }
    renderLockBar();
    scheduleLockTick();
  }, 1000);
}

export function unlockNow() {
  // 「立即解除锁定」= 清本地计时 + 立刻重跑一次排序（后端到期判定是独立的，
  // 因此这里同时把 local 视图松开，下一次测速即恢复自动切换）
  lock = null;
  renderLockBar();
  void ipc.invoke("speedtest_start", { trigger: "Manual" });
}
```

| 场景 | 表现 |
| --- | --- |
| `manual_override_min = 0` | 不显示锁定条；用户选了就可能被立刻切走（策略弹窗里有说明） |
| 覆盖期内用户再选一次 | **续期**：`lockedAt` 重置为当前时间（C.5「再次手动选择续期」） |
| 覆盖期结束 | 锁定条自动消失，无需用户操作；下一次定时测速恢复参与自动切换 |
| 应用重启 | 覆盖期是内存态（C.5），重启后清空 → 锁定条不出现，自动策略立即生效 |
| 与「自动切换」总开关的关系 | 总开关关闭时依然显示锁定条（因为锁定条说明的是「即使开着也不会切」） |
| 多组场景 | `SwitchRuntimeLocks` 是 per-group 的；本 MVP 只有一个 scope 组，UI 只渲染一条 |
| 解除锁定按钮 | 「立即解除锁定」→ 清本地计时 + 手动触发一次测速，让自动策略马上重新评估 |

> **为什么锁定状态不进 `RuntimeState`**（C.5 明文）：`traffic:tick` 每 2s 推送，
> 若把覆盖期塞进去会让每帧负载携带无关字段。前端本地计时足够——它是纯 UI 提示，
> 后端自己按 `locked_at` 独立判定，两边不一致也不影响正确性。

### 09.6.10 策略弹窗（`settings.switch_policy` 的可视化）

| 字段 | 控件 | 默认 | 说明文案 |
| --- | --- | --- | --- |
| `enabled` | 开关 | true | 测速后自动切到最低延迟节点 |
| `tolerance_ms` | 数字 0–500 | 30 | 新节点要比当前快这么多才切（防抖） |
| `min_dwell_sec` | 数字 0–3600 | 300 | 切换后至少多久不再切 |
| `failure_threshold` | 数字 1–10 | 2 | 连续失败几次才认为当前节点挂了 |
| `exclude_keywords` | 标签输入 | 8 个默认词 | 匹配到的节点不参与自动切换 |
| `prefer_region_priority` | 有序标签 | 空 | 同延迟档内优先的地域 |
| `manual_override_min` | 数字 0–1440 | 60 | **手动选节点后，多久内不再自动切换**（0=不锁定）；见 09.6.9 |

---

## 09.7 设置页

| 分组 | 项 | 控件 | 生效方式（08.11） |
| --- | --- | --- | --- |
| 常规 | 主题 | 三态（跟随系统/浅色/深色） | 立即 |
| 常规 | 开机自启 | 开关 | 立即（HKCU Run） |
| 常规 | 启动时最小化到托盘 | 开关 | 立即 |
| 常规 | 关闭窗口时最小化到托盘 | 开关（默认 true） | 立即 |
| 订阅 | 新订阅默认周期 | 下拉 | 立即 |
| 订阅 | 拉取并发 | 1–4 | 立即 |
| 订阅 | 拉取超时 | 5–60s | 立即 |
| 订阅 | 拉取代理 | 输入（可空） | 立即 |
| 订阅 | User-Agent | 输入 | 立即 |
| 测速 | 启用定时测速 | 开关 | 立即 |
| 测速 | 间隔 | 5–180 分钟 | 立即 |
| 测速 | 探测 URL | 输入 | 立即 |
| 测速 | 超时 | 1000–10000 ms | 立即 |
| 测速 | 并发 | 1–16 | 立即 |
| 测速 | **手动锁定分钟数** | 数字 0–1440（默认 60，0=不锁定）→ `switch_policy.manual_override_min` | 立即（下次手动选节点起生效） |
| 内核 | 当前版本 | 下拉 → `core_versions_list`（`installed` + `available`，见 08.1.2） | 需重启内核 |
| 内核 | 重新安装/修复 | 按钮 → `core_install`（进度看 `core:install-progress`） | — |
| 内核 | API 端口 | 数字（0=自动） | **需重启内核** |
| 代理 | 代理模式 | 系统代理 / TUN / 手动 | **需重启内核** |
| 代理 | 系统代理开关 | 开关 → `sysproxy_set`，初值取 `runtime.sysproxy.enabled` | 立即 |
| 通知 | 切换时通知 | 开关（**默认 false**） | 立即 |
| 通知 | 到期提醒 | 开关（默认 true） | 立即 |
| 通知 | 流量预警阈值 | 50–95%（0=关） | 立即 |
| 高级 | 日志级别 | 下拉 | 立即（reload handle） |
| 高级 | 查看日志 | 按钮 → 弹窗 → `logs_tail` | — |
| 高级 | 导出诊断包 | 按钮 → `diagnostics_export` | — |
| 关于 | 版本/内核版本/许可证/免责声明 | 只读 | 见 11 章 |

> 设置页所有项**即时保存**（`settings_save` 单字段 patch），不做「保存」按钮——
> 减少一次误操作；但需 300ms 防抖合并连续输入。

---

## 09.8 托盘（Tauri 2 `TrayIconBuilder` + `Menu`）

### 09.8.1 菜单结构表

| id | 菜单项 | 类型 | 启用条件 | 点击行为 |
| --- | --- | --- | --- | --- |
| `show` | 显示主界面 | 普通 | 恒可用 | 显示并聚焦主窗口（默认左键行为） |
| `toggle_proxy` | 系统代理：已开启 / 已关闭 | **勾选** | 内核 Running | `sysproxy_set{enabled:!current}`，见下方语义表 |
| `node_root` | 当前节点 | 子菜单 | 内核 Running 且 ≥1 组 | 展开 `node:<g>:<n>` 列表 |
| `node:<group>:<name>` | 动态节点项 | 勾选（当前选中打勾） | — | `switch_apply_node`，触发手动覆盖期（见下） |
| `node_more` | 更多节点… | 普通 | 节点数 > 15 | 打开主窗口节点页 |
| `update_all` | 立即更新订阅 | 普通 | ≥1 个启用订阅 | `subscription_refresh_all` |
| `speedtest` | 立即测速 | 普通 | 内核 Running | `speedtest_start{trigger:"Manual"}` |
| `—` | 分隔符 | — | — | — |
| `open_logs` | 打开日志目录 | 普通 | 恒可用 | `ShellExecute(open, logs dir)` |
| `about` | 关于 | 普通 | 恒可用 | 打开设置页的「关于」 |
| `quit` | 退出 | 普通 | 恒可用 | 走 08.8 退出序列 |

**托盘动态性约束**：节点子菜单**最多列 15 项**（超出给「更多节点…」）。原因：Windows 托盘菜单
超过约 20 项会出现滚动箭头，体验差；且每次重建菜单都要 `Menu::new()`，属可感知开销。

#### `toggle_proxy` 的勾选与文案（v1.1：由 `SysProxyState` 驱动）

菜单项的勾选态与文案**全部来自 `RuntimeState.sysproxy`**（C.5），不靠本地缓存猜：

| `sysproxy.enabled` | `sysproxy.managed_by_us` | 勾选 | 菜单文案 | 点击后行为 |
| --- | --- | --- | --- | --- |
| `false` | `false` | 不勾 | 系统代理：已关闭 | 开启（写 journal + 写注册表 + 刷新） |
| `true` | `true` | 勾 | 系统代理：已开启（本应用） | 关闭并**恢复原设置** |
| `true` | `false` | 勾 | 系统代理：已开启（非本应用） | **先弹确认**：「当前代理不是本应用设置的，关闭会改掉你原有的配置，继续？」 |
| 任意 | 任意 | 禁用 | 系统代理（内核未运行） | 不可点，`enabled=false` 且内核非 Running |

> 第 3 行是最容易被忽略的态：用户原本就有系统代理（公司 PAC、其他客户端），我们**没有**接管它。
> 此时点击必须是「询问 + 写回我们没记录过的值」而不是「静默恢复」。这正是 C.5 把
> `managed_by_us` 单独拆出来的原因。

**托盘重建时机**：`core:state`、`sysproxy:state`、`switch:applied`、订阅列表变化、语言变化。
其中 `sysproxy:state` 是 v1.1 新增——之前只能靠 `core:state` 顺带刷新，现在外部程序改动也能同步。

#### 托盘里「手动选节点」与覆盖期（v1.2）

从托盘选节点和从主界面点「使用」是**同一条路径**：都调 `switch_apply_node`，
都会让该组进入手动覆盖期。托盘侧只需一个不含状态的轻提示：

```rust
// system/tray.rs —— 节点菜单点击
"node" => {
    let (group, node) = split_node_id(&id)?;
    let rec: SwitchRecord = api.switch_apply_node(&group, &node).await?;   // C.6 返回 SwitchRecord
    // 托盘不维护锁定状态（那是内存态、在 speedtest/selector.rs），
    // 只根据设置值给出一次性气泡提示，避免用户以为「刚选完就被切走」是 bug
    let ttl = settings.read().switch_policy.manual_override_min;
    if ttl > 0 {
        notify::toast(&app, &format!("已锁定「{group}」{ttl} 分钟，期间不会自动切换"));
    }
    // 勾选态刷新走 switch:applied 事件（逐条广播），不在点击处手改菜单
}
```

| 要点 | 说明 |
| --- | --- |
| 提示形式 | 托盘**气泡/通知**，不是菜单文本；菜单文本只显示节点名与勾 |
| `ttl == 0` | 不提示（用户明确关掉了锁定语义） |
| 勾选刷新 | 依赖 `switch:applied` 事件重建菜单，**不**在点击回调里手动改勾选 |
| 与 `switch_apply_best` 的区别 | 托盘从不调 `switch_apply_best`（那是「一键最优」，属主界面动作） |

### 09.8.2 代码结构（`system/tray.rs`）

```rust
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Wry,
};
use tauri::image::Image;

pub fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let show      = MenuItem::with_id(app, "show", "显示主界面", true, None::<&str>)?;
    let proxy     = CheckMenuItem::with_id(app, "toggle_proxy", "系统代理", true, false, None::<&str>)?;
    let update    = MenuItem::with_id(app, "update_all", "立即更新订阅", true, None::<&str>)?;
    let test      = MenuItem::with_id(app, "speedtest", "立即测速", true, None::<&str>)?;
    let logs      = MenuItem::with_id(app, "open_logs", "打开日志目录", true, None::<&str>)?;
    let about     = MenuItem::with_id(app, "about", "关于 Clash Zhengl", true, None::<&str>)?;
    let quit      = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let sep1      = PredefinedMenuItem::separator(app)?;
    let sep2      = PredefinedMenuItem::separator(app)?;

    // 节点子菜单每 5s 或 core:state / switch:applied 时整体重建（见 rebuild_nodes）
    let nodes = Submenu::with_id(app, "node_root", "当前节点", true)?;

    let menu = Menu::with_items(
        app,
        &[&show, &proxy, &sep1, &nodes, &update, &test, &sep2, &logs, &about, &sep2, &quit],
    )?;

    let _tray = TrayIconBuilder::with_id("main")
        .icon(Image::from_bytes(include_bytes!("../../icons/tray-32.png"))?)
        .tooltip("Clash Zhengl")
        .menu(&menu)
        .show_menu_on_left_click(false)          // 左键=显示窗口，右键=菜单（符合 Windows 习惯）
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main(app),
            "quit" => crate::app::request_exit(app.clone()),
            "about" => { show_main(app); crate::ipc::emit_route(app, "settings#about"); }
            id if id.starts_with("node:") => crate::commands::speedtest::apply_node_from_tray(app, id),
            other => tracing::warn!(target = "tray", "unhandled menu id={}", other),
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}
```

| 项 | 要求 |
| --- | --- |
| 图标尺寸 | 16×16 @1x、32×32 @2x（多尺寸 `.ico`，见 11 章）；托盘用 32 PNG 由 Tauri 缩放 |
| tooltip | `Clash Zhengl · {CoreState 文案}`；测速时追加 `· 测速中 68%`（≤60 字符） |
| 状态可视化 | 内核 Failed → 图标加红色角标图（`tray-err.png`）；系统代理关闭 → 图标降低不透明度 |
| 菜单重建时机 | `core:state` 变化、`switch:applied`、订阅列表变化、语言变化 |
| 重建开销 | 全量重建约 2–5 ms；禁止在 `traffic:tick`（2s）里重建 |
| 双击托盘 | 显示主界面（与左键一致） |

### 09.8.3 托盘与窗口的交互约定

| 用户动作 | 行为 |
| --- | --- |
| 点关闭按钮 `×` | `close_to_tray=true`（默认）→ 隐藏窗口，**首次**弹一次性 toast「应用已最小化到托盘」；`false` → 退出应用 |
| 双击托盘图标 | 显示窗口（不切换显示/隐藏，避免误操作） |
| 右键托盘 | 打开菜单 |
| Alt+F4 | 同关闭按钮 |
| 任务栏图标 | 隐藏窗口后从任务栏消失（`skipTaskbar` 需动态调用） |

---

## 09.9 主题（CSS 变量）与深色模式

```css
/* src/ui/styles/tokens.css */
:root {
  /* 语义色 —— 组件只许用语义色，禁止硬编码 hex */
  --bg:            #ffffff;
  --bg-sub:        #f6f7f9;
  --bg-elevated:   #ffffff;
  --fg:            #1f2328;
  --fg-muted:      #5f6368;
  --border:        #e3e5e8;
  --accent:        #2f6feb;
  --accent-fg:     #ffffff;
  --ok-fg:  #0f9d58;  --ok-bg:  #e6f4ea;
  --info-fg:#1a73e8;  --info-bg:#e8f0fe;
  --warn-fg:#b06000;  --warn-bg:#fef7e0;
  --err-fg: #c5221f;  --err-bg: #fce8e6;
  --neutral-bg:#f1f3f4;

  /* 尺寸与节奏 */
  --radius: 8px;
  --radius-sm: 6px;
  --sp-1: 4px; --sp-2: 8px; --sp-3: 12px; --sp-4: 16px; --sp-5: 24px;
  --row-h: 44px;
  --font: "Microsoft YaHei UI", "Segoe UI", system-ui, -apple-system, sans-serif;
  --font-mono: "Cascadia Mono", "Consolas", monospace;
  --dur: 120ms;                 /* 动画时长：短，避免"重客户端"观感 */
}

/* 深色：跟随系统（默认） */
@media (prefers-color-scheme: dark) {
  :root[data-theme="system"] { --bg:#1b1d1f; --bg-sub:#212427; --bg-elevated:#26292c;
    --fg:#e6e8ea; --fg-muted:#9aa0a6; --border:#34383c; --accent:#4b8bf5;
    --ok-fg:#5cc98f; --ok-bg:#16301f; --info-fg:#79aef7; --info-bg:#16233a;
    --warn-fg:#f0b354; --warn-bg:#33280f; --err-fg:#f28b82; --err-bg:#3a1d1b;
    --neutral-bg:#2b2f33; }
}
/* 手动选择深色 */
:root[data-theme="dark"] { /* 同上，抽为 @mixin 或重复一次 */ }
:root[data-theme="light"] { /* 即 :root 默认值 */ }
```

| 规则 | 说明 |
| --- | --- |
| 主题切换 | `document.documentElement.dataset.theme = settings.theme.toLowerCase()` |
| 跟随系统 | `theme === "System"` 时 `dataset.theme = "system"`，靠媒体查询生效 |
| 无闪烁 | 在 `index.html` 的 `<head>` 内联 8 行脚本，读 `localStorage['theme-cache']` 提前设 `data-theme` |
| 避免大面积白闪 | `body { background: var(--bg) }` 且 `color-scheme: light dark` |
| 禁止 | `transition` 作用于 `background-color` 全屏元素（会掉帧）；只给按钮/开关加 |

---

## 09.10 DPI 缩放与窗口尺寸

| 场景 | 要求 |
| --- | --- |
| 100% / 125% / 150% / 175% / 200% | 布局不破、文字不裁切、行高自适应 |
| 单位 | 一律 `rem` 或 `px` + `clamp()`；**禁止**写死 `height` 装文本（用 `min-height`） |
| 1px 边框 | 200% 缩放下用 `1px` 仍清晰（WebView2 按 DPI 渲染，不需要 `0.5px` 技巧） |
| 窗口 | 默认 880×620 **逻辑像素**；Tauri 的 `width/height` 即逻辑像素，系统自动乘 scale |
| 最小尺寸 | 720×520；低于此值内容区出现横向滚动而非挤压 |
| 多显示器 | 窗口跨屏拖动时由 WebView2 自动重排，禁止监听 `resize` 做像素级 hack |
| 虚拟滚动 | `ResizeObserver` 监听容器高度变化（DPI 变化会触发），自动重算可视行数 |
| 图标 | SVG（矢量），不用位图 |

**必须实测的缩放组合**：125%（最常见）+ 200%（高分屏），检查：列表行高、进度条文字、模态框宽度、
托盘 tooltip 截断。

---

## 09.11 无障碍与键盘

### 09.11.1 语义与焦点

| 项 | 要求 |
| --- | --- |
| 进度条 | `role="progressbar"` + `aria-valuenow/min/max` + `aria-label` |
| 开关 | `role="switch"` + `aria-checked`；用 `<button>` 而非 `<div>` |
| 模态框 | `role="dialog" aria-modal="true"` + 焦点陷阱（Tab 循环）+ 关闭后焦点还原 |
| 图标按钮 | 必须有 `aria-label` 或 `title` |
| 错误 | `aria-live="polite"` 容器承载 toast；`aria-invalid` 标在输入框 |
| 色觉 | 延迟色阶额外用 ✓/✕/● 符号区分（09.6.3） |
| 对比度 | 正文 ≥ 4.5:1，大字号 ≥ 3:1 |
| 减少动效 | `@media (prefers-reduced-motion: reduce) { * { transition: none !important } }` |

### 09.11.2 快捷键表

| 快捷键 | 作用 | 作用域 |
| --- | --- | --- |
| `Ctrl+1` / `Ctrl+2` / `Ctrl+3` / `Ctrl+4` | 切到 订阅/流量/节点/设置 | 全局（窗口内） |
| `Ctrl+R` | 刷新当前页（订阅页=全部更新；流量页=重新统计；节点页=测速） | 全局 |
| `Ctrl+N` | 添加订阅 | 订阅页 |
| `Ctrl+F` | 搜索节点（节点页顶部出现搜索框） | 节点页 |
| `Ctrl+,` | 打开设置 | 全局 |
| `Ctrl+Shift+L` | 查看日志弹窗 | 全局 |
| `F5` | 重新加载前端（仅 debug 构建） | 全局 |
| `Esc` | 关闭模态框 / 清空搜索 / 取消行内编辑 | 全局 |
| `↑` / `↓` | 在列表内移动当前行焦点 | 列表内 |
| `Enter` | 对聚焦行执行主操作（使用该节点 / 立即更新该订阅） | 列表内 |
| `Delete` | 删除聚焦订阅（弹二次确认） | 订阅页 |
| `Space` | 切换聚焦的开关项 | 全局 |

> 快捷键实现集中在 `ui/shortcuts.ts`，用 `keydown` + `event.ctrlKey/metaKey` 判定，
> **禁止**劫持浏览器默认（除 `Ctrl+R`/`F5`，因为我们不是网页）。
> 输入框聚焦时除 `Esc` 外一律不拦截（否则中文输入法会出问题）。

---

## 09.12 通知策略

| 触发 | 默认 | 级别 | 标题 / 正文 | 去重 |
| --- | --- | --- | --- | --- |
| 自动切换成功 | **关**（`notify_on_switch=false`，C.5 默认） | info | 「已切换到 {node}」/「延迟 {n} ms」 | 无（开关关着） |
| 自动切换失败降级 | 开 | warn | 「节点切换失败」/「已回退到 {fallback}」 | 1 次/10 分钟 |
| 订阅更新失败 | 开 | warn | 「订阅更新失败」/「{name}：{message}」 | 同一 uid 1 次/小时 |
| 订阅即将到期（≤3 天） | 开（`notify_on_expire`） | warn | 「订阅即将到期」/「{name} 将于 {date} 到期」 | 1 次/天 |
| 流量达阈值 | 开（`traffic_warn_percent`） | warn | 「流量即将用完」/「{name} 已用 {p}%」 | 1 次/天/订阅 |
| 内核反复崩溃（5 次） | 开 | error | 「内核无法启动」/「请查看日志或重新安装内核」 | 每次失败序列 1 次 |
| 系统代理残留已恢复 | 开 | warn | 「已恢复系统代理」/「上次异常退出……」 | 每次启动 1 次 |
| 首次最小化到托盘 | 开 | info | 「仍在后台运行」/「点击托盘图标可重新打开」 | 每安装周期 1 次（持久标记） |

| 实现要求 | 说明 |
| --- | --- |
| 通道 | Tauri 2 notification 插件；失败降级为托盘气泡（`system/notify.rs`，C.2 已规划） |
| 静默启动时 | `silent_start=true` 且由 `--autostart` 触发 → 抑制 info 级通知，只保留 warn/error |
| 应用内 toast | 与系统通知并存但文案更短；同一文案 5s 内不重复 |
| 不打扰 | 窗口在前台时**不发系统通知**，只发应用内 toast |
| 禁止 | 任何"推荐""赞助""节点推广"类通知（01 §8） |

---

## 09.13 加载 / 空 / 错 三态总表

| 页面 | 加载态 | 空态 | 错误态 |
| --- | --- | --- | --- |
| 订阅 | 3 张骨架卡 | 引导卡（含合规提示） | 行内 error-box + 重试 |
| 流量 | 骨架 + 「统计中…」 | 「没有订阅，无法统计」 | 顶部黄条 + 保留旧数据 |
| 节点 | 「正在读取节点…」 | 「暂无节点，请先更新订阅」 | 顶部红条 + 重试 |
| 设置 | 直接渲染（无网络依赖） | — | 保存失败 toast + 字段回滚 |
| 顶栏 | 内核状态灯 `灰/黄/绿/红` | — | 点击状态灯 → 日志弹窗 |

**加载态硬规则**：任何操作 > 200ms 必须有可见反馈；> 1s 必须可取消。
禁止全屏遮罩 spinner（会让 1.2s 冷启动预算变成"看起来卡了 3 秒"）。

---

## 09.14 性能约束（与 C.11、10 章对接）

| 约束 | 做法 | 违反后果 |
| --- | --- | --- |
| 隐藏时停止渲染 | `document.hidden` / Tauri 窗口 hide → 暂停 `traffic:tick` 的 DOM 更新（只累加数值） | RSS/CPU 超标 |
| 事件合并 | 所有高频事件经 `requestAnimationFrame` 或 250ms 定时器合并 | 列表抖动、CPU 尖峰 |
| 列表虚拟化 | >40 项启用 | 300 节点全量 DOM ≈ 60 MB |
| 无框架 | 手写 DOM + `DocumentFragment` | 包体 + 启动时间 |
| 无图标字体 | 内联 SVG symbol | 首屏多一次字体解析 |
| 图片 | 仅 `.ico`/`.png` 图标，无大图 | 包体 |
| CSS | 单文件 ≤ 20 KB（未压缩） | 解析时间 |
| 前端产物 | ≤ 60 KB gzip 前 | 与 12 MB 包体预算冲突 |

```ts
// src/main.ts —— 隐藏时降级：前端只负责「不写 DOM」，降频由后端自己做
let hidden = false;
document.addEventListener("visibilitychange", () => {
  hidden = document.hidden;                 // 仅本地标记，不发任何 IPC
});
```

> **不做 `ui_visibility_set`**（C.6 注记明文）：窗口可见性由 Rust 侧监听 Tauri 窗口事件
> （`WindowEvent::Focused` / 最小化 / `Resized`）自行判断，`traffic:tick` 的 2s→10s 降频由
> **后端决定**。前端不得为此发心跳，也不得假设「我隐藏了后端就知道了」——两边各自判定即可，
> 降频只是省电优化，不需要严格一致。

---

## 09.15 验收要点（P5 门禁）

1. 三页面在 100%/125%/200% 缩放下无裁切、无横向滚动（列表外）；
2. 空数据首次启动：显示引导空态，不出现空白页或报错；
3. 添加 300 个节点的订阅：节点页滚动帧率 ≥ 55 fps（DevTools Performance 实测）；
4. 测速进行中：列表可滚动、可点「使用」、CPU 不尖峰（≤25% 单核，C.11）；
5. 断网后点「立即更新」：行内出现 `E_NET_TIMEOUT` 中文文案 + 重试按钮，应用不卡；
6. 删除订阅必须二次确认；确认后列表立即移除；
7. 托盘：右键菜单 10 项内、节点子菜单 ≤15 项、勾选状态与主界面一致；
8. 关闭窗口到托盘 → 托盘双击恢复；`close_to_tray=false` 时关闭直接退出；
9. 深色模式下所有卡片、徽标、进度条对比度达标（DevTools 检查）；
10. 快捷键表逐项可用；输入框内不误触发；
11. 前端产物 `dist/` 总大小 ≤ 60 KB（gzip 前）；`pnpm tsc --noEmit` 无错、无 `any`（除已标注处）。

---

## 09.16 契约对齐记录（v1.1）

> C 章 v1.1 裁决了本章提出的 7 条缺口中的 5 条（第 4、6、7 条按原样保留，见 09.17）。
> 下表记录已落地的对齐点；原「冲突点」表作废（保留于 git 历史）。

| # | 裁决结果（C 章） | 本章修改 | 对应节 |
| --- | --- | --- | --- |
| 1 | 不做 `ui_visibility_set`；窗口可见性由 Rust 侧监听 Tauri 窗口事件判断 | 09.14 删除 `invoke("ui_visibility_set")` 代码与「契约缺口」注记，改为「前端只当本地标记、降频由后端决定」 | C.6 注记（`ui_visibility_set` 之后） |
| 2 | `RuntimeState.sysproxy: SysProxyState{enabled, server, managed_by_us, port}` | 新增 **09.1.2**（顶栏开关初值 + `bindSysproxy()` / `toggleSysproxy()`，用返回值回填）；09.8.1 增「四态表」区分 `managed_by_us` | C.5 `RuntimeState` / `SysProxyState` |
| 3 | `TrafficSummary` / `SubscriptionTraffic` / `AggregateTraffic` 完整定义，含 `used`/`used_percent`/`days_left`/`stale`/`has_missing` | 09.5.3 从「前端自算派生」改为「字段来源表 + 仅格式化」；删除 `usedBytes`/`remainBytes`/`usagePercent`/`daysLeft` 自算函数，改 `fmtRemain`/`fmtExpireText`/`fmtPercent`/`fmtAggTotal`；展示表改用 `item.*`/`agg.*`，新增 `stale` 与 `has_missing` 两行；09.5.4 空态按 `items`/`has_missing`/`stale` 重写 | C.5 三个新结构 + 派生字段规则 |
| 4 | 新命令 `switch_history {limit?}` → `Vec<SwitchRecord>`（默认 50、上限 200）；落盘 `runtime/switch-history.json` | 新增 **09.6.7「最近切换」**（线框 + `loadSwitchHistory()` + `REASON_TEXT` 四值映射），明确不再用 `logs_tail` 兜底；原「策略弹窗」小节顺延（v1.2 后为 **09.6.10**） | C.6 `switch_history`；C.3 `runtime/switch-history.json` |
| 5 | 新命令 `core_versions_list` → `{installed, current, available}` | 09.7 设置页「当前版本」改为下拉 → `core_versions_list`；「重新安装」补 `core:install-progress` | C.6 `core_versions_list` |
| 6 | `AppSettings.compliance_accepted_at: Option<i64>`（schema_version → 2） | 09.4.4 空态卡的合规提示补引用该字段（首次未接受时不可跳过） | C.5 `AppSettings` |
| 7 | 新事件 `sysproxy:state` / `core:install-progress` | 09.1.2 订阅 `sysproxy:state`；09.8.1 托盘重建时机加入 `sysproxy:state`；09.7 补 `core:install-progress` | C.7 |

### 09.16.1 首次合规提示的持久化（对应裁决 6）

| 项 | 取值 |
| --- | --- |
| 是否显示 | `settings.compliance_accepted_at == null` |
| 用户确认后 | `settings_save` 写入 `complianceAcceptedAt = now()`（Unix 秒，C.10） |
| 是否可跳过 | ❌ 不可；「继续」按钮在勾选前 disabled |
| 后续 | 非 null 即不再弹（文案大改需升 schema 并清空该字段） |

---

## 09.17 本章仍未决项（非契约缺口）

> 契约类缺口已全部由 C v1.1 裁决（见 09.16）。以下为**产品决策项**，需 Lead 最后一轮统一处理。

| # | 待确认项 | 当前默认 | 类型 |
| --- | --- | --- | --- |
| 1 | 是否自绘标题栏（`decorations:false`） | **是**（要放代理开关）；备选：保留系统标题栏，开关移到侧栏底部 | 产品决策 |
| 2 | 节点搜索（`Ctrl+F`）做多细 | 最简版：按名称/类型过滤，无模糊高亮 | 产品决策 |
| 3 | 是否提供 en-US | 只做 zh-CN，但所有文案走 i18n key | 产品决策 |
| 4 | `AppSettings` 无 `ui_scale` / 字号设置（无障碍大字号） | MVP 不做，用系统 DPI + 浏览器缩放兜底 | 契约扩展（需另提 CHANGE-REQUEST） |
| 5 | C.4 未规定前端事件名 TS 类型的大小写 | 统一：TS 类型 `PascalCase`，事件字符串照抄 C.7 | 约定细化（不阻塞） |
| 6 | 托盘菜单项的 i18n 键命名未规定 | 约定 `tray.<id>`，Rust 启动时载入 `zh-CN` 表 | 约定细化（不阻塞） |

> 原「是否需要最近切换历史页面」已由裁决 4 解决：**做**，作为节点页可折叠区（09.6.7）。

---

## 09.18 契约对齐记录（v1.2）

> C 章 v1.2 新增 3 条裁决，全部影响本章。

| # | 裁决结果（C 章） | 本章修改 | 对应节 |
| --- | --- | --- | --- |
| 1 | `switch_apply_best` 返回 `{primary: SwitchRecord, all: Vec<SwitchRecord>}` | 09.6.4 新增 `applyBest()`：按 `all.length` 分三种 toast 文案，明确 `primary` 用于文案、`all` 用于影响面；`scope=Group` 时 `all.len()==1` 不显示"N 个策略组" | C.6 `switch_apply_best` |
| 2 | `SwitchRecord.to_delay` → `Option<u32>`（`Manual` 时可能无新鲜测速数据，不得用 0 冒充） | 09.6.7 新增 `fmtDelayPair()`（`null` → `—`）+ 「可能为 null 的字段」四行表；线框中的延迟列注明 `超时 → 128ms` 与 `—` 的区别 | C.5 `SwitchRecord.to_delay` |
| 3 | 新增 `SwitchPolicy.manual_override_min`（默认 60，0=不暂停）+ 手动覆盖语义（内存态 `SwitchRuntimeLocks`，不进 `RuntimeState`） | 新增 **09.6.9 手动覆盖期提示**（`onManualSelect` / `remainSec` / `renderLockBar` / `unlockNow` + 7 行场景表 + 为何不进 `RuntimeState`）；09.6.1 线框加黄色锁定条；09.6.2 控件树加 `.manual-lock-bar`；09.6.10 策略弹窗加该字段；09.7 设置页「测速」组加输入项（0–1440）；09.8.1 托盘选中节点后按 `ttl` 给一次性气泡 | C.5 `manual_override_min` + 手动覆盖语义 |
| 4 | `switch_policy.scope` 指向的组必须是 `type: select` | 新增 **09.6.8 哪些组会被自动切换**（4 行规则表 + 冲突后果说明）；明确非 select 组内「使用」按钮置灰 | C.5 合并配置的组类型约束 |

> 09.16（v1.1）与 09.18（v1.2）并存：v1.1 记录 `sysproxy` / `TrafficSummary` / `switch_history` /
> `core_versions_list` / `compliance_accepted_at` 的对齐，v1.2 记录切换返回值与手动覆盖。

---

## 09.19 本章自检清单（Agent 必须逐条确认）

> 交付任何前端代码前逐条回答「是/否」，任一「否」不得进入下一阶段。与 08 章的
> 「08.15 验收要点」互补：08 管后端行为，本章管**前端契约与体验**。

- [ ] 1. 是否**没有**引入 React/Vue/Svelte/任何 UI 组件库/图标字体？是否全部手写 DOM + 内联 SVG symbol？
- [ ] 2. 状态是否只经 `Store`（按 key 订阅 + rAF 合并）流动？有无组件间直接互相改 DOM 的隐式耦合？
- [ ] 3. 首屏是否只调**一次** `app_bootstrap`？有无在页面里再补 `settings_get` / `subscription_list` 等零散请求？
- [ ] 4. 所有 IPC 是否经 `call<T>()` 归一（C.8 错误码 + 中文文案）？有无裸 `invoke` 散落各处？
- [ ] 5. 顶栏/托盘的系统代理开关是否**只用** `runtime.sysproxy`（`enabled` / `managed_by_us`）驱动？
      有无在前端另存一个 `isProxyOn` 布尔值？（v1.1）
- [ ] 6. 流量页是否**只格式化、不重算**？有无自行 `upload + download` 或自算 `total - used`？
      `total===0` / `null` / `stale` / `has_missing` 四个分支是否都有对应文案？（v1.1）
- [ ] 7. 「最近切换」是否用 `switch_history` 而非 `logs_tail`？`to_delay` 为 `null` 时是否显示 `—` 而非 `0ms`？（v1.1/v1.2）
- [ ] 8. `switch_apply_best` 的返回是否按 `{primary, all}` 消费？有无假设它返回单个 `SwitchRecord`？（v1.2）
- [ ] 9. 手动选节点后是否显示「🔒 已手动锁定，{n} 分钟内不会被自动切换」？`manual_override_min=0` 时是否**不**显示？（v1.2）
- [ ] 10. 节点列表是否在 >40 项时启用虚拟滚动？`itemHeight` 是否 44px 且 `contain: strict`？
- [ ] 11. `speedtest:progress` 是否做了 250ms 二次合并且**不重排列表**？单条事件负载是否 < 8 KB（C.7）？
- [ ] 12. 是否**没有**实现 `ui_visibility_set`？降频是否完全交给 Rust 侧窗口事件（C.6 注记）？（v1.1）
- [ ] 13. 托盘节点子菜单是否 ≤15 项？超出是否给「更多节点…」？勾选态是否只由 `switch:applied` 刷新？
- [ ] 14. 销毁性操作（删除订阅）是否二次确认且默认焦点不在确认按钮？`Esc` 是否取消？
- [ ] 15. 深色模式下所有卡片/徽标/进度条/延迟色阶对比度是否达标？延迟是否**同时**用符号（✓/✕）不只靠颜色（无障碍）？
- [ ] 16. 100% / 125% / 200% DPI 下是否无裁切、无横向滚动？最小窗口 720×520 下是否可用？
- [ ] 17. 空态/加载态/错误态是否三态齐全（不是只有"正常"一种）？错误态是否给出可操作按钮？
- [ ] 18. 前端产物 `dist/` 是否 ≤60 KB（gzip 前）？`pnpm tsc --noEmit` 是否无错、无 `any`（除已标注处）？
- [ ] 19. 所有文案是否走 i18n key（`tray.<id>` / 页面前缀），无硬编码中文字符串散落在逻辑里？
- [ ] 20. 本章是否**没有**修改 `01-master-prompt.md`、`appendices/C-conventions.md` 或其它章节？
      发现契约缺口是否以 `CONTRACT-CHANGE-REQUEST:` 提出而非直接改？
