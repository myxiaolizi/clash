/**
 * 设置视图。
 *
 * 每个控件变更即调用 `settings_update`（后端做 sanitize），无需"保存"按钮，
 * 避免用户改完忘记保存。
 */

import {
  api,
  formatBytes,
  formatDateTime,
  type AppInfo,
  type InstalledCore,
  type Settings,
  type SettingsPatch,
} from "../../ipc/client";
import { setLocale, t } from "../../i18n";
import { clear, el, modal, runWithBusy, toast } from "../kit";
import { refreshSettings, refreshRuntime, store, subscribe } from "../store";

let container: HTMLElement | null = null;
let info: AppInfo | null = null;
let versions: InstalledCore[] = [];
let lastSig = "";

export function mount(root: HTMLElement): void {
  container = root;
  render();
  subscribe("settings", () => render());
  subscribe("runtime", () => render());
  void loadInfo();
}

export function refresh(): void {
  void loadInfo();
  void refreshSettings();
}

async function loadInfo(): Promise<void> {
  try {
    info = await api.appInfo();
    versions = await api.coreListVersions().catch(() => []);
  } catch {
    info = null;
  }
  lastSig = "";
  render();
}

function render(): void {
  if (!container) return;
  const s = store.settings;
  if (!s) return;

  const sig = JSON.stringify({ s, info, versions, core: store.runtime?.core.state });
  if (sig === lastSig) return;
  lastSig = sig;

  clear(container);

  container.append(
    el("div", { class: "page-head" }, [
      el("div", {}, [
        el("h2", { text: t("settings.title") }),
        el("div", { class: "page-desc", text: t("settings.desc") }),
      ]),
    ]),
  );

  container.append(section(t("settings.general"), renderGeneral(s)));
  container.append(section(t("settings.core"), renderCore(s)));
  container.append(section(t("settings.sub"), renderSub(s)));
  container.append(section(t("settings.speedtest"), renderSpeedtest(s)));
  container.append(section(t("settings.policy"), renderPolicy(s)));
  container.append(section(t("settings.advanced"), renderAdvanced(s)));
}

function section(title: string, rows: HTMLElement[]): HTMLElement {
  return el("div", { class: "settings-section" }, [
    el("h3", { text: title }),
    el("div", { class: "card" }, rows),
  ]);
}

function row(label: string, hint: string | null, control: HTMLElement): HTMLElement {
  return el("div", { class: "setting-row" }, [
    el("div", {}, [
      el("div", { class: "setting-label", text: label }),
      hint ? el("div", { class: "setting-hint", text: hint }) : null,
    ]),
    control,
  ]);
}

/**
 * 统一的设置提交：成功则用后端返回值更新本地（后端会 sanitize）。
 */
async function patch(p: SettingsPatch): Promise<void> {
  try {
    const next = await api.settingsUpdate(p);
    store.settings = next;
    lastSig = "";
    // 语言变更立即生效
    setLocale(next.locale);
    applyTheme(next.theme);
    render();
    toast(t("common.saved"), "ok");
  } catch (e) {
    toast((e as Error).message, "err");
  }
}

export function applyTheme(theme: string): void {
  const root = document.documentElement;
  if (theme === "system") {
    // 跟随系统：浅色系统必须显式落 light，不能只 removeAttribute。
    // `:root` 默认是深色变量，浅色系统下什么都不做 = 永远深色，
    // 用户选"跟随系统"却看不到任何变化。
    const dark = window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? true;
    root.setAttribute("data-theme", dark ? "dark" : "light");
  } else {
    root.setAttribute("data-theme", theme);
  }
}

/// 监听系统主题变化（仅在 theme=system 时生效），返回取消订阅函数
export function watchSystemTheme(getTheme: () => string): () => void {
  const mq = window.matchMedia?.("(prefers-color-scheme: dark)");
  if (!mq?.addEventListener) return () => {};
  const handler = () => {
    if (getTheme() === "system") applyTheme("system");
  };
  mq.addEventListener("change", handler);
  return () => mq.removeEventListener("change", handler);
}

// ============================================================================
//  各分区
// ============================================================================

