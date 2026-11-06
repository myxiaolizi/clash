using System.Text.Json.Serialization;

namespace ClashZhengl.Core;

// ============================================================================
//  与 src-tauri/src/commands/dto.rs 一一对应。
//  字段名走 camelCase，可选值一律用可空类型（Rust 的 Option<T> → null）。
//
//  ⚠ 改动 Rust DTO 必须同步改这里。用非空类型接 Option 字段会在
//    反序列化时抛异常并让整页失败（total=null 曾导致流量页崩溃）。
// ============================================================================

public sealed class AppInfo
{
    public string Version { get; set; } = "";
    public string? CoreVersion { get; set; }
    public string DataDir { get; set; } = "";
    public string LogDir { get; set; } = "";
    public string Os { get; set; } = "";
    public string Arch { get; set; } = "";
}

public sealed class OpResult
{
    public bool Ok { get; set; }
    public string Message { get; set; } = "";
}

// ============================================================================
//  设置
// ============================================================================

public sealed class Settings
{
    public int SchemaVersion { get; set; }
    public string Locale { get; set; } = "zh-CN";
    public string Theme { get; set; } = "system";
    public bool AutoLaunch { get; set; }
    public bool SilentStart { get; set; }
    public bool StartCoreOnLaunch { get; set; }

    /// <summary>是否已接受使用须知（Rust 侧由 compliance_accepted_at.is_some() 派生）。</summary>
    public bool ComplianceAccepted { get; set; }

    public string ProxyMode { get; set; } = "system_proxy";
    public int ApiPort { get; set; }
    public string? CoreVersion { get; set; }
    public string? CoreDownloadProxy { get; set; }
    public int SubDefaultIntervalMin { get; set; } = 360;
    public int SubFetchConcurrency { get; set; } = 2;
    public int SubFetchTimeoutMs { get; set; } = 15000;
    public string? SubFetchProxy { get; set; }
    public string SubUserAgent { get; set; } = "clash.meta/1.19.0";
    public bool SpeedtestEnabled { get; set; } = true;
    public int SpeedtestIntervalMin { get; set; } = 30;
    public string SpeedtestProbeUrl { get; set; } = "http://www.gstatic.com/generate_204";
    public int SpeedtestTimeoutMs { get; set; } = 3000;
    public int SpeedtestConcurrency { get; set; } = 8;
    public SwitchPolicy SwitchPolicy { get; set; } = new();
    public bool NotifyOnSwitch { get; set; }
    public bool NotifyOnExpire { get; set; } = true;
    public int TrafficWarnPercent { get; set; } = 80;
    public string LogLevel { get; set; } = "info";
    public bool CloseToTray { get; set; } = true;
}

public sealed class SwitchPolicy
{
    public bool Enabled { get; set; } = true;
    public SwitchScope Scope { get; set; } = new();
    public int ToleranceMs { get; set; } = 30;
    public int MinDwellSec { get; set; } = 300;
    public int FailureThreshold { get; set; } = 2;
    public int ManualOverrideMin { get; set; } = 60;
    public List<string> ExcludeKeywords { get; set; } = new();
    public List<string> PreferRegionPriority { get; set; } = new();
}

public sealed class SwitchScope
{
    public string Group { get; set; } = "Proxy";
}

// ============================================================================
//  内核 / 运行时
// ============================================================================

/// <summary>core_status 返回。state: running / stopped / starting / crashed。</summary>
public sealed class CoreStatus
{
    public string State { get; set; } = "stopped";
    public int? MixedPort { get; set; }
    public int? ApiPort { get; set; }
    public string? Version { get; set; }
    public long? UptimeSec { get; set; }
    public string? Error { get; set; }
}

/// <summary>
/// runtime_get 返回。注意系统代理字段是**扁平**的
/// （sysproxyEnabled / sysproxyManaged / sysproxyServer），不是嵌套对象。
/// </summary>
public sealed class RuntimeState
{
    public CoreStatus Core { get; set; } = new();
    public string? ActiveSubscription { get; set; }
    public bool SysproxyEnabled { get; set; }
    public bool SysproxyManaged { get; set; }
    public string? SysproxyServer { get; set; }
    public long TrafficUp { get; set; }
    public long TrafficDown { get; set; }
    public long UpRate { get; set; }
    public long DownRate { get; set; }

    public bool CoreRunning => Core.State == "running";
    public int? MixedPort => Core.MixedPort;
}

// ============================================================================
//  订阅
// ============================================================================

public sealed class Subscription
{
    public string Uid { get; set; } = "";
    public string Name { get; set; } = "";
    public string UrlMasked { get; set; } = "";
    public bool Enabled { get; set; }
    public int IntervalMin { get; set; }
    public long? LastFetchAt { get; set; }
    public long? NextFetchAt { get; set; }
    public int? NodeCount { get; set; }
    public string? LastError { get; set; }
    public UserInfo? Userinfo { get; set; }
}

