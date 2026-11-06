/**
 * 订阅视图。
 */

import {
  api,
  formatRelative,
  errorHint,
  IpcError,
  type Subscription,
} from "../../ipc/client";
import { t } from "../../i18n";
import { alertBox, clear, el, modal, runWithBusy, toast } from "../kit";
import { refreshTraffic, store, subscribe } from "../store";

let container: HTMLElement | null = null;
let subs: Subscription[] = [];
let lastSig = "";

export function mount(root: HTMLElement): void {
  container = root;
  render();
  subscribe("subscriptions", () => void reload());
  subscribe("runtime", () => render());
}

async function reload(): Promise<void> {
  try {
    subs = await api.subList();
  } catch (e) {
    toast((e as IpcError).message, "err");
    subs = [];
  }
  render();
}

export function refresh(): void {
  void reload();
}

function render(): void {
  if (!container) return;

  // 签名去重：内核状态每 2s 轮询触发重绘，但列表没变时不该重排 DOM
  const sig = JSON.stringify({
    s: subs.map((x) => [x.uid, x.name, x.enabled, x.nodeCount, x.lastUpdateStatus, x.lastUpdateAt, x.traffic?.used]),
    core: store.coreRunning,
  });
  if (sig === lastSig) return;
  lastSig = sig;

  clear(container);

  // ---- 页头 ----
  const btnAdd = el("button", { class: "btn primary", text: t("sub.add") });
  btnAdd.addEventListener("click", () => void openAddDialog());

  const btnAll = el("button", { class: "btn", text: t("sub.updateAll") });
  btnAll.addEventListener("click", () =>
    runWithBusy(btnAll, async () => {
      const r = await api.subFetchAll();
      await reload();
      await refreshTraffic();
      return r;
    }, { errPrefix: t("sub.updateAll") }),
  );
  btnAll.disabled = subs.length === 0;

  const btnApply = el("button", { class: "btn", text: t("sub.apply") });
  btnApply.addEventListener("click", () =>
    runWithBusy(btnApply, () => api.subApply(), { okMsg: t("sub.applied") }),
  );
  btnApply.disabled = !store.coreRunning || subs.length === 0;
  if (!store.coreRunning) {
    btnApply.title = t("nodes.needCore");
  }

  container.append(
    el("div", { class: "page-head" }, [
      el("div", {}, [
        el("h2", { text: t("sub.title") }),
        el("div", { class: "page-desc", text: t("sub.desc") }),
      ]),
      el("div", { class: "row" }, [btnApply, btnAll, btnAdd]),
    ]),
  );

  // ---- 列表 ----
  if (subs.length === 0) {
    container.append(el("div", { class: "empty", text: t("sub.empty") }));
    return;
  }

  for (const s of subs) {
    container.append(renderItem(s));
  }
}

function renderItem(s: Subscription): HTMLElement {
  const isFailed = s.lastUpdateStatus === "failed";

  const statusBadge = el("span", {
    class: `badge ${isFailed ? "err" : s.lastUpdateStatus === "ok" ? "ok" : "dim"}`,
    text: isFailed ? "失败" : s.lastUpdateStatus === "ok" ? "正常" : "未更新",
  });

  // ---- 开关 ----
  const toggle = el("input", {
    type: "checkbox",
    checked: s.enabled,
  }) as HTMLInputElement;
  toggle.disabled = !s.enabled && false;
  toggle.addEventListener("change", async () => {
    const prev = s.enabled;
    toggle.disabled = true;
    try {
      await api.subUpdate(s.uid, { enabled: toggle.checked });
      s.enabled = toggle.checked;
      notifyLocal();
      toast(t("sub.updateOk"), "ok");
    } catch (e) {
      toggle.checked = prev;
      toast((e as IpcError).message, "err");
    } finally {
      toggle.disabled = false;
    }
  });

  // ---- 操作按钮 ----
  const btnUpdate = el("button", { class: "btn sm", text: t("sub.update") });
  btnUpdate.addEventListener("click", () =>
    runWithBusy(btnUpdate, async () => {
      await api.subFetch(s.uid, true);
      await reload();
      await refreshTraffic();
    }, { okMsg: t("sub.updateOk") }),
  );

  const btnEdit = el("button", { class: "btn sm ghost", text: t("sub.edit") });
  btnEdit.addEventListener("click", () => void openEditDialog(s));

  const btnDel = el("button", { class: "btn sm ghost danger", text: t("sub.delete") });
  btnDel.addEventListener("click", async () => {
    const yes = await modal({
      title: t("sub.delete"),
      body: t("sub.deleteConfirm", { name: s.name }),
      okText: t("common.delete"),
      danger: true,
    });
    if (!yes) return;
    try {
      await api.subRemove(s.uid);
      toast(t("sub.deleteOk"), "ok");
      await reload();
      await refreshTraffic();
    } catch (e) {
      toast((e as IpcError).message, "err");
    }
  });

  // ---- 元信息 ----
  const meta: (HTMLElement)[] = [
    el("span", { text: t("sub.nodes", { n: s.nodeCount }) }),
    el("span", { text: `${t("sub.lastUpdate")}：${formatRelative(s.lastUpdateAt)}` }),
    el("span", { text: t("sub.intervalFmt", { n: s.updateIntervalMin }) }),
  ];
  if (s.traffic?.used != null && s.traffic.total != null) {
    const pct = s.traffic.usedPercent ?? 0;
    meta.push(
      el("span", {
        class: pct >= 90 ? "delay-bad" : pct >= 70 ? "delay-slow" : "delay-good",
        text: `${pct.toFixed(1)}%`,
      }),
    );
  }

  const head = el("div", { class: "sub-name" }, [
    el("span", { text: s.name }),
    statusBadge,
    s.enabled ? null : el("span", { class: "badge dim", text: "已停用" }),
  ]);

  const item = el("div", { class: `sub-item${s.enabled ? "" : " disabled"}` }, [
    el("div", { class: "grow" }, [
      head,
      el("div", { class: "sub-url ellipsis", text: s.urlHint, title: s.urlHint }),
      el("div", { class: "sub-meta" }, meta),
    ]),
    el("div", { class: "sub-actions" }, [
      el("label", { class: "switch", title: t("sub.enabled") }, [toggle, el("span", { class: "track" })]),
      btnUpdate,
      btnEdit,
      btnDel,
    ]),
  ]);

  if (isFailed && s.lastError) {
    item.append(el("div", { class: "sub-error" }, [
      el("span", { text: s.lastError }),
      errorHint(s.lastError) ? el("span", { class: "faint small", text: ` · ${errorHint(s.lastError)}` }) : null,
    ]));
  }

  return item;
}

