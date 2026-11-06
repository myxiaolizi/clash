/**
 * IPC 客户端（契约 C.6）。
 *
 * 全部命令经由此处调用，好处：
 * 1. 命令名集中管理，改名只需改一处
 * 2. 错误统一转成 `IpcError`，前端据 `code` 分支
 * 3. 类型完整，编译期即可发现字段拼错
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ============================================================================
//  类型（与后端 commands/dto.rs 一一对应）
// ============================================================================

export interface ErrorDto {
  code: string;
  message: string;
  detail: string | null;
  retryable: boolean;
}

/** 后端统一错误载荷；invoke 失败时 reject 的就是这个对象 */
export class IpcError extends Error {
  readonly code: string;
  readonly detail: string | null;
  readonly retryable: boolean;

  constructor(dto: ErrorDto) {
    super(dto.message);
    this.name = "IpcError";
    this.code = dto.code;
    this.detail = dto.detail;
    this.retryable = dto.retryable;
  }
}

function toIpcError(e: unknown): IpcError {
  if (e && typeof e === "object" && "code" in e && "message" in e) {
    const d = e as ErrorDto;
    return new IpcError({
      code: String(d.code),
      message: String(d.message),
      detail: d.detail ?? null,
      retryable: Boolean(d.retryable),
    });
  }
  return new IpcError({
    code: "E_INTERNAL",
    message: e instanceof Error ? e.message : String(e),
    detail: null,
    retryable: false,
  });
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw toIpcError(e);
  }
}

// ---- DTO ----

export interface AppInfo {
  version: string;
  coreVersion: string | null;
  dataDir: string;
  logDir: string;
  os: string;
  arch: string;
}

export interface TrafficInfo {
  upload: number | null;
  download: number | null;
  total: number | null;
  used: number | null;
  usedPercent: number | null;
  expireAt: number | null;
  daysLeft: number | null;
  source: "header" | "web" | "unknown" | string;
  stale: boolean;
}

export interface Subscription {
  uid: string;
  name: string;
  urlHint: string;
  enabled: boolean;
  updateIntervalMin: number;
  lastUpdateAt: number | null;
  lastUpdateStatus: "never" | "ok" | "failed" | string;
  lastError: string | null;
  nodeCount: number;
  sortOrder: number;
  traffic: TrafficInfo | null;
}

export interface ItemTraffic {
  uid: string;
  name: string;
  upload: number | null;
  download: number | null;
  total: number | null;
  used: number | null;
  usedPercent: number | null;
  expireAt: number | null;
  daysLeft: number | null;
  source: string;
  stale: boolean;
}

export interface TrafficSummary {
  items: ItemTraffic[];
  upload: number;
  download: number;
  total: number | null;
  used: number;
  usedPercent: number | null;
  expireAt: number | null;
  daysLeft: number | null;
  subscriptionCount: number;
  hasMissing: boolean;
  at: number;
}

export interface PolicyDto {
  enabled: boolean;
  toleranceMs: number;
  minDwellSec: number;
  failureThreshold: number;
  manualOverrideMin: number;
  excludeKeywords: string[];
  preferRegionPriority: string[];
}

export interface Settings {
  locale: string;
  theme: string;
  autoLaunch: boolean;
  silentStart: boolean;
  startCoreOnLaunch: boolean;
  complianceAccepted: boolean;
  proxyMode: string;
  apiPort: number;
  coreVersion: string | null;
  coreDownloadProxy: string | null;
  subDefaultIntervalMin: number;
  subFetchConcurrency: number;
  subFetchTimeoutMs: number;
  subFetchProxy: string | null;
  subUserAgent: string;
  speedtestEnabled: boolean;
  speedtestIntervalMin: number;
  speedtestProbeUrl: string;
  speedtestTimeoutMs: number;
  speedtestConcurrency: number;
  switchPolicy: PolicyDto;
  notifyOnSwitch: boolean;
  notifyOnExpire: boolean;
  trafficWarnPercent: number;
  logLevel: string;
  closeToTray: boolean;
}

export type SettingsPatch = Partial<Omit<Settings, "complianceAccepted" | "switchPolicy">> & {
  switchPolicy?: Partial<PolicyDto>;
};

export interface CoreStatus {
  state: "stopped" | "starting" | "running" | "stopping" | "failed" | string;
  error: string | null;
  version: string | null;
  mixedPort: number | null;
  apiPort: number | null;
  uptimeSec: number;
}

export interface Runtime {
  core: CoreStatus;
  sysproxyEnabled: boolean;
  sysproxyManaged: boolean;
  sysproxyServer: string | null;
  activeSubscription: string | null;
  trafficUp: number;
  trafficDown: number;
  upRate: number;
  downRate: number;
}

