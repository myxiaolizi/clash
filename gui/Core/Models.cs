using System.Text.Json;
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

    /// <summary>
    /// 内核分流模式："rule" | "global" | "direct"。
    /// 与 <see cref="ProxyMode"/> 正交——前者决定「进内核后怎么走」，
    /// 后者决定「流量怎么进内核」。首页的模式切换只动这个。
    /// </summary>
    public string MihomoMode { get; set; } = "rule";

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

    // ---- 0.1.1 新增（后端未实现时保持默认值，不影响序列化回传） ----

    public bool FailoverEnabled { get; set; }

    /// <summary>"switch" | "retry" | "none"。</summary>
    public string OnFailureAction { get; set; } = "switch";

    public int RetryIntervalSec { get; set; } = 10;
    public int MaxRetries { get; set; } = 3;
    public int DwellAfterSwitchSec { get; set; } = 300;
    public bool PreferHealthyOnly { get; set; }

    /// <summary>
    /// 深拷贝后改单个字段再整体回传。
    /// Rust 的 PolicyDto **没有** serde default，收到缺字段的补丁会直接报
    /// 「参数不合法: missing field xxx」，所以绝不能用匿名对象只传一个字段。
    /// </summary>
    public SwitchPolicy Clone() => new()
    {
        Enabled = Enabled,
        Scope = new SwitchScope { Group = Scope.Group },
        ToleranceMs = ToleranceMs,
        MinDwellSec = MinDwellSec,
        FailureThreshold = FailureThreshold,
        ManualOverrideMin = ManualOverrideMin,
        ExcludeKeywords = new List<string>(ExcludeKeywords),
        PreferRegionPriority = new List<string>(PreferRegionPriority),
        FailoverEnabled = FailoverEnabled,
        OnFailureAction = OnFailureAction,
        RetryIntervalSec = RetryIntervalSec,
        MaxRetries = MaxRetries,
        DwellAfterSwitchSec = DwellAfterSwitchSec,
        PreferHealthyOnly = PreferHealthyOnly,
    };
}

public sealed class SwitchScope
{
    public string Group { get; set; } = "Proxy";
}

// ============================================================================
//  内核 / 运行时
// ============================================================================

/// <summary>core_status 返回。state: stopped / starting / running / stopping / failed。</summary>
public sealed class CoreStatus
{
    public string State { get; set; } = "stopped";
    public int? MixedPort { get; set; }
    public int? ApiPort { get; set; }
    public string? Version { get; set; }
    public long? UptimeSec { get; set; }
    public string? Error { get; set; }

    /// <summary>
    /// 内核二进制是否真实存在。老版本 daemon 不返回该字段，反序列化后为 false，
    /// 因此**不能**单凭它判定「未安装」——见 <see cref="InstalledOrRunning"/>。
    /// </summary>
    public bool Installed { get; set; }

    /// <summary>磁盘上实际可用的最新版本（未安装时为 null）。</summary>
    public string? InstalledVersion { get; set; }

    public bool Running => State == "running";
    public bool Starting => State == "starting";

    /// <summary>
    /// 综合判定「内核可用」：installed 为 true，或已经在运行/启动中
    /// （运行中必然装着，兼容不返回 installed 的旧 daemon）。
    /// </summary>
    public bool InstalledOrRunning => Installed || Running || Starting;

    /// <summary>实际版本号：优先用磁盘上探测到的，回退设置里记的。</summary>
    public string? EffectiveVersion
        => !string.IsNullOrWhiteSpace(InstalledVersion) ? InstalledVersion : Version;

    /// <summary>给 UI 的一句话状态文案。</summary>
    public string DisplayText => State switch
    {
        "running" => MixedPort is int mp
            ? $"运行中 · 127.0.0.1:{mp}"
            : "运行中",
        "starting" => "启动中…",
        "stopping" => "停止中…",
        "failed" => string.IsNullOrWhiteSpace(Error) ? "启动失败" : $"启动失败：{Error}",
        _ => InstalledOrRunning
            ? (EffectiveVersion is { Length: > 0 } v ? $"已安装 · 未运行（{v}）" : "已安装 · 未运行")
            : "未安装",
    };
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

/// <summary>
/// subscription_list 的单条。字段名逐字对齐 Rust 的 SubscriptionDto。
/// 注意：地址只给 <see cref="UrlHint"/>（脱敏后的提示），daemon 绝不返回 token。
/// </summary>
public sealed class Subscription
{
    public string Uid { get; set; } = "";
    public string Name { get; set; } = "";