function renderGeneral(s: Settings): HTMLElement[] {
  const localeSel = el("select", {}, [
    option("zh-CN", "简体中文", s.locale === "zh-CN"),
    option("en-US", "English", s.locale === "en-US"),
  ]) as HTMLSelectElement;
  localeSel.addEventListener("change", () => void patch({ locale: localeSel.value }));

  const themeSel = el("select", {}, [
    option("system", t("settings.theme.system"), s.theme === "system"),
    option("dark", t("settings.theme.dark"), s.theme === "dark"),
    option("light", t("settings.theme.light"), s.theme === "light"),
  ]) as HTMLSelectElement;
  themeSel.addEventListener("change", () => void patch({ theme: themeSel.value }));

  return [
    row(t("settings.language"), null, localeSel),
    row(t("settings.theme"), null, themeSel),
    row(t("settings.closeToTray"), "关闭后从托盘右键菜单可退出", toggle(s.closeToTray, (v) => void patch({ closeToTray: v }))),
    row(t("settings.autoLaunch"), s.silentStart ? "需同时开启静默启动才会隐藏窗口" : null, toggle(s.autoLaunch, (v) => void patch({ autoLaunch: v }))),
    row(t("settings.silentStart"), null, toggle(s.silentStart, (v) => void patch({ silentStart: v }))),
  ];
}

function renderCore(s: Settings): HTMLElement[] {
  const rt = store.runtime;
  const stateText = rt ? rt.core.state : "stopped";

  const btnInstall = el("button", { class: "btn", text: t("settings.installCore") });
  btnInstall.addEventListener("click", () =>
    runWithBusy(btnInstall, () => api.coreInstall(), { okMsg: "已开始下载，请稍候" }),
  );

  const btnRestart = el("button", { class: "btn sm", text: "重启内核" });
  btnRestart.addEventListener("click", () =>
    runWithBusy(btnRestart, async () => {
      const r = await api.coreRestart();
      await refreshRuntime();
      return r;
    }, { okMsg: "内核已重启" }),
  );

  const btnToggle = el("button", {
    class: `btn sm ${stateText === "running" ? "danger" : "primary"}`,
    text: stateText === "running" ? "停止内核" : "启动内核",
  });
  btnToggle.addEventListener("click", () =>
    runWithBusy(btnToggle, async () => {
      const r = stateText === "running" ? await api.coreStop() : await api.coreStart();
      await refreshRuntime();
      return r;
    }),
  );

  const proxySel = el("select", {}, [
    option("system_proxy", t("settings.proxyMode.system"), s.proxyMode === "system_proxy"),
    option("tun", t("settings.proxyMode.tun"), s.proxyMode === "tun"),
    option("manual", t("settings.proxyMode.manual"), s.proxyMode === "manual"),
  ]) as HTMLSelectElement;
  proxySel.addEventListener("change", () => void patch({ proxyMode: proxySel.value }));

  const dlProxy = el("input", {
    type: "text",
    value: s.coreDownloadProxy ?? "",
    placeholder: "http://127.0.0.1:1886",
  }) as HTMLInputElement;
  dlProxy.addEventListener("change", () =>
    void patch({ coreDownloadProxy: dlProxy.value.trim() }),
  );

  const rows: HTMLElement[] = [
    row(t("settings.coreState"), `版本 ${rt?.core.version ?? s.coreVersion ?? "未安装"}`, el("div", { class: "row" }, [btnToggle, btnRestart])),
    row(t("settings.startOnLaunch"), null, toggle(s.startCoreOnLaunch, (v) => void patch({ startCoreOnLaunch: v }))),
    row(t("settings.proxyMode"), "系统代理模式无需 TUN 权限", proxySel),
    row(t("settings.downloadProxy"), t("settings.downloadProxyHint"), dlProxy),
    row(t("settings.installCore"), null, btnInstall),
  ];

  // 已安装版本列表
  if (versions.length > 0) {
    const list = el("div", { class: "row", style: "flex-wrap:wrap;gap:6px" }, 
      versions.map((v) =>
        el("span", {
          class: `badge ${v.isActive ? "ok" : "dim"}`,
          text: `${v.version} · ${formatBytes(v.size)}`,
          title: `安装于 ${formatDateTime(v.installedAt)}`,
        }),
      ),
    );
    rows.push(row(t("settings.coreInstalled"), null, list));
  }

  return rows;
}

