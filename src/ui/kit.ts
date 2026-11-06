/**
 * 极简 DOM 构建工具 + 提示/对话框。
 *
 * 不用框架的原因（契约 C.1）：包体 ≤12MB、冷启动 ≤1.2s、RSS ≤90MB。
 * 一个 React 运行时就要 ~140KB gz，且首帧需要 hydration。原生 DOM 直接首帧可用。
 */

type Props = Record<string, unknown> | null;

/** 创建元素：`el("div", { class: "x" }, [child])` */
export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props?: Props,
  children?: (Node | string | null | undefined | false)[],
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (props) {
    for (const [k, v] of Object.entries(props)) {
      if (v == null || v === false) continue;
      if (k === "class") {
        node.className = String(v);
      } else if (k === "text") {
        node.textContent = String(v);
      } else if (k === "html") {
        node.innerHTML = String(v);
      } else if (k.startsWith("on") && typeof v === "function") {
        node.addEventListener(k.slice(2).toLowerCase(), v as EventListener);
      } else if (k === "dataset" && typeof v === "object") {
        Object.assign(node.dataset, v as Record<string, string>);
      } else if (k in node) {
        (node as unknown as Record<string, unknown>)[k] = v;
      } else {
        node.setAttribute(k, String(v));
      }
    }
  }
  if (children) {
    for (const c of children) {
      if (c == null || c === false) continue;
      node.append(typeof c === "string" ? document.createTextNode(c) : c);
    }
  }
  return node;
}

export function clear(node: HTMLElement): void {
  while (node.firstChild) node.removeChild(node.firstChild);
}

// ============================================================================
//  轻提示
// ============================================================================

export type ToastKind = "info" | "ok" | "warn" | "err";

export function toast(message: string, kind: ToastKind = "info", ms = 2600): void {
  const root = document.getElementById("toast-root");
  if (!root) return;
  const node = el("div", { class: `toast ${kind === "info" ? "" : kind}`.trim(), text: message });
  root.append(node);
  const timer = window.setTimeout(() => {
    node.style.opacity = "0";
    node.style.transition = "opacity .18s";
    window.setTimeout(() => node.remove(), 200);
  }, ms);
  // 提前清理防止节点堆积
  node.addEventListener("click", () => {
    window.clearTimeout(timer);
    node.remove();
  });
}

// ============================================================================
//  对话框
// ============================================================================

interface ModalOpts {
  title: string;
  /** 正文：字符串按纯文本处理（防注入） */
  body: string | HTMLElement;
  okText?: string;
  cancelText?: string | null;
  /** 危险操作给红色按钮 */
  danger?: boolean;
  /** 额外内容（如错误详情折叠区） */
  extra?: HTMLElement | null;
}

export function modal(opts: ModalOpts): Promise<boolean> {
  return new Promise((resolve) => {
    const root = document.getElementById("modal-root");
    if (!root) {
      resolve(false);
      return;
    }

    let settled = false;
    const finish = (v: boolean) => {
      if (settled) return;
      settled = true;
      document.removeEventListener("keydown", onKey);
      backdrop.remove();
      resolve(v);
    };

    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && opts.cancelText !== null) finish(false);
      if (e.key === "Enter") finish(true);
    };

    const bodyNode =
      typeof opts.body === "string"
        ? el("div", { text: opts.body })
        : opts.body;

    const foot = el("div", { class: "modal-foot" });
    if (opts.cancelText !== null) {
      foot.append(
        el("button", {
          class: "btn",
          text: opts.cancelText ?? "取消",
          onclick: () => finish(false),
        }),
      );
    }
    foot.append(
      el("button", {
        class: opts.danger ? "btn danger" : "btn primary",
        text: opts.okText ?? "确定",
        onclick: () => finish(true),
      }),
    );

    const dialog = el("div", { class: "modal", role: "dialog", "aria-modal": "true" }, [
      el("div", { class: "modal-head", text: opts.title }),
      el("div", { class: "modal-body" }, [bodyNode, opts.extra ?? null]),
      foot,
    ]);

    const backdrop = el("div", {
      class: "modal-backdrop",
      onclick: (e: Event) => {
        if (e.target === backdrop && opts.cancelText !== null) finish(false);
      },
    }, [dialog]);

    root.append(backdrop);
    document.addEventListener("keydown", onKey);
    // 焦点给确认按钮，支持键盘操作
    (foot.lastElementChild as HTMLElement | null)?.focus();
  });
}

/** 只提示一个按钮的对话框（错误详情等） */
export function alertBox(title: string, body: string | HTMLElement, extra?: HTMLElement): Promise<boolean> {
  return modal({ title, body, okText: "知道了", cancelText: null, extra: extra ?? null });
}

// ============================================================================
//  按钮忙碌态
// ============================================================================

/**
 * 包装异步操作：自动禁用按钮 + 显示 spinner + 统一错误提示。
 *
 * 为什么不写 try/catch 到处重复：30 个命令每个都要，重复 30 次必然漏。
 */
export async function runWithBusy<T>(
  btn: HTMLButtonElement | null,
  fn: () => Promise<T>,
  opts: { okMsg?: string; errPrefix?: string } = {},
): Promise<T | null> {
  const prevHtml = btn?.innerHTML;
  if (btn) {
    btn.disabled = true;
    btn.innerHTML = "";
    btn.append(el("span", { class: "spinner" }));
  }
  try {
    const r = await fn();
    if (opts.okMsg) toast(opts.okMsg, "ok");
    return r;
  } catch (e) {
    const err = e as { code?: string; message?: string; detail?: string | null };
    const msg = err.message ?? String(e);
    toast(opts.errPrefix ? `${opts.errPrefix}：${msg}` : msg, "err");
    return null;
  } finally {
    if (btn) {
      btn.disabled = false;
      if (prevHtml != null) btn.innerHTML = prevHtml;
    }
  }
}

/** 判断两个渲染签名是否相同（用于跳过无变化的重绘） */
export function sameSig(a: string, b: string): boolean {
  return a === b;
}