export interface InstalledCore {
  version: string;
  size: number;
  installedAt: number;
  isActive: boolean;
}

export interface NodeInfo {
  name: string;
  delayMs: number | null;
  ok: boolean;
  rank: number | null;
  error: string | null;
  selected: boolean;
}

export interface GroupInfo {
  name: string;
  now: string | null;
  nodeCount: number;
  nodes: NodeInfo[];
}

export interface SpeedtestStatus {
  running: boolean;
  runId: string | null;
  total: number;
  finished: number;
  okCount: number;
  startedAt: number | null;
  finishedAt: number | null;
  bestDelay: number | null;
}

export interface SwitchRecord {
  at: number;
  group: string;
  fromNode: string | null;
  toNode: string;
  fromDelay: number | null;
  toDelay: number | null;
  reason: string;
}

export interface OpResult {
  ok: boolean;
  message: string;
}

// ============================================================================
//  命令封装
// ============================================================================

export const api = {
  // ---- app ----
  appInfo: () => call<AppInfo>("app_info"),
  acceptCompliance: () => call<OpResult>("app_accept_compliance"),
  openDataDir: () => call<OpResult>("app_open_data_dir"),
  openLogDir: () => call<OpResult>("app_open_log_dir"),

  // ---- settings ----
  settingsGet: () => call<Settings>("settings_get"),
  settingsUpdate: (patch: SettingsPatch) =>
    call<Settings>("settings_update", { patch }),

  // ---- core ----
  coreStatus: () => call<CoreStatus>("core_status"),
  coreStart: () => call<CoreStatus>("core_start"),
  coreStop: () => call<CoreStatus>("core_stop"),
  coreRestart: () => call<CoreStatus>("core_restart"),
  coreListVersions: () => call<InstalledCore[]>("core_list_versions"),
  coreInstall: (version?: string) =>
    call<OpResult>("core_install", { version: version ?? null }),

  // ---- sysproxy / runtime ----
  sysproxyGet: () => call<Runtime>("sysproxy_get"),
  sysproxyEnable: () => call<Runtime>("sysproxy_enable"),
  sysproxyDisable: () => call<Runtime>("sysproxy_disable"),
  runtimeGet: () => call<Runtime>("runtime_get"),
  showWindow: () => call<OpResult>("runtime_show_window"),

  // ---- window ----
  windowMinimize: () => call<OpResult>("window_minimize"),
  windowToggleMaximize: () => call<OpResult>("window_toggle_maximize"),
  windowClose: () => call<OpResult>("window_close"),

  // ---- subscription ----
  subList: () => call<Subscription[]>("subscription_list"),
  subAdd: (name: string, url: string) =>
    call<Subscription>("subscription_add", { name, url }),
  subRemove: (uid: string) => call<OpResult>("subscription_remove", { uid }),
  subUpdate: (
    uid: string,
    opts: { name?: string; enabled?: boolean; intervalMin?: number } = {},
  ) =>
    call<Subscription>("subscription_update", {
      uid,
      name: opts.name ?? null,
      enabled: opts.enabled ?? null,
      intervalMin: opts.intervalMin ?? null,
    }),
  subFetch: (uid: string, force = false) =>
    call<Subscription>("subscription_fetch", { uid, force }),
  subFetchAll: () => call<OpResult>("subscription_fetch_all"),
  subTraffic: () => call<TrafficSummary>("subscription_traffic"),
  subApply: () => call<OpResult>("subscription_apply"),
  subPreviewConfig: () => call<string>("subscription_preview_config"),

  // ---- node / speedtest / switch ----
  nodeGroups: () => call<GroupInfo[]>("node_groups"),
  nodeSelect: (group: string, node: string) =>
    call<OpResult>("node_select", { group, node }),
  nodeRawKind: (group: string) => call<string>("node_group_raw_kind", { group }),
  speedtestRun: () => call<SpeedtestStatus>("speedtest_run"),
  speedtestStatus: () => call<SpeedtestStatus>("speedtest_status"),
  switchAutoApply: () => call<SwitchRecord[]>("switch_auto_apply"),
  switchHistory: (limit = 50) => call<SwitchRecord[]>("switch_history", { limit }),
  switchManual: (group: string, node: string) =>
    call<SwitchRecord>("switch_manual", { group, node }),
};

// ============================================================================
//  事件（契约 C.7：`<domain>:<event>`）
// ============================================================================

export type AppEventName =
  | "core:state"
  | "core:downloadProgress"
  | "core:installed"
  | "core:installFailed"
  | "subscription:added"
  | "subscription:removed"
  | "subscription:fetched"
  | "subscription:fetchedAll"
  | "subscription:updated"
  | "subscription:applied"
  | "sysproxy:changed"
  | "sysproxy:failed"
  | "speedtest:started"
  | "speedtest:done"
  | "switch:applied"
  | "switch:manual"
  | "node:switched"
  | "settings:changed";