    /// <summary>脱敏后的地址提示，形如 https://a.com/api/v1/******。</summary>
    public string UrlHint { get; set; } = "";

    public bool Enabled { get; set; }
    public int UpdateIntervalMin { get; set; }
    public long? LastUpdateAt { get; set; }

    /// <summary>"never" | "ok" | "failed"。</summary>
    public string LastUpdateStatus { get; set; } = "never";

    public string? LastError { get; set; }
    public int NodeCount { get; set; }
    public int SortOrder { get; set; }
    public SubscriptionTraffic? Traffic { get; set; }

    // ---- 0.1.1 新增的管理字段（后端未实现时全部为 null/false，UI 自动隐藏） ----

    public string? Username { get; set; }

    /// <summary>是否已保存密码。daemon 不返回明文密码。</summary>
    public bool HasPassword { get; set; }

    /// <summary>机场地址（官网 / 管理后台）。</summary>
    public string? Homepage { get; set; }

    public string? Remark { get; set; }

    public bool NeverUpdated => LastUpdateStatus == "never";
    public bool UpdateFailed => LastUpdateStatus == "failed";
}

/// <summary>TrafficDto：单条订阅的流量与到期信息，全部可空。</summary>
public sealed class SubscriptionTraffic
{
    public long? Upload { get; set; }
    public long? Download { get; set; }
    public long? Total { get; set; }
    public long? Used { get; set; }
    public float? UsedPercent { get; set; }
    public long? ExpireAt { get; set; }
    public long? DaysLeft { get; set; }

    /// <summary>"header" | "web" | "unknown"。</summary>
    public string Source { get; set; } = "unknown";

    /// <summary>数据是否过期（daemon 判定）。</summary>
    public bool Stale { get; set; }

    public bool HasTotal => Total is > 0;

    /// <summary>总量为 0 表示不限量（附录 B.3：total=0 是有效值，不是未知）。</summary>
    public bool Unlimited => Total == 0;
}

public sealed class UserInfo
{
    public long? Upload { get; set; }
    public long? Download { get; set; }
    public long? Total { get; set; }
    public long? Expire { get; set; }
}

// ============================================================================
//  规则（0.1.1 新增，后端并行实现）
// ============================================================================

/// <summary>
/// rule_list / rule_add / rule_update 的单条，对应 Rust 的 UserRule / RuleDto。
/// </summary>
public sealed class Rule
{
    /// <summary>
    /// 稳定 id（Rust 侧是 String）。加宽松转换器：万一后端给数字也不会让整页解析失败。
    /// </summary>
    [JsonConverter(typeof(FlexibleTextConverter))]
    public string Id { get; set; } = "";

    /// <summary>"domain" | "domain_suffix" | "domain_keyword" | "ip" | "process"。</summary>
    public string Kind { get; set; } = "domain";

    /// <summary>匹配值：域名 / 后缀 / 关键字 / CIDR / 进程名。</summary>
    public string Value { get; set; } = "";

    /// <summary>"proxy" | "direct" | "block"。</summary>
    public string Policy { get; set; } = "proxy";

    public bool Enabled { get; set; } = true;
    public int SortOrder { get; set; }

    /// <summary>
    /// 后端生成的 mihomo 规则行原文，如 <c>DOMAIN-SUFFIX,example.com,DIRECT</c>。
    /// 直接展示给用户，让他确认自己写的规则最终变成了什么。
    /// </summary>
    public string Line { get; set; } = "";

    public bool HasId => !string.IsNullOrEmpty(Id);

    public bool HasLine => !string.IsNullOrWhiteSpace(Line);

    /// <summary>规则类型的中文名。</summary>
    public static string KindLabel(string kind) => kind switch
    {
        "domain" => "域名",
        "domain_suffix" => "域名后缀",
        "domain_keyword" => "域名关键词",
        "ip" => "IP 段",
        "process" => "进程名",
        _ => kind,
    };

    /// <summary>去向的中文名。</summary>
    public static string PolicyLabel(string policy) => policy switch
    {
        "proxy" => "代理",
        "direct" => "直连",
        "block" => "阻止",
        _ => policy,
    };

    public string KindText => KindLabel(Kind);
    public string PolicyText => PolicyLabel(Policy);
}

// ============================================================================
//  流量统计（0.1.1 新增，后端并行实现）
// ============================================================================

/// <summary>
/// traffic_stats 返回，对应 Rust <c>TrafficStats</c>。
/// 字段全部按字节计。后端未实现时整页降级，不会崩。
/// </summary>
public sealed class TrafficStats
{
    public long TotalUp { get; set; }
    public long TotalDown { get; set; }
    public long TodayUp { get; set; }
    public long TodayDown { get; set; }

