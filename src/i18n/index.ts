/**
 * 界面文案（中英双语）。
 *
 * 契约 C.1 要求 i18n 覆盖面：不作为功能卖点，但必须完整，
 * 否则英文环境下会出现中英混杂。
 */

export type Locale = "zh-CN" | "en-US";

type Dict = Record<string, string>;

const zh: Dict = {
  "app.name": "Clash Zhengl",

  "nav.subscriptions": "订阅",
  "nav.traffic": "流量",
  "nav.nodes": "节点",
  "nav.settings": "设置",

  "status.core": "内核",
  "status.notStarted": "未启动",
  "status.sysproxy": "系统代理",

  "sub.title": "订阅管理",
  "sub.desc": "添加机场订阅链接，软件会按周期自动更新节点与流量信息。",
  "sub.add": "添加订阅",
  "sub.updateAll": "全部更新",
  "sub.apply": "应用到内核",
  "sub.empty": "还没有订阅。点击右上角「添加订阅」开始。",
  "sub.name": "名称",
  "sub.url": "订阅链接",
  "sub.namePh": "例如：主要机场",
  "sub.urlPh": "https://example.com/api/v1/client/subscribe?token=...",
  "sub.interval": "更新周期",
  "sub.intervalFmt": "{n} 分钟",
  "sub.nodes": "{n} 个节点",
  "sub.lastUpdate": "上次更新",
  "sub.never": "从未更新",
  "sub.enabled": "启用",
  "sub.update": "立即更新",
  "sub.edit": "编辑",
  "sub.delete": "删除",
  "sub.deleteConfirm": "确定删除订阅「{name}」？该操作不会删除已下载的节点缓存。",
  "sub.addOk": "订阅已添加",
  "sub.updateOk": "订阅已更新",
  "sub.deleteOk": "订阅已删除",
  "sub.applied": "配置已应用到内核",
  "sub.invalidUrl": "请填写以 http:// 或 https:// 开头的完整链接",
  "sub.needName": "请填写订阅名称",

  "traffic.title": "流量与到期",
  "traffic.desc": "数据来自订阅响应头（Subscription-Userinfo），机场不提供时显示「—」。",
  "traffic.used": "已用流量",
  "traffic.total": "总流量",
  "traffic.remain": "剩余流量",
  "traffic.upload": "上传",
  "traffic.download": "下载",
  "traffic.expire": "到期时间",
  "traffic.daysLeft": "剩余天数",
  "traffic.noLimit": "不限量",
  "traffic.unlimited": "无限期",
  "traffic.expired": "已过期",
  "traffic.today": "今天到期",
  "traffic.tomorrow": "明天到期",
  "traffic.inDays": "{n} 天后到期",
  "traffic.empty": "暂无流量数据。请先添加订阅并更新。",
  "traffic.missing": "部分订阅未提供流量信息",
  "traffic.refresh": "刷新",

  "nodes.title": "节点与测速",
  "nodes.desc": "点击节点即可切换。测速会并发探测所有节点延迟。",
  "nodes.test": "开始测速",
  "nodes.testing": "测速中…",
  "nodes.autoSwitch": "自动选择最优",
  "nodes.group": "策略组",
  "nodes.selected": "当前",
  "nodes.empty": "暂无节点。请先添加订阅、更新并应用到内核。",
  "nodes.needCore": "内核未运行，无法获取节点",
  "nodes.best": "最快 {ms} ms",
  "nodes.progress": "已完成 {done}/{total}",
  "nodes.switched": "已切换到 {node}",

  "settings.title": "设置",
  "settings.desc": "所有设置立即生效并持久化。",
  "settings.general": "通用",
  "settings.language": "界面语言",
  "settings.theme": "外观主题",
  "settings.theme.system": "跟随系统",
  "settings.theme.light": "浅色",
  "settings.theme.dark": "深色",
  "settings.closeToTray": "关闭窗口时最小化到托盘",
  "settings.autoLaunch": "开机自动启动",
  "settings.silentStart": "静默启动（仅托盘）",

  "settings.core": "内核",
  "settings.coreVersion": "内核版本",
  "settings.coreState": "内核状态",
  "settings.startOnLaunch": "启动时自动运行内核",
  "settings.proxyMode": "代理模式",
  "settings.proxyMode.system": "系统代理",
  "settings.proxyMode.tun": "TUN 模式",
  "settings.proxyMode.manual": "手动",
  "settings.apiPort": "API 端口（0 = 自动）",
  "settings.downloadProxy": "下载代理",
  "settings.downloadProxyHint": "留空表示直连；例如 http://127.0.0.1:1886",
  "settings.installCore": "下载 / 更新内核",
  "settings.coreInstalled": "已安装版本",

  "settings.sub": "订阅",
  "settings.subInterval": "默认更新周期（分钟）",
  "settings.subConcurrency": "拉取并发数",
  "settings.subTimeout": "拉取超时（毫秒）",
  "settings.subProxy": "拉取代理",
  "settings.subProxyHint": "留空表示直连",
  "settings.userAgent": "User-Agent",

  "settings.speedtest": "测速",
  "settings.speedtestEnabled": "启用定时测速",
  "settings.speedtestInterval": "测速周期（分钟）",
  "settings.probeUrl": "探测地址",
  "settings.probeTimeout": "探测超时（毫秒）",
  "settings.concurrency": "并发数",

  "settings.policy": "自动切换策略",
  "settings.policyEnabled": "启用自动切换",
  "settings.tolerance": "切换优势阈值（毫秒）",
  "settings.toleranceHint": "新节点需快出这么多才切换，避免来回跳",
  "settings.minDwell": "最短驻留（秒）",
  "settings.failureThreshold": "连续失败阈值",
  "settings.manualOverride": "手动覆盖时长（分钟）",
  "settings.manualOverrideHint": "手动选择节点后，多久内不自动切换",
  "settings.exclude": "排除关键词",
  "settings.excludeHint": "节点名包含这些词就不参与自动选择（逗号分隔）",
  "settings.regionPriority": "地区优先",
  "settings.regionPriorityHint": "同延迟时优先这些关键词（逗号分隔）",

  "settings.advanced": "高级",
  "settings.logLevel": "日志级别",
  "settings.notifySwitch": "切换节点时通知",
  "settings.notifyExpire": "流量/到期提醒",
  "settings.trafficWarn": "流量告警阈值（%）",
  "settings.dataDir": "数据目录",
  "settings.logDir": "日志目录",
  "settings.openData": "打开数据目录",
  "settings.openLog": "打开日志目录",
  "settings.about": "关于",
  "settings.version": "版本",

  "common.ok": "确定",
  "common.cancel": "取消",
  "common.save": "保存",
  "common.saving": "保存中…",
  "common.saved": "已保存",
  "common.delete": "删除",
  "common.close": "关闭",
  "common.retry": "重试",
  "common.refresh": "刷新",
  "common.loading": "加载中…",
  "common.details": "详情",
  "common.unknown": "未知",

  "compliance.title": "使用须知",
  "compliance.body":
    "本软件是代理内核的图形界面，本身不提供任何代理服务器或网络加速服务。\n\n" +
    "您需要使用自己合法获得的订阅链接。请遵守当地法律法规以及服务提供方的使用条款。\n\n" +
    "作者不对因使用本软件产生的任何后果承担责任。继续使用即表示您已理解并接受上述说明。",
  "compliance.accept": "我已了解并同意",

  "err.title": "操作失败",
  "err.showDetail": "查看技术详情",
};