export function on<T = unknown>(
  event: AppEventName,
  handler: (payload: T) => void,
): Promise<UnlistenFn> {
  return listen<T>(event, (e) => handler(e.payload));
}

// ============================================================================
//  格式化工具（UI 共用）
// ============================================================================

export function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null || !isFinite(bytes) || bytes < 0) return "—";
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const v = bytes / Math.pow(1024, i);
  return `${v.toFixed(i === 0 ? 0 : v >= 100 ? 0 : 1)} ${units[i]}`;
}

export function formatRate(bytesPerSec: number): string {
  return `${formatBytes(bytesPerSec)}/s`;
}

export function formatPercent(p: number | null | undefined): string {
  if (p == null || !isFinite(p)) return "不限量";
  return `${p.toFixed(1)}%`;
}

export function formatDelay(ms: number | null | undefined): string {
  if (ms == null) return "—";
  return `${ms} ms`;
}

/** Unix 秒 → 本地 `YYYY-MM-DD` */
export function formatDate(unixSec: number | null | undefined): string {
  if (unixSec == null || unixSec <= 0) return "—";
  const d = new Date(unixSec * 1000);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

/** Unix 秒 → 本地 `MM-DD HH:mm` */
export function formatDateTime(unixSec: number | null | undefined): string {
  if (unixSec == null || unixSec <= 0) return "—";
  const d = new Date(unixSec * 1000);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

/** 相对时间：刚刚 / N 分钟前 / N 小时前 / N 天前 */
export function formatRelative(unixSec: number | null | undefined): string {
  if (unixSec == null || unixSec <= 0) return "从未";
  const diff = Math.floor(Date.now() / 1000) - unixSec;
  if (diff < 60) return "刚刚";
  if (diff < 3600) return `${Math.floor(diff / 60)} 分钟前`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} 小时前`;
  return `${Math.floor(diff / 86400)} 天前`;
}

/** 剩余天数的人类可读描述 */
export function formatDaysLeft(days: number | null | undefined): string {
  if (days == null) return "无限期";
  if (days < 0) return "已过期";
  if (days === 0) return "今天到期";
  if (days === 1) return "明天到期";
  return `${days} 天后到期`;
}

/** 延迟分级，用于配色 */
export function delayClass(ms: number | null | undefined): string {
  if (ms == null) return "delay-none";
  if (ms < 100) return "delay-good";
  if (ms < 300) return "delay-ok";
  if (ms < 800) return "delay-slow";
  return "delay-bad";
}

/** 内核状态 → 中文文案 */
export function coreStateText(state: string): string {
  switch (state) {
    case "running":
      return "运行中";
    case "starting":
      return "启动中";
    case "stopping":
      return "停止中";
    case "failed":
      return "启动失败";
    case "stopped":
      return "已停止";
    default:
      return state;
  }
}

/** 错误码 → 友好建议（帮助用户自救） */
export function errorHint(code: string): string {
  switch (code) {
    case "E_CORE_NOT_FOUND":
      return "请先在设置中下载内核";
    case "E_CORE_START_FAILED":
      return "请查看日志，可能是端口被占用或配置无效";
    case "E_CORE_API_UNAUTH":
      return "请尝试重启内核以重新同步鉴权信息";
    case "E_CORE_PORT_EXHAUSTED":
      return "端口被占用，请重启软件或关闭占用端口的程序";
    case "E_NET_TIMEOUT":
      return "请检查网络或代理设置";
    case "E_NET_DNS":
      return "域名解析失败，请检查 DNS 或网络连接";
    case "E_NET_TLS":
      return "证书校验失败，请检查系统时间或机场证书";
    case "E_NET_PROXY":
      return "代理连接失败，请检查拉取代理设置";
    case "E_HTTP_STATUS":
      return "订阅服务器返回错误，请确认订阅地址与令牌";
    case "E_PARSE_YAML":
      return "返回内容不是有效配置，可能是登录页或 UA 被拒";
    case "E_PARSE_EMPTY":
      return "该订阅里没有可用节点";
    case "E_SUB_URL_INVALID":
      return "请填写完整的 http(s) 订阅地址";
    case "E_USERINFO_MISSING":
      return "该机场未提供流量信息，属正常情况";
    case "E_SYSPROXY_FAILED":
      return "系统代理设置失败，请检查是否有安全软件拦截";
    case "E_PERM_ADMIN":
      return "该操作需要管理员权限";
    case "E_SPEEDTEST_RUNNING":
      return "请等待当前测速完成";
    case "E_SPEEDTEST_NO_NODE":
      return "请先添加订阅并更新";
    default:
      return "";
  }
}
