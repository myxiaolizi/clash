/**
 * 流量与到期视图。
 */

import {
  api,
  formatBytes,
  formatDate,
  formatDaysLeft,
  formatPercent,
  IpcError,
  type ItemTraffic,
  type TrafficSummary,
} from "../../ipc/client";
import { t } from "../../i18n";
import { clear, el, runWithBusy, toast } from "../kit";
import { refreshTraffic, store, subscribe } from "../store";

let container: HTMLElement | null = null;
let lastSig = "";

export function mount(root: HTMLElement): void {
  container = root;
  render();
  subscribe("traffic", () => render());
}

export function refresh(): void {
  void refreshTraffic();
}

function render(): void {
  if (!container) return;
  const d = store.traffic;

  const sig = JSON.stringify(d);
  if (sig === lastSig) return;
  lastSig = sig;

  clear(container);

  const btnRefresh = el("button", { class: "btn", text: t("traffic.refresh") });
  btnRefresh.addEventListener("click", () =>
    runWithBusy(btnRefresh, () => refreshTraffic(), { okMsg: t("common.saved") }),
  );

  container.append(
    el("div", { class: "page-head" }, [
      el("div", {}, [
        el("h2", { text: t("traffic.title") }),
        el("div", { class: "page-desc", text: t("traffic.desc") }),
      ]),
      btnRefresh,
    ]),
  );

  if (!d || d.items.length === 0) {
    container.append(el("div", { class: "empty", text: t("traffic.empty") }));
    return;
  }

  // ---- 顶部汇总 ----
  container.append(renderHero(d));

  // ---- 明细 ----
  if (d.items.length > 1) {
    for (const it of d.items) {
      container.append(renderItemCard(it));
    }
  }

  if (d.hasMissing) {
    container.append(el("div", { class: "faint small", style: "padding:4px 2px", text: t("traffic.missing") }));
  }
}

function renderHero(d: TrafficSummary): HTMLElement {
  const used = d.used ?? 0;
  const total = d.total;
  const pct = d.usedPercent;
  const noLimit = total == null;

  const barClass = pct == null ? "" : pct >= 90 ? "bad" : pct >= 70 ? "warn" : "";

  const hero = el("div", { class: "traffic-hero" }, [
    el("div", { class: "row between" }, [
      el("div", {}, [
        el("div", { class: "traffic-num", text: formatBytes(used) }),
        el("div", {
          class: "traffic-sub",
          text: noLimit
            ? t("traffic.noLimit")
            : `${t("traffic.total")} ${formatBytes(total)} · ${formatPercent(pct)}`,
        }),
      ]),
      d.daysLeft != null
        ? el("div", { style: "text-align:right" }, [
            el("div", {
              class: `traffic-num ${d.daysLeft < 0 ? "delay-bad" : d.daysLeft <= 3 ? "delay-slow" : ""}`,
              style: "font-size:20px",
              text: d.daysLeft < 0 ? t("traffic.expired") : `${d.daysLeft}`,
            }),
            el("div", { class: "traffic-sub", text: d.daysLeft < 0 ? formatDate(d.expireAt) : t("traffic.daysLeft") }),
          ])
        : null,
    ]),
  ]);

  if (!noLimit && pct != null) {
    hero.append(
      el("div", { class: `progress ${barClass}`, style: "margin-top:14px" }, [
        el("i", { style: `width:${Math.min(100, Math.max(0, pct)).toFixed(2)}%` }),
      ]),
    );
  }

  hero.append(
    el("div", { class: "traffic-grid" }, [
      cell(t("traffic.upload"), formatBytes(d.upload)),
      cell(t("traffic.download"), formatBytes(d.download)),
      cell(t("traffic.total"), noLimit ? t("traffic.noLimit") : formatBytes(total)),
      cell(t("traffic.expire"), d.expireAt == null ? t("traffic.unlimited") : formatDate(d.expireAt)),
    ]),
  );

  return hero;
}

function renderItemCard(it: ItemTraffic): HTMLElement {
  const pct = it.usedPercent;
  const barClass = pct == null ? "" : pct >= 90 ? "bad" : pct >= 70 ? "warn" : "";

  const card = el("div", { class: "card" }, [
    el("div", { class: "row between" }, [
      el("div", { class: "row" }, [
        el("strong", { text: it.name }),
        it.stale ? el("span", { class: "badge warn", text: "已过期" }) : null,
      ]),
      el("div", { class: "small dim mono", text: formatDaysLeft(it.daysLeft) }),
    ]),
    el("div", { class: "row between", style: "margin-top:8px" }, [
      el("span", { class: "small dim", text: `${formatBytes(it.used)} / ${formatBytes(it.total)}` }),
      el("span", { class: "small dim mono", text: formatPercent(pct) }),
    ]),
  ]);

  if (it.total != null && pct != null) {
    card.append(
      el("div", { class: `progress ${barClass}`, style: "margin-top:8px" }, [
        el("i", { style: `width:${Math.min(100, Math.max(0, pct)).toFixed(2)}%` }),
      ]),
    );
  }

  return card;
}

function cell(k: string, v: string): HTMLElement {
  return el("div", { class: "traffic-cell" }, [
    el("div", { class: "k", text: k }),
    el("div", { class: "v", text: v }),
  ]);
}

/** 供外部使用，避免未使用告警 */
export const _internal = { api, toast, IpcError };
