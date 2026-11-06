/**
 * 应用入口：初始化、导航、标题栏、状态栏、事件绑定。
 */

import { api, coreStateText, formatRate, IpcError, on } from "./ipc/client";
import { setLocale, t } from "./i18n";
import { el, alertBox, modal, toast } from "./ui/kit";
import {
  refreshAll,
  refreshRuntime,
  refreshTraffic,
  store,
  subscribe,
} from "./ui/store";
import * as viewNodes from "./ui/views/nodes";
import * as viewSettings from "./ui/views/settings";
import * as viewSubs from "./ui/views/subscriptions";
import * as viewTraffic from "./ui/views/traffic";

const views = {
  subscriptions: viewSubs,
  traffic: viewTraffic,
  nodes: viewNodes,
  settings: viewSettings,
} as const;
type ViewName = keyof typeof views;

// ============================================================================
//  启动
// ============================================================================

async function main(): Promise<void> {
  bindTitlebar();
  bindNav();

  // 先拿设置（语言/主题要先应用，避免闪一下中文再切英文）
  try {
    const s = await api.settingsGet();
    store.settings = s;
    setLocale(s.locale);
    viewSettings.applyTheme(s.theme);
    // 跟随系统时要能响应系统主题切换（否则要重启才生效）
    viewSettings.watchSystemTheme(() => store.settings?.theme ?? "system");
  } catch {
    // 设置读取失败不阻塞启动，用默认
  }

  // 挂载所有视图（隐藏的也挂，切换时不卡）
  viewSubs.mount(mustGet("view-subscriptions"));
  viewTraffic.mount(mustGet("view-traffic"));
  viewNodes.mount(mustGet("view-nodes"));
  viewSettings.mount(mustGet("view-settings"));

  bindStatusbar();
  await bindEvents();

  // 首轮数据
  await refreshAll();
  viewSubs.refresh();
  viewTraffic.refresh();

  // 合规提示（首次启动）
  if (store.settings && !store.settings.complianceAccepted) {
    void showCompliance();
  }

  // 低频轮询：内核状态与系统代理（事件不可靠时兜底）
  window.setInterval(() => {
    void refreshRuntime();
  }, 2000);

  // 流量每 60 秒刷新
  window.setInterval(() => {
    void refreshTraffic();
  }, 60_000);

  updateStatusbar();
  subscribe("*", updateStatusbar);
}

function mustGet(id: string): HTMLElement {
  const n = document.getElementById(id);
  if (!n) throw new Error(`缺少节点 #${id}`);
  return n;
}

// ============================================================================
//  标题栏
// ============================================================================

function bindTitlebar(): void {
  document.getElementById("btn-min")?.addEventListener("click", () => void api.windowMinimize());
  document.getElementById("btn-max")?.addEventListener("click", () => void api.windowToggleMaximize());
  document.getElementById("btn-close")?.addEventListener("click", () => void api.windowClose());

  // 双击标题栏最大化
  document.getElementById("titlebar")?.addEventListener("dblclick", (e) => {
    if ((e.target as HTMLElement).closest(".tb-btn")) return;
    void api.windowToggleMaximize();
  });
}

// ============================================================================
//  导航
// ============================================================================

function bindNav(): void {
  document.querySelectorAll<HTMLButtonElement>(".nav-item").forEach((btn) => {
    btn.addEventListener("click", () => {
      const name = btn.dataset.view as ViewName | undefined;
      if (name) switchView(name);
    });
  });
}

function switchView(name: ViewName): void {
  store.view = name;

  document.querySelectorAll<HTMLButtonElement>(".nav-item").forEach((b) => {
    b.classList.toggle("active", b.dataset.view === name);
  });
  document.querySelectorAll<HTMLElement>(".view").forEach((v) => {
    v.classList.toggle("active", v.id === `view-${name}`);
  });

  // 切到对应页时拉一次最新数据
  if (name === "subscriptions") viewSubs.refresh();
  else if (name === "traffic") viewTraffic.refresh();
  else if (name === "nodes") viewNodes.refresh();
  else if (name === "settings") viewSettings.refresh();
}

// ============================================================================
//  状态栏
// ============================================================================

function bindStatusbar(): void {
  document.getElementById("sb-proxy")?.addEventListener("click", async () => {
    const managed = store.runtime?.sysproxyManaged ?? false;
    try {
      if (managed) {
        await api.sysproxyDisable();
        toast("系统代理已关闭", "ok");
      } else {
        await api.sysproxyEnable();
        toast("系统代理已开启", "ok");
      }
      await refreshRuntime();
    } catch (e) {
      toast((e as IpcError).message, "err");
    }
  });
}