function renderSub(s: Settings): HTMLElement[] {
  const interval = numInput(s.subDefaultIntervalMin, 15, 10080, 15, (v) =>
    void patch({ subDefaultIntervalMin: v }),
  );
  const conc = numInput(s.subFetchConcurrency, 1, 4, 1, (v) => void patch({ subFetchConcurrency: v }));
  const timeout = numInput(s.subFetchTimeoutMs, 3000, 60000, 1000, (v) =>
    void patch({ subFetchTimeoutMs: v }),
  );

  const proxy = el("input", {
    type: "text",
    value: s.subFetchProxy ?? "",
    placeholder: "http://127.0.0.1:1886",
  }) as HTMLInputElement;
  proxy.addEventListener("change", () => void patch({ subFetchProxy: proxy.value.trim() }));

  const ua = el("input", { type: "text", value: s.subUserAgent }) as HTMLInputElement;
  ua.addEventListener("change", () => void patch({ subUserAgent: ua.value.trim() }));

  return [
    row(t("settings.subInterval"), "低于 15 分钟会被强制修正", interval),
    row(t("settings.subConcurrency"), "同时拉取的订阅数量", conc),
    row(t("settings.subTimeout"), null, timeout),
    row(t("settings.subProxy"), t("settings.subProxyHint"), proxy),
    row(t("settings.userAgent"), "部分机场靠 UA 判断客户端类型", ua),
  ];
}

function renderSpeedtest(s: Settings): HTMLElement[] {
  const interval = numInput(s.speedtestIntervalMin, 5, 1440, 5, (v) =>
    void patch({ speedtestIntervalMin: v }),
  );
  const timeout = numInput(s.speedtestTimeoutMs, 500, 10000, 500, (v) =>
    void patch({ speedtestTimeoutMs: v }),
  );
  const conc = numInput(s.speedtestConcurrency, 1, 16, 1, (v) =>
    void patch({ speedtestConcurrency: v }),
  );

  const probe = el("input", {
    type: "url",
    value: s.speedtestProbeUrl,
    spellcheck: false,
  }) as HTMLInputElement;
  probe.addEventListener("change", () => void patch({ speedtestProbeUrl: probe.value.trim() }));

  return [
    row(t("settings.speedtestEnabled"), null, toggle(s.speedtestEnabled, (v) => void patch({ speedtestEnabled: v }))),
    row(t("settings.speedtestInterval"), "低于 5 分钟会被强制修正", interval),
    row(t("settings.probeUrl"), "推荐 generate_204 类地址（返回 204 才算通）", probe),
    row(t("settings.probeTimeout"), null, timeout),
    row(t("settings.concurrency"), "过高可能被机场限速", conc),
  ];
}

function renderPolicy(s: Settings): HTMLElement[] {
  const p = s.switchPolicy;

  const tol = numInput(p.toleranceMs, 1, 5000, 10, (v) =>
    void patch({ switchPolicy: { ...p, toleranceMs: v } }),
  );
  const dwell = numInput(p.minDwellSec, 30, 3600, 30, (v) =>
    void patch({ switchPolicy: { ...p, minDwellSec: v } }),
  );
  const fail = numInput(p.failureThreshold, 1, 10, 1, (v) =>
    void patch({ switchPolicy: { ...p, failureThreshold: v } }),
  );
  const manual = numInput(p.manualOverrideMin, 0, 1440, 5, (v) =>
    void patch({ switchPolicy: { ...p, manualOverrideMin: v } }),
  );

  const exclude = el("input", {
    type: "text",
    value: p.excludeKeywords.join(", "),
    placeholder: "过期, 剩余, 流量",
  }) as HTMLInputElement;
  exclude.addEventListener("change", () =>
    void patch({ switchPolicy: { ...p, excludeKeywords: splitList(exclude.value) } }),
  );

  const region = el("input", {
    type: "text",
    value: p.preferRegionPriority.join(", "),
    placeholder: "香港, 台湾, 日本",
  }) as HTMLInputElement;
  region.addEventListener("change", () =>
    void patch({ switchPolicy: { ...p, preferRegionPriority: splitList(region.value) } }),
  );

  return [
    row(t("settings.policyEnabled"), null, toggle(p.enabled, (v) => void patch({ switchPolicy: { ...p, enabled: v } }))),
    row(t("settings.tolerance"), t("settings.toleranceHint"), tol),
    row(t("settings.minDwell"), null, dwell),
    row(t("settings.failureThreshold"), "当前节点连续失败多少次后强制切换", fail),
    row(t("settings.manualOverride"), t("settings.manualOverrideHint"), manual),
    row(t("settings.exclude"), t("settings.excludeHint"), exclude),
    row(t("settings.regionPriority"), t("settings.regionPriorityHint"), region),
  ];
}

