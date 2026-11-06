/**
 * 前端共享状态。
 *
 * 单一可变 store + 订阅回调，避免每个视图各拉一遍后端（冷启动与 RSS 都有预算）。
 */

import { api, type Runtime, type Settings, type TrafficSummary } from "../ipc/client";

export interface AppStore {
  settings: Settings | null;
  runtime: Runtime | null;
  traffic: TrafficSummary | null;
  /** 当前视图 */
  view: ViewName;
  /** 内核是否运行 */
  coreRunning: boolean;
  /** 是否正在做耗时操作（全局禁用某些按钮） */
  busy: boolean;
}

export type ViewName = "subscriptions" | "traffic" | "nodes" | "settings";

type Listener = () => void;

const listeners = new Map<string, Set<Listener>>();

export const store: AppStore = {
  settings: null,
  runtime: null,
  traffic: null,
  view: "subscriptions",
  coreRunning: false,
  busy: false,
};

/** 订阅某类变更（"*" 表示全部） */
export function subscribe(topic: string, fn: Listener): () => void {
  let set = listeners.get(topic);
  if (!set) {
    set = new Set();
    listeners.set(topic, set);
  }
  set.add(fn);
  return () => set?.delete(fn);
}

export function notify(topic: string): void {
  listeners.get(topic)?.forEach((fn) => fn());
  listeners.get("*")?.forEach((fn) => fn());
}

/** 刷新设置 */
export async function refreshSettings(): Promise<Settings | null> {
  try {
    store.settings = await api.settingsGet();
    notify("settings");
    return store.settings;
  } catch {
    return null;
  }
}

/** 刷新运行态（内核 / 系统代理） */
export async function refreshRuntime(): Promise<Runtime | null> {
  try {
    const r = await api.runtimeGet();
    store.runtime = r;
    store.coreRunning = r.core.state === "running";
    notify("runtime");
    return r;
  } catch {
    return null;
  }
}

/** 刷新流量 */
export async function refreshTraffic(): Promise<TrafficSummary | null> {
  try {
    store.traffic = await api.subTraffic();
    notify("traffic");
    return store.traffic;
  } catch {
    return null;
  }
}

/** 三路并发刷新（首屏用） */
export async function refreshAll(): Promise<void> {
  await Promise.allSettled([refreshSettings(), refreshRuntime(), refreshTraffic()]);
}