function updateStatusbar(): void {
  const rt = store.runtime;
  const tb = document.getElementById("tb-status");
  const sbCore = document.getElementById("sb-core");
  const sbProxy = document.getElementById("sb-proxy");
  const sbTraffic = document.getElementById("sb-traffic");

  const state = rt?.core.state ?? "stopped";
  const text = coreStateText(state);

  if (tb) {
    tb.textContent = text;
    tb.className = "tb-status";
    if (state === "running") tb.classList.add("is-running");
    else if (state === "failed") tb.classList.add("is-failed");
    else if (state === "starting" || state === "stopping") tb.classList.add("is-starting");
  }

  if (sbCore) {
    const port = rt?.core.mixedPort;
    sbCore.textContent = `内核：${text}${port ? ` · 127.0.0.1:${port}` : ""}`;
  }

  if (sbProxy) {
    const on_ = rt?.sysproxyManaged ?? false;
    sbProxy.textContent = `系统代理：${on_ ? "已开启" : "关闭"}`;
    sbProxy.style.cursor = "pointer";
    sbProxy.title = "点击切换系统代理";
    sbProxy.style.color = on_ ? "var(--good)" : "";
  }

  if (sbTraffic && rt) {
    sbTraffic.textContent = `↑ ${formatRate(rt.upRate)}  ↓ ${formatRate(rt.downRate)}`;
  }
}

// ============================================================================
//  后端事件
// ============================================================================

async function bindEvents(): Promise<void> {
  await on("core:state", (p) => {
    void refreshRuntime();
    const payload = p as { state?: string; error?: string };
    if (payload.state === "failed" && payload.error) {
      void alertBox(t("err.title"), payload.error);
    }
  });

  await on("core:installFailed", (p) => {
    const payload = p as { message?: string };
    toast(`内核下载失败：${payload.message ?? ""}`, "err", 5000);
  });

  await on("core:installed", (p) => {
    const payload = p as { version?: string };
    toast(`内核 ${payload.version ?? ""} 安装完成`, "ok");
    void refreshRuntime();
  });

  await on("core:downloadProgress", (p) => {
    const payload = p as { downloaded: number; total: number | null };
    if (payload.total) {
      const pct = ((payload.downloaded / payload.total) * 100).toFixed(0);
      // 只在 25% 整数关口提示，避免刷屏
      if (Number(pct) % 25 === 0) toast(`内核下载 ${pct}%`);
    }
  });

  await on("subscription:fetched", () => {
    viewSubs.refresh();
    void refreshTraffic();
  });

  await on("subscription:fetchedAll", () => {
    viewSubs.refresh();
    void refreshTraffic();
  });

  await on("subscription:applied", () => {
    viewNodes.refresh();
  });

  await on("sysproxy:changed", () => void refreshRuntime());

  await on("sysproxy:failed", (p) => {
    const payload = p as { message?: string };
    toast(`系统代理设置失败：${payload.message ?? ""}`, "err", 4000);
  });

  await on("speedtest:done", () => {
    viewNodes.refresh();
  });

  await on("switch:applied", (p) => {
    const recs = p as { toNode: string }[];
    if (Array.isArray(recs) && recs.length > 0) {
      toast(`已自动切换到 ${recs[0].toNode}`, "ok");
    }
    viewNodes.refresh();
  });

  await on("switch:manual", () => viewNodes.refresh());

  await on("settings:changed", () => {
    void refreshAll();
  });
}

// ============================================================================
//  合规提示
// ============================================================================

async function showCompliance(): Promise<void> {
  const body = el("div", {}, [
    el("div", { style: "white-space:pre-line", text: t("compliance.body") }),
  ]);
  const ok = await modal({
    title: t("compliance.title"),
    body,
    okText: t("compliance.accept"),
    cancelText: null,
  });
  if (ok) {
    try {
      await api.acceptCompliance();
      if (store.settings) store.settings.complianceAccepted = true;
    } catch {
      // 记录失败不阻塞使用
    }
  }
}

// ---- 启动 ----
main().catch((e) => {
  console.error("启动失败", e);
  const root = document.getElementById("content");
  if (root) {
    root.innerHTML = "";
    root.append(
      el("div", { class: "empty" }, [
        el("div", { text: "界面初始化失败" }),
        el("pre", {
          class: "mono small",
          style: "margin-top:10px;text-align:left;white-space:pre-wrap",
          text: e instanceof Error ? e.message : String(e),
        }),
      ]),
    );
  }
});