public sealed class UserInfo
{
    public long? Upload { get; set; }
    public long? Download { get; set; }
    public long? Total { get; set; }
    public long? Expire { get; set; }
}

/// <summary>
/// 流量汇总。Total / UsedPercent 可空：订阅没给 userinfo 时 daemon 返回 null。
/// </summary>
public sealed class TrafficSummary
{
    public List<ItemTraffic> Items { get; set; } = new();
    public long Upload { get; set; }
    public long Download { get; set; }
    public long? Total { get; set; }
    public long Used { get; set; }
    public float? UsedPercent { get; set; }
    public long? ExpireAt { get; set; }
    public long? DaysLeft { get; set; }
    public int SubscriptionCount { get; set; }

    /// <summary>有订阅缺少 userinfo，总量无法统计。</summary>
    public bool HasMissing { get; set; }

    /// <summary>统计时间戳。</summary>
    public long At { get; set; }

    public bool HasTotal => Total is > 0;
}

public sealed class ItemTraffic
{
    public string Uid { get; set; } = "";
    public string Name { get; set; } = "";
    public long? Upload { get; set; }
    public long? Download { get; set; }
    public long? Total { get; set; }
    public long? Used { get; set; }
    public long? ExpireAt { get; set; }
    public int? DaysLeft { get; set; }
}

// ============================================================================
//  节点
// ============================================================================

public sealed class NodeGroup
{
    public string Name { get; set; } = "";
    public string? Now { get; set; }
    public int NodeCount { get; set; }
    public List<NodeItem> Nodes { get; set; } = new();

    /// <summary>
    /// 组类型，由 <c>node_group_raw_kind</c> 单独拉取后填入。
    /// GroupDto 本身不带 kind，必须额外请求 —— 不能靠猜。
    /// </summary>
    public string Kind { get; set; } = "";

    /// <summary>
    /// 是否允许手动选节点。只有 Selector 组可以；
    /// 用 Ordinal 严格比较，因为 mihomo 返回的正是 "Selector"（大写 S）。
    /// </summary>
    public bool CanSelect => string.Equals(Kind, "Selector", StringComparison.Ordinal);
}

public sealed class NodeItem
{
    public string Name { get; set; } = "";
    public int? DelayMs { get; set; }
    public bool Ok { get; set; }
    public int? Rank { get; set; }
    public string? Error { get; set; }
    public bool Selected { get; set; }
}

public sealed class SpeedtestStatus
{
    public bool Running { get; set; }
    public string? RunId { get; set; }
    public int Total { get; set; }
    public int Finished { get; set; }
    public int OkCount { get; set; }
    public long? StartedAt { get; set; }
    public long? FinishedAt { get; set; }
    public int? BestDelay { get; set; }
}

public sealed class SwitchRecord
{
    public long At { get; set; }
    public string Group { get; set; } = "";
    public string? FromNode { get; set; }
    public string ToNode { get; set; } = "";
    public int? FromDelay { get; set; }
    public int? ToDelay { get; set; }
    public string Reason { get; set; } = "";
}

public sealed class InstalledCore
{
    public string Version { get; set; } = "";
    public string Path { get; set; } = "";
    public long SizeBytes { get; set; }
    public bool Current { get; set; }
}

public sealed class FetchResult
{
    public string Uid { get; set; } = "";
    public int NodeCount { get; set; }
    public bool Changed { get; set; }
    public UserInfo? Userinfo { get; set; }
}

// ============================================================================
//  请求体（发给 daemon 的参数）
// ============================================================================

public sealed class SettingsPatch
{
    public string? Locale { get; set; }
    public string? Theme { get; set; }
    public bool? AutoLaunch { get; set; }
    public bool? SilentStart { get; set; }
    public bool? StartCoreOnLaunch { get; set; }
    public string? ProxyMode { get; set; }
    public string? CoreDownloadProxy { get; set; }
    public int? SubDefaultIntervalMin { get; set; }
    public int? SubFetchConcurrency { get; set; }
    public int? SubFetchTimeoutMs { get; set; }
    public string? SubFetchProxy { get; set; }
    public string? SubUserAgent { get; set; }
    public bool? SpeedtestEnabled { get; set; }
    public int? SpeedtestIntervalMin { get; set; }
    public string? SpeedtestProbeUrl { get; set; }
    public int? SpeedtestTimeoutMs { get; set; }
    public int? SpeedtestConcurrency { get; set; }
    public bool? NotifyOnSwitch { get; set; }
    public bool? NotifyOnExpire { get; set; }
    public int? TrafficWarnPercent { get; set; }
    public string? LogLevel { get; set; }
    public bool? CloseToTray { get; set; }
}