const en: Dict = {
  "app.name": "Clash Zhengl",

  "nav.subscriptions": "Subscriptions",
  "nav.traffic": "Traffic",
  "nav.nodes": "Nodes",
  "nav.settings": "Settings",

  "status.core": "Core",
  "status.notStarted": "Stopped",
  "status.sysproxy": "System proxy",

  "sub.title": "Subscriptions",
  "sub.desc": "Add your provider subscription links. Nodes and traffic are refreshed automatically.",
  "sub.add": "Add subscription",
  "sub.updateAll": "Update all",
  "sub.apply": "Apply to core",
  "sub.empty": "No subscriptions yet. Click \"Add subscription\" to start.",
  "sub.name": "Name",
  "sub.url": "Subscription URL",
  "sub.namePh": "e.g. Primary provider",
  "sub.urlPh": "https://example.com/api/v1/client/subscribe?token=...",
  "sub.interval": "Update interval",
  "sub.intervalFmt": "{n} min",
  "sub.nodes": "{n} nodes",
  "sub.lastUpdate": "Last update",
  "sub.never": "Never",
  "sub.enabled": "Enabled",
  "sub.update": "Update now",
  "sub.edit": "Edit",
  "sub.delete": "Delete",
  "sub.deleteConfirm": "Delete subscription \"{name}\"? Cached nodes will remain on disk.",
  "sub.addOk": "Subscription added",
  "sub.updateOk": "Subscription updated",
  "sub.deleteOk": "Subscription deleted",
  "sub.applied": "Config applied to core",
  "sub.invalidUrl": "Enter a full URL starting with http:// or https://",
  "sub.needName": "Name is required",

  "traffic.title": "Traffic & Expiry",
  "traffic.desc": "Data comes from the Subscription-Userinfo header. Shown as \"—\" when unavailable.",
  "traffic.used": "Used",
  "traffic.total": "Total",
  "traffic.remain": "Remaining",
  "traffic.upload": "Upload",
  "traffic.download": "Download",
  "traffic.expire": "Expires",
  "traffic.daysLeft": "Days left",
  "traffic.noLimit": "Unlimited",
  "traffic.unlimited": "Never",
  "traffic.expired": "Expired",
  "traffic.today": "Expires today",
  "traffic.tomorrow": "Expires tomorrow",
  "traffic.inDays": "In {n} days",
  "traffic.empty": "No traffic data yet. Add and update a subscription first.",
  "traffic.missing": "Some subscriptions did not report traffic",
  "traffic.refresh": "Refresh",

  "nodes.title": "Nodes & Latency",
  "nodes.desc": "Click a node to select it. The test probes all nodes concurrently.",
  "nodes.test": "Test latency",
  "nodes.testing": "Testing…",
  "nodes.autoSwitch": "Auto-select best",
  "nodes.group": "Group",
  "nodes.selected": "Active",
  "nodes.empty": "No nodes. Add a subscription, update it, then apply to the core.",
  "nodes.needCore": "Core is not running; nodes unavailable",
  "nodes.best": "Best {ms} ms",
  "nodes.progress": "{done}/{total} done",
  "nodes.switched": "Switched to {node}",

  "settings.title": "Settings",
  "settings.desc": "All changes apply immediately and are persisted.",
  "settings.general": "General",
  "settings.language": "Language",
  "settings.theme": "Theme",
  "settings.theme.system": "System",
  "settings.theme.light": "Light",
  "settings.theme.dark": "Dark",
  "settings.closeToTray": "Minimize to tray on close",
  "settings.autoLaunch": "Launch at startup",
  "settings.silentStart": "Silent start (tray only)",

  "settings.core": "Core",
  "settings.coreVersion": "Core version",
  "settings.coreState": "Core state",
  "settings.startOnLaunch": "Start core on launch",
  "settings.proxyMode": "Proxy mode",
  "settings.proxyMode.system": "System proxy",
  "settings.proxyMode.tun": "TUN",
  "settings.proxyMode.manual": "Manual",
  "settings.apiPort": "API port (0 = auto)",
  "settings.downloadProxy": "Download proxy",
  "settings.downloadProxyHint": "Empty means direct; e.g. http://127.0.0.1:1886",
  "settings.installCore": "Download / update core",
  "settings.coreInstalled": "Installed versions",

  "settings.sub": "Subscriptions",
  "settings.subInterval": "Default interval (minutes)",
  "settings.subConcurrency": "Fetch concurrency",
  "settings.subTimeout": "Fetch timeout (ms)",
  "settings.subProxy": "Fetch proxy",
  "settings.subProxyHint": "Empty means direct",
  "settings.userAgent": "User-Agent",

  "settings.speedtest": "Latency testing",
  "settings.speedtestEnabled": "Enable scheduled testing",
  "settings.speedtestInterval": "Interval (minutes)",
  "settings.probeUrl": "Probe URL",
  "settings.probeTimeout": "Probe timeout (ms)",
  "settings.concurrency": "Concurrency",

  "settings.policy": "Auto-switch policy",
  "settings.policyEnabled": "Enable auto-switch",
  "settings.tolerance": "Advantage threshold (ms)",
  "settings.toleranceHint": "A new node must be this much faster to switch",
  "settings.minDwell": "Minimum dwell (seconds)",
  "settings.failureThreshold": "Failure threshold",
  "settings.manualOverride": "Manual override (minutes)",
  "settings.manualOverrideHint": "Grace period after manual selection",
  "settings.exclude": "Exclude keywords",
  "settings.excludeHint": "Nodes matching these are skipped (comma separated)",
  "settings.regionPriority": "Region priority",
  "settings.regionPriorityHint": "Preferred keywords on equal latency (comma separated)",

  "settings.advanced": "Advanced",
  "settings.logLevel": "Log level",
  "settings.notifySwitch": "Notify on switch",
  "settings.notifyExpire": "Traffic / expiry reminders",
  "settings.trafficWarn": "Traffic warning threshold (%)",
  "settings.dataDir": "Data directory",
  "settings.logDir": "Log directory",
  "settings.openData": "Open data directory",
  "settings.openLog": "Open log directory",
  "settings.about": "About",
  "settings.version": "Version",

  "common.ok": "OK",
  "common.cancel": "Cancel",
  "common.save": "Save",
  "common.saving": "Saving…",
  "common.saved": "Saved",
  "common.delete": "Delete",
  "common.close": "Close",
  "common.retry": "Retry",
  "common.refresh": "Refresh",
  "common.loading": "Loading…",
  "common.details": "Details",
  "common.unknown": "Unknown",

  "compliance.title": "Before you start",
  "compliance.body":
    "This software is a graphical front-end for a proxy core. It does not provide any proxy server or acceleration service.\n\n" +
    "You must supply your own legally obtained subscription link. Please comply with local laws and your provider's terms.\n\n" +
    "The author accepts no liability for consequences arising from use of this software. Continuing means you understand and accept this.",
  "compliance.accept": "I understand and agree",

  "err.title": "Operation failed",
  "err.showDetail": "Show technical details",
};

const dicts: Record<Locale, Dict> = { "zh-CN": zh, "en-US": en };

let current: Locale = "zh-CN";

export function setLocale(l: string): void {
  current = l === "en-US" ? "en-US" : "zh-CN";
  document.documentElement.lang = current;
}

export function getLocale(): Locale {
  return current;
}

/**
 * 取文案并替换 `{name}` 占位符。
 *
 * 缺失 key 时返回 key 本身（便于开发期发现漏翻），
 * 而不是返回空串——空串在界面上完全看不出问题。
 */
export function t(key: string, vars?: Record<string, string | number>): string {
  const d = dicts[current] ?? zh;
  let s = d[key] ?? zh[key] ?? key;
  if (vars) {
    for (const [k, v] of Object.entries(vars)) {
      s = s.split(`{${k}}`).join(String(v));
    }
  }
  return s;
}