function notifyLocal(): void {
  lastSig = ""; // 强制下次重绘
  render();
}

// ============================================================================
//  对话框：添加 / 编辑
// ============================================================================

function openAddDialog(): void {
  const nameInput = el("input", {
    type: "text",
    placeholder: t("sub.namePh"),
  }) as HTMLInputElement;

  const urlInput = el("input", {
    type: "url",
    placeholder: t("sub.urlPh"),
    spellcheck: false,
  }) as HTMLInputElement;

  const intervalInput = el("input", {
    type: "number",
    value: String(store.settings?.subDefaultIntervalMin ?? 360),
    min: "15",
    step: "15",
  }) as HTMLInputElement;

  const errLine = el("div", { class: "sub-error", style: "display:none" });

  const body = el("div", {}, [
    el("label", { class: "field" }, [
      el("span", { text: t("sub.name") }),
      nameInput,
    ]),
    el("label", { class: "field" }, [
      el("span", { text: t("sub.url") }),
      urlInput,
    ]),
    el("label", { class: "field" }, [
      el("span", { text: t("sub.interval") }),
      intervalInput,
    ]),
    errLine,
    el("div", {
      class: "small faint",
      style: "margin-top:8px",
      text: "链接会用 Windows DPAPI 加密后保存在本机，不会明文落盘，也不会写进日志。",
    }),
  ]);

  void modal({
    title: t("sub.add"),
    body,
    okText: t("common.save"),
  }).then(async (ok) => {
    if (!ok) return;
    const name = nameInput.value.trim();
    const url = urlInput.value.trim();

    errLine.style.display = "none";
    if (!name) {
      showErr(errLine, t("sub.needName"));
      return;
    }
    if (!/^https?:\/\//i.test(url)) {
      showErr(errLine, t("sub.invalidUrl"));
      return;
    }

    try {
      const sub = await api.subAdd(name, url);
      // 自动设周期
      const iv = parseInt(intervalInput.value, 10);
      if (isFinite(iv) && iv !== sub.updateIntervalMin) {
        await api.subUpdate(sub.uid, { intervalMin: iv }).catch(() => undefined);
      }
      toast(t("sub.addOk"), "ok");
      await reload();
      // 添加后立刻拉一次，用户马上看到节点
      try {
        await api.subFetch(sub.uid, false);
        await reload();
        await refreshTraffic();
      } catch (e) {
        toast((e as IpcError).message, "warn");
      }
    } catch (e) {
      showErr(errLine, (e as IpcError).message);
    }
  });
}

function openEditDialog(s: Subscription): void {
  const nameInput = el("input", { type: "text", value: s.name }) as HTMLInputElement;
  const intervalInput = el("input", {
    type: "number",
    value: String(s.updateIntervalMin),
    min: "15",
    step: "15",
  }) as HTMLInputElement;
  const errLine = el("div", { class: "sub-error", style: "display:none" });

  const body = el("div", {}, [
    el("label", { class: "field" }, [el("span", { text: t("sub.name") }), nameInput]),
    el("label", { class: "field" }, [
      el("span", { text: t("sub.interval") }),
      intervalInput,
    ]),
    el("div", { class: "small faint", text: `地址：${s.urlHint}` }),
    el("div", {
      class: "small faint",
      style: "margin-top:6px",
      text: "出于安全考虑，已保存的订阅链接不显示明文。如需更换请删除后重新添加。",
    }),
    errLine,
  ]);

  void modal({ title: t("sub.edit"), body, okText: t("common.save") }).then(async (ok) => {
    if (!ok) return;
    errLine.style.display = "none";
    const name = nameInput.value.trim();
    if (!name) {
      showErr(errLine, t("sub.needName"));
      return;
    }
    try {
      await api.subUpdate(s.uid, {
        name,
        intervalMin: parseInt(intervalInput.value, 10) || s.updateIntervalMin,
      });
      toast(t("common.saved"), "ok");
      await reload();
    } catch (e) {
      showErr(errLine, (e as IpcError).message);
    }
  });
}

function showErr(node: HTMLElement, msg: string): void {
  node.textContent = msg;
  node.style.display = "block";
  toast(msg, "err");
}

/** 供外部（设置页）显式使用，避免 lint 报未使用 */
export const _internal = { alertBox };