function renderAdvanced(s: Settings): HTMLElement[] {
  const logSel = el("select", {}, 
    ["error", "warn", "info", "debug"].map((lv) =>
      option(lv, lv.toUpperCase(), s.logLevel === lv),
    ),
  ) as HTMLSelectElement;
  logSel.addEventListener("change", () => void patch({ logLevel: logSel.value }));

  const warn = numInput(s.trafficWarnPercent, 50, 100, 5, (v) =>
    void patch({ trafficWarnPercent: v }),
  );

  const btnData = el("button", { class: "btn sm", text: t("settings.openData") });
  btnData.addEventListener("click", () => void api.openDataDir());

  const btnLog = el("button", { class: "btn sm", text: t("settings.openLog") });
  btnLog.addEventListener("click", () => void api.openLogDir());

  const btnPreview = el("button", { class: "btn sm", text: "查看当前配置" });
  btnPreview.addEventListener("click", async () => {
    try {
      const yaml = await api.subPreviewConfig();
      await modal({
        title: "当前生成的配置",
        body: el("pre", { text: yaml || "（空：请先添加订阅）" }),
        okText: t("common.close"),
        cancelText: null,
      });
    } catch (e) {
      toast((e as Error).message, "err");
    }
  });

  return [
    row(t("settings.logLevel"), "DEBUG 会产生大量日志，仅排障时使用", logSel),
    row(t("settings.notifySwitch"), null, toggle(s.notifyOnSwitch, (v) => void patch({ notifyOnSwitch: v }))),
    row(t("settings.notifyExpire"), null, toggle(s.notifyOnExpire, (v) => void patch({ notifyOnExpire: v }))),
    row(t("settings.trafficWarn"), null, warn),
    row(t("settings.dataDir"), info?.dataDir ?? "—", btnData),
    row(t("settings.logDir"), info?.logDir ?? "—", btnLog),
    row("配置", "查看下发给内核的完整 YAML", btnPreview),
    row(t("settings.version"), `${info?.version ?? "0.1.0"} · ${info?.os ?? ""}/${info?.arch ?? ""}`, el("span", {})),
  ];
}

// ============================================================================
//  控件工厂
// ============================================================================

function option(value: string, label: string, selected: boolean): HTMLOptionElement {
  const o = document.createElement("option");
  o.value = value;
  o.textContent = label;
  o.selected = selected;
  return o;
}

function toggle(checked: boolean, onChange: (v: boolean) => void): HTMLElement {
  const input = el("input", { type: "checkbox", checked }) as HTMLInputElement;
  input.addEventListener("change", () => onChange(input.checked));
  return el("label", { class: "switch" }, [input, el("span", { class: "track" })]);
}

function numInput(
  value: number,
  min: number,
  max: number,
  step: number,
  onChange: (v: number) => void,
): HTMLElement {
  const input = el("input", {
    type: "number",
    value: String(value),
    min: String(min),
    max: String(max),
    step: String(step),
  }) as HTMLInputElement;
  input.addEventListener("change", () => {
    const v = parseInt(input.value, 10);
    if (!isFinite(v)) {
      input.value = String(value);
      return;
    }
    const clamped = Math.min(max, Math.max(min, v));
    if (clamped !== v) input.value = String(clamped);
    onChange(clamped);
  });
  return input;
}

function splitList(s: string): string[] {
  return s
    .split(/[,，;；]/)
    .map((x) => x.trim())
    .filter(Boolean);
}