    /// <summary>
    /// 本次运行（会话）上下行。Rust 的 TrafficStats 本身没有这两个字段，
    /// 由 runtime_get 的 trafficUp/trafficDown 补充，缺失时为 null。
    /// </summary>
    public long? SessionUp { get; set; }
    public long? SessionDown { get; set; }

    /// <summary>最近 N 小时分桶，按 hour 升序。</summary>
    public List<HourlyTraffic> Hourly { get; set; } = new();

    public long Today => TodayUp + TodayDown;
    public long Total => TotalUp + TotalDown;
    public long Session => (SessionUp ?? 0) + (SessionDown ?? 0);
    public bool HasSession => SessionUp is not null || SessionDown is not null;
}

/// <summary>趋势图的一个点，对应 Rust <c>HourlyTraffic</c>。</summary>
public sealed class HourlyTraffic
{
    /// <summary>
    /// 小时起点（Unix 秒，已对齐整点）。Rust 是 i64；
    /// 宽松转换器兼容后端改成字符串的情况。
    /// </summary>
    [JsonConverter(typeof(FlexibleLongConverter))]
    public long Hour { get; set; }

    public long Up { get; set; }
    public long Down { get; set; }

    public long Total => Up + Down;

    /// <summary>本地时间的 "HH:00" 标签。</summary>
    public string HourLabel
    {
        get
        {
            if (Hour <= 0) return "";
            try
            {
                return DateTimeOffset.FromUnixTimeSeconds(Hour).LocalDateTime.ToString("HH:00");
            }
            catch
            {
                return "";
            }
        }
    }
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

    /// <summary>内核分流模式："rule" | "global" | "direct"。</summary>
    public string? MihomoMode { get; set; }

    /// <summary>
    /// 切换策略补丁。
    ///
    /// ⚠ Rust 侧 <c>SettingsPatch.switch_policy: Option&lt;PolicyDto&gt;</c>，
    /// PolicyDto **没有**对全部字段加 serde default，只传单个字段会报
    /// 「参数不合法: missing field `toleranceMs`」。要改策略必须传完整对象，
    /// 见 <see cref="SwitchPolicy.Clone"/>。
    /// </summary>
    public SwitchPolicy? SwitchPolicy { get; set; }

    public int? ApiPort { get; set; }
    public string? CoreVersion { get; set; }
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

// ============================================================================
//  宽松转换器
// ============================================================================

/// <summary>
/// 把后端可能给出的数字 / 字符串统一读成字符串。
/// 用于 traffic_stats.hourly[].hour 这类「契约未钉死标量类型」的字段：
/// 后端给 14、给 "14"、给 "2026-11-07T14:00:00+08:00" 都不应该让整页解析失败。
/// </summary>
public sealed class FlexibleTextConverter : JsonConverter<string>
{
    public override string Read(ref Utf8JsonReader reader, Type typeToConvert,
                                 JsonSerializerOptions options)
        => reader.TokenType switch
        {
            JsonTokenType.String => reader.GetString() ?? "",
            JsonTokenType.Number => reader.TryGetInt64(out var l)
                ? l.ToString()
                : reader.GetDouble().ToString("0.##"),
            JsonTokenType.True => "true",
            JsonTokenType.False => "false",
            JsonTokenType.Null => "",
            _ => "",
        };

    public override void Write(Utf8JsonWriter writer, string value,
                               JsonSerializerOptions options)
        => writer.WriteStringValue(value);
}

/// <summary>
/// 把后端可能给出的字符串 / 数字统一读成 long。
/// 用于 hourly[].hour 这类「数值但后端可能改成字符串」的字段。
/// 读不出来就返回 0，绝不抛异常打断整页解析。
/// </summary>
public sealed class FlexibleLongConverter : JsonConverter<long>
{
    public override long Read(ref Utf8JsonReader reader, Type typeToConvert,
                              JsonSerializerOptions options)
        => reader.TokenType switch
        {
            JsonTokenType.Number => reader.TryGetInt64(out var l) ? l : (long)reader.GetDouble(),
            JsonTokenType.String => long.TryParse(reader.GetString(), out var p) ? p : 0L,
            JsonTokenType.Null => 0L,
            _ => 0L,
        };

    public override void Write(Utf8JsonWriter writer, long value,
                               JsonSerializerOptions options)
        => writer.WriteNumberValue(value);
}
