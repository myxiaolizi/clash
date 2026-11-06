/**
 * 节点与测速视图。
 */

import {
  api,
  delayClass,
  formatDelay,
  IpcError,
  type GroupInfo,
  type SpeedtestStatus,
} from "../../ipc/client";
import { t } from "../../i18n";
import { clear, el, runWithBusy, toast } from "../kit";
import { store, subscribe } from "../store";

let container: HTMLElement | null = null;
let groups: GroupInfo[] = [];
let speed: SpeedtestStatus | null = null;
let lastSig = "";
let pollTimer = 0;

export function mount(root: HTMLElement): void {
  container = root;
  render();
  subscribe("runtime", () => {
    render();
    // 内核刚起来时自动拉一次节点
    if (store.coreRunning && groups.length === 0) void reload();
  });
  subscribe("nodes", () => void reload());
  window.addEventListener("beforeunload", () => window.clearInterval(pollTimer));
}

export function refresh(): void {
  void reload();
}

async function reload(): Promise<void> {
  if (!store.coreRunning) {
    groups = [];
    render();
    return;
  }
  try {
    groups = await api.nodeGroups();
  } catch (e) {
    const err = e as IpcError;
    // 内核刚起时 API 可能还没就绪，不打扰用户
    if (err.code !== "E_CORE_NOT_RUNNING") toast(err.message, "err");
    groups = [];
  }
  render();
}

function render(): void {
  if (!container) return;

  const sig = JSON.stringify({
    g: groups.map((x) => [x.name, x.now, x.nodes.length]),
    running: store.coreRunning,
    s: speed ? [speed.running, speed.finished, speed.okCount] : null,
  });
  if (sig === lastSig) return;
  lastSig = sig;

  clear(container);

  const btnTest = el("button", { class: "btn primary", text: t("nodes.test") });
  const btnAuto = el("button", { class: "btn", text: "自动应用最优" });
  const btnRefresh = el("button", { class: "btn ghost sm", text: t("common.refresh") });

  btnTest.disabled = !store.coreRunning || (speed?.running ?? false);
  btnRefresh.disabled = !store.coreRunning;

  btnTest.addEventListener("click", () => void runSpeedtest(btnTest));
  btnAuto.addEventListener("click", () =>
    runWithBusy(btnAuto, async () => {
      const recs = await api.switchAutoApply();
      if (recs.length === 0) return { ok: true, message: "当前已是最优节点" };
      await reload();
      return recs[0];
    }, { okMsg: "已应用最优节点" }),
  );
  btnAuto.disabled = !store.coreRunning || groups.length === 0;
  btnRefresh.addEventListener("click", () => void reload());

  container.append(
    el("div", { class: "page-head" }, [
      el("div", {}, [
        el("h2", { text: t("nodes.title") }),
        el("div", { class: "page-desc", text: t("nodes.desc") }),
      ]),
      el("div", { class: "row" }, [btnRefresh, btnAuto, btnTest]),
    ]),
  );

  // ---- 测速进度条 ----
  if (speed && (speed.running || (speed.total > 0 && speed.finished < speed.total))) {
    const pct = speed.total > 0 ? (speed.finished / speed.total) * 100 : 0;
    container.append(
      el("div", { class: "card", style: "padding:10px 14px" }, [
        el("div", { class: "row between small dim" }, [
          el("span", { text: t("nodes.progress", { done: speed.finished, total: speed.total }) }),
          el("span", { class: "mono", text: `${pct.toFixed(0)}%` }),
        ]),
        el("div", { class: "progress", style: "margin-top:6px" }, [
          el("i", { style: `width:${pct.toFixed(1)}%` }),
        ]),
      ]),
    );
  }

  // ---- 无内核 ----
  if (!store.coreRunning) {
    container.append(el("div", { class: "empty", text: t("nodes.needCore") }));
    return;
  }

  // ---- 无节点 ----
  if (groups.length === 0) {
    container.append(el("div", { class: "empty", text: t("nodes.empty") }));
    return;
  }

  // ---- 组列表 ----
  for (const g of groups) {
    container.append(renderGroup(g));
  }
}

function renderGroup(g: GroupInfo): HTMLElement {
  const head = el("div", { class: "group-head" }, [
    el("span", { text: g.name }),
    el("span", { class: "small dim", text: `${g.nodeCount} 个节点 · 当前 ${g.now ?? "—"}` }),
  ]);

  const body = el("div", { class: "group-body" });

  for (const n of g.nodes) {
    const row = el("div", {
      class: `node-row${n.selected ? " selected" : ""}`,
      title: n.error ?? n.name,
    }, [
      el("span", { class: "node-rank", text: n.rank != null ? String(n.rank) : "—" }),
      el("span", { class: "node-name", text: n.name }),
      el("span", { class: `node-delay ${delayClass(n.delayMs)}`, text: formatDelay(n.delayMs) }),
      el("span", { class: "small", text: n.selected ? "✓" : "" }),
    ]);

    row.addEventListener("click", async () => {
      if (n.selected) return;
      row.style.opacity = "0.5";
      try {
        await api.nodeSelect(g.name, n.name);
        toast(t("nodes.switched", { node: n.name }), "ok");
        await reload();
      } catch (e) {
        toast((e as IpcError).message, "err");
      } finally {
        row.style.opacity = "";
      }
    });

    body.append(row);
  }

  return el("div", {}, [head, body]);
}

async function runSpeedtest(btn: HTMLButtonElement): Promise<void> {
  speed = { running: true, runId: null, total: 0, finished: 0, okCount: 0, startedAt: null, finishedAt: null, bestDelay: null };
  lastSig = "";
  render();

  // 轮询进度（后端每 5 个节点发事件，但轮询更可靠且开销小）
  window.clearInterval(pollTimer);
  pollTimer = window.setInterval(async () => {
    try {
      speed = await api.speedtestStatus();
      lastSig = "";
      render();
      if (!speed.running) {
        window.clearInterval(pollTimer);
        await reload();
      }
    } catch {
      window.clearInterval(pollTimer);
    }
  }, 400);

  try {
    speed = await api.speedtestRun();
    window.clearInterval(pollTimer);
    toast(
      speed.bestDelay != null ? t("nodes.best", { ms: speed.bestDelay }) : `测速完成：${speed.okCount}/${speed.total} 可用`,
      speed.okCount > 0 ? "ok" : "warn",
    );
    await reload();
  } catch (e) {
    window.clearInterval(pollTimer);
    speed = null;
    lastSig = "";
    render();
    toast((e as IpcError).message, "err");
  } finally {
    btn.disabled = false;
  }
}
