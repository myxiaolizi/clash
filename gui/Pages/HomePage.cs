using System.Text.Json;
using ClashZhengl.Core;
using Microsoft.UI;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using Windows.Foundation;
using Windows.UI;

namespace ClashZhengl.Pages;

/// <summary>
/// 首页（默认页）：一眼看清当前状态 + 常用开关。
///
/// 内容：
/// 1. 顶部主卡片：运行状态、代理方式切换（规则/全局/直连）、系统代理开关、内核开关
/// 2. 流量三栏：今日 / 本次运行 / 累计
/// 3. 24 小时趋势图（原生 Polyline 手绘，不引第三方图表库）
///
/// 降级策略：traffic_stats 后端可能还没实现，此时趋势图与流量栏
/// 显示「功能开发中」，其余开关照常可用，整页不崩。
/// </summary>
public sealed class HomePage
{
    private readonly StackPanel _host;

    // 状态缓存
    private CoreStatus _core = new();
    private bool _sysProxyOn;
    private bool _sysProxyManaged;
    private string _proxyMode = "system_proxy";
    private string _mihomoMode = "rule";

    private TrafficStats _stats = new();
    private long _upRate;
    private long _downRate;
    private bool _statsAvailable;

    private bool _ready;

    /// <summary>本页是否在前台。后台页不做定时刷新，省 CPU。</summary>
    private bool _active;

    // 刷新防重入
    private bool _refreshing;

    public HomePage(StackPanel host) => _host = host;

    public async Task InitializeAsync()
    {
        _ready = true;
        _active = true; // 首页是默认页，启动时就在前台
        await RefreshAsync();
    }

    /// <summary>
    /// 低频兜底刷新。事件并不覆盖所有状态变化（例如内核自动重启、
    /// 系统代理被外部程序改动），靠这个定时器保证首页数字不会长期停在旧值。
    /// 只在首页可见时调用，见 <see cref="SetActive"/>。
    /// </summary>
    public async Task TickAsync()
    {
        if (!_ready || !_active) return;
        await RefreshAsync();
    }

    /// <summary>导航切换时告知本页是否在前台，避免后台页空转刷新。</summary>
    public void SetActive(bool active)
    {
        _active = active;
    }

    // ========================================================================
    //  数据
    // ========================================================================

    public async Task RefreshAsync()
    {
        if (_refreshing) return;
        _refreshing = true;
        try
        {
            // 四路数据并行拉，任何一路失败都不影响其它
            var coreTask = SafeCall<CoreStatus>("core_status");
            var rtTask = SafeCall<RuntimeState>("runtime_get");
            var statsTask = SafeCall<TrafficStats>("traffic_stats");
            var setTask = SafeCall<Settings>("settings_get");
            var modeTask = ReadModeAsync();

            await Task.WhenAll(coreTask, rtTask, statsTask, setTask, modeTask);

            if (coreTask.Result is { } core) _core = core;

            if (setTask.Result is { } st)
                _proxyMode = st.ProxyMode;

            // 分流模式以 mode_get 为准（独立的运行时状态，不挂在 settings 上）
            if (modeTask.Result is { } mode)
                _mihomoMode = mode;

            if (rtTask.Result is { } rt)
            {
                _sysProxyOn = rt.SysproxyEnabled;
                _sysProxyManaged = rt.SysproxyManaged;
                _upRate = rt.UpRate;
                _downRate = rt.DownRate;

                if (rt.Core is { } rc) _core = rc;

                // 会话流量优先从 runtime 补，traffic_stats 本身没有这两个字段
                _stats.SessionUp = rt.TrafficUp;
                _stats.SessionDown = rt.TrafficDown;
            }

            var stats = statsTask.Result;
            _statsAvailable = stats is not null;
            if (stats is not null)
            {
                // 保留 runtime 补的会话流量
                stats.SessionUp ??= _stats.SessionUp;
                stats.SessionDown ??= _stats.SessionDown;
                _stats = stats;
            }

            Render();
        }
        finally
        {
            _refreshing = false;
        }
    }

    /// <summary>吞掉 DaemonException，失败返回 null。首页任何一块失败都不能拖垮整页。</summary>
    private static async Task<T?> SafeCall<T>(string cmd) where T : class
    {
        try
        {
            return await App.Daemon.CallAsync<T>(cmd);
        }
        catch (DaemonException e)
        {
            Log.Warn($"首页加载 {cmd} 失败：{e.Code} {e.FriendlyMessage}");
            return null;
        }
        catch (Exception e)
        {
            Log.Warn($"首页加载 {cmd} 异常：{e.Message}");
            return null;
        }
    }

    // ========================================================================
    //  事件入口
    // ========================================================================

    /// <summary>core:state 事件。</summary>
    public void OnCoreEvent(JsonElement data)
    {
        try
        {
            if (data.TryGetProperty("state", out var s) && s.ValueKind == JsonValueKind.String)
            {
                _core.State = s.GetString() ?? "stopped";
            }
            if (data.TryGetProperty("error", out var err) && err.ValueKind == JsonValueKind.String)
            {
                _core.Error = err.GetString();
            }
            if (_ready) Render();
        }
        catch (Exception e)
        {
            Log.Warn($"首页处理 core 事件失败：{e.Message}");
        }
    }

    /// <summary>sysproxy:changed 事件。</summary>
    public void OnSysProxyEvent(JsonElement data)
    {
        try
        {
            if (data.TryGetProperty("enabled", out var en) &&
                (en.ValueKind == JsonValueKind.True || en.ValueKind == JsonValueKind.False))
            {
                _sysProxyOn = en.GetBoolean();
            }
            if (data.TryGetProperty("managedByUs", out var mb) &&
                (mb.ValueKind == JsonValueKind.True || mb.ValueKind == JsonValueKind.False))
            {
                _sysProxyManaged = mb.GetBoolean();
            }
            if (_ready) Render();
        }
        catch (Exception e)
        {
            Log.Warn($"首页处理系统代理事件失败：{e.Message}");
        }
    }

    /// <summary>settings:changed 事件。</summary>
    public void OnSettingsEvent(JsonElement data)
    {
        try
        {
            if (data.TryGetProperty("proxyMode", out var pm) && pm.ValueKind == JsonValueKind.String)
                _proxyMode = pm.GetString() ?? _proxyMode;
            if (data.TryGetProperty("mihomoMode", out var mm) && mm.ValueKind == JsonValueKind.String)
                _mihomoMode = mm.GetString() ?? _mihomoMode;
            if (_ready) Render();
        }
        catch (Exception e)
        {
            Log.Warn($"首页处理设置事件失败：{e.Message}");
        }
    }

    /// <summary>traffic:updated 事件（后端若新增则自动生效）。</summary>
    public void OnTrafficEvent(JsonElement data)
    {
        try
        {
            if (data.TryGetProperty("upRate", out var ur) && ur.TryGetInt64(out var u)) _upRate = u;
            if (data.TryGetProperty("downRate", out var dr) && dr.TryGetInt64(out var d)) _downRate = d;
            if (_ready) Render();
        }
        catch (Exception e)
        {
            Log.Warn($"首页处理流量事件失败：{e.Message}");
        }
    }

    // ========================================================================
    //  渲染
    // ========================================================================

    private void Render()
    {
        if (!_ready) return;
        if (_host.XamlRoot is null) return;

        _host.Children.Clear();

        _host.Children.Add(new TextBlock
        {
            Text = "首页",
            Style = (Style)Application.Current.Resources["PageTitleStyle"],
        });
        _host.Children.Add(new TextBlock
        {
            Text = BuildSubtitle(),
            Style = (Style)Application.Current.Resources["PageSubtitleStyle"],
            Margin = new Thickness(0, 4, 0, 6),
        });

        _host.Children.Add(BuildStatusCard());
        _host.Children.Add(BuildTrafficTripleCard());
        _host.Children.Add(BuildTrendCard());
    }

    private string BuildSubtitle()
    {
        if (_core.Running)
        {
            return _sysProxyOn
                ? "内核运行中，系统代理已接管"
                : "内核运行中，系统代理未开启";
        }
        return _core.InstalledOrRunning
            ? "内核已就绪但未运行，打开下方开关即可开始"
            : "还没安装内核，请到「设置」页安装";
    }

    // ---- 主状态卡片 ----

    private UIElement BuildStatusCard()
    {
        var card = new Border { Style = (Style)Application.Current.Resources["CardStyle"] };
        var sp = new StackPanel { Spacing = 0 };

        // 第一行：状态圆点 + 文案 + 实时速率
        var headRow = new Grid();
        headRow.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        headRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var left = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 9 };

        left.Children.Add(new Ellipse
        {
            Width = 10,
            Height = 10,
            VerticalAlignment = VerticalAlignment.Center,
            Fill = (Brush)Application.Current.Resources[CoreStateBrushKey()],
        });

        left.Children.Add(new TextBlock
        {
            Text = _core.DisplayText,
            FontSize = 15,
            FontWeight = FontWeights.SemiBold,
            VerticalAlignment = VerticalAlignment.Center,
            Foreground = (Brush)Application.Current.Resources["TextPrimaryBrush"],
        });

        headRow.Children.Add(left);

        // 实时速率
        var rate = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Spacing = 14,
            VerticalAlignment = VerticalAlignment.Center,
        };
        rate.Children.Add(RateText("\uE74A", _downRate, "GoodBrush"));
        rate.Children.Add(RateText("\uE74B", _upRate, "BrandBrush"));
        Grid.SetColumn(rate, 1);
        headRow.Children.Add(rate);

        sp.Children.Add(headRow);

        // 第二行：三个开关/选择
        var controls = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Spacing = 28,
            Margin = new Thickness(0, 18, 0, 0),
        };

        controls.Children.Add(BuildModeSelector());
        controls.Children.Add(BuildSysProxyToggle());
        controls.Children.Add(BuildCoreToggle());

        sp.Children.Add(controls);

        // 错误提示
        if (_core.State == "failed" && _core.Error is { Length: > 0 } err)
        {
            sp.Children.Add(new TextBlock
            {
                Text = $"内核错误：{err}",
                FontSize = 12,
                Margin = new Thickness(0, 12, 0, 0),
                TextWrapping = TextWrapping.Wrap,
                Foreground = (Brush)Application.Current.Resources["BadBrush"],
            });
        }

        card.Child = sp;
        return card;
    }

    private string CoreStateBrushKey() => _core.State switch
    {
        "running" => "GoodBrush",
        "starting" or "stopping" => "OkBrush",
        "failed" => "BadBrush",
        _ => "TextTertiaryBrush",
    };

    private static UIElement RateText(string glyph, long rate, string brushKey)
    {
        var sp = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 5 };
        sp.Children.Add(new FontIcon
        {
            Glyph = glyph,
            FontSize = 11,
            Foreground = (Brush)Application.Current.Resources[brushKey],
            VerticalAlignment = VerticalAlignment.Center,
        });
        sp.Children.Add(new TextBlock
        {
            Text = MainWindow.FormatRate(rate),
            FontSize = 12.5,
            VerticalAlignment = VerticalAlignment.Center,
            Foreground = (Brush)Application.Current.Resources["TextSecondaryBrush"],
        });
        return sp;
    }

    /// <summary>
    /// 代理方式：规则 / 全局 / 直连。
    ///
    /// 用独立的 <c>mode_set</c> 命令，**不是** settings_update。
    /// 后端把两件事分开了：
    ///   · proxyMode  —— 抓流方式（system_proxy / tun / manual）
    ///   · mihomoMode —— 分流模式（rule / global / direct）
    /// mode_set 会立即重载内核生效；内核没跑时也返回成功（存盘，下次启动生效）。
    /// </summary>
    private UIElement BuildModeSelector()
    {
        var sp = new StackPanel { Spacing = 6 };
        sp.Children.Add(Label("代理方式"));

        var combo = new ComboBox
        {
            MinWidth = 132,
            FontSize = 13,
            ItemsSource = new[] { "规则模式", "全局模式", "直连模式" },
            SelectedIndex = ModeToIndex(_mihomoMode),
        };

        // 上面的「代理方式」是独立 TextBlock，不是 ComboBox 的 Header，
        // 读屏软件读不出来这个下拉是干什么的。显式补一个可访问名称。
        AutomationProperties.SetName(combo, "代理方式");

        combo.SelectionChanged += async (_, _) =>
        {
            if (combo.SelectedIndex < 0) return;
            var mode = IndexToMode(combo.SelectedIndex);
            if (mode == _mihomoMode) return;
            await SetModeAsync(mode, combo);
        };

        sp.Children.Add(combo);
        return sp;
    }

    private static int ModeToIndex(string mode) => mode switch
    {
        "global" => 1,
        "direct" => 2,
        _ => 0,
    };

    private static string IndexToMode(int index) => index switch
    {
        1 => "global",
        2 => "direct",
        _ => "rule",
    };

    private async Task SetModeAsync(string mode, ComboBox combo)
    {
        try
        {
            await App.Daemon.CallAsync("mode_set", new { mode });

            // 以服务端回读的值为准，避免本地状态与内核不一致
            _mihomoMode = await ReadModeAsync() ?? mode;
        }
        catch (DaemonException e)
        {
            Log.Error($"切换代理方式失败：{e.FriendlyMessage}");
            combo.SelectedIndex = ModeToIndex(_mihomoMode);
        }
    }

    /// <summary>读当前分流模式；失败返回 null，调用方保留旧值。</summary>
    private static async Task<string?> ReadModeAsync()
    {
        try
        {
            var mode = await App.Daemon.CallAsync<string>("mode_get");
            return string.IsNullOrWhiteSpace(mode) ? null : mode;
        }
        catch (DaemonException e)
        {
            Log.Warn($"读取分流模式失败：{e.FriendlyMessage}");
            return null;
        }
    }

    /// <summary>系统代理开关。</summary>
    private UIElement BuildSysProxyToggle()
    {
        var sp = new StackPanel { Spacing = 6 };
        sp.Children.Add(Label("系统代理"));

        var toggle = new ToggleSwitch
        {
            IsOn = _sysProxyOn,
            OnContent = "已开启",
            OffContent = "已关闭",
            MinWidth = 0,
        };

        toggle.Toggled += async (_, _) =>
        {
            var want = toggle.IsOn;
            toggle.IsEnabled = false;
            try
            {
                if (want)
                {
                    await App.Daemon.CallAsync("sysproxy_enable");
                    _sysProxyOn = true;
                    _sysProxyManaged = true;
                }
                else
                {
                    await App.Daemon.CallAsync("sysproxy_disable");
                    _sysProxyOn = false;
                    _sysProxyManaged = false;
                }
            }
            catch (DaemonException e)
            {
                Log.Error($"切换系统代理失败：{e.FriendlyMessage}");
                toggle.IsOn = !want; // 回滚
            }
            finally
            {
                toggle.IsEnabled = true;
            }
        };

        sp.Children.Add(toggle);

        if (_sysProxyOn && !_sysProxyManaged)
        {
            sp.Children.Add(new TextBlock
            {
                Text = "当前代理非本应用设置",
                FontSize = 10.5,
                Foreground = (Brush)Application.Current.Resources["TextTertiaryBrush"],
            });
        }

        return sp;
    }

    /// <summary>内核开关。</summary>
    private UIElement BuildCoreToggle()
    {
        var sp = new StackPanel { Spacing = 6 };
        sp.Children.Add(Label("内核"));

        var busy = _core.State is "starting" or "stopping";
        var installed = _core.InstalledOrRunning;

        var toggle = new ToggleSwitch
        {
            IsOn = _core.Running,
            IsEnabled = !busy && installed,
            OnContent = _core.Starting ? "启动中…" : "运行中",
            OffContent = installed ? "已停止" : "未安装",
            MinWidth = 0,
        };

        toggle.Toggled += async (_, _) =>
        {
            var want = toggle.IsOn;
            toggle.IsEnabled = false;
            try
            {
                var st = await App.Daemon.CallAsync<CoreStatus>(
                    want ? "core_start" : "core_stop", timeoutSec: 60);
                if (st is not null) _core = st;
                Render();
            }
            catch (DaemonException e)
            {
                Log.Error($"将内核切换为{(want ? "启动" : "停止")}失败：{e.FriendlyMessage}");
                toggle.IsOn = !want;
                toggle.IsEnabled = true;
            }
        };

        sp.Children.Add(toggle);
        return sp;
    }

    private static TextBlock Label(string text) => new()
    {
        Text = text,
        FontSize = 11,
        Foreground = (Brush)Application.Current.Resources["TextTertiaryBrush"],
    };

    // ---- 流量三栏 ----

    private UIElement BuildTrafficTripleCard()
    {
        var card = new Border { Style = (Style)Application.Current.Resources["CardStyle"] };
        var sp = new StackPanel { Spacing = 12 };

        var head = new Grid();
        head.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        head.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        head.Children.Add(new TextBlock
        {
            Text = "流量",
            Style = (Style)Application.Current.Resources["SectionTitleStyle"],
            Margin = new Thickness(0),
        });

        if (!_statsAvailable)
        {
            var tip = new TextBlock
            {
                Text = "功能开发中",
                Style = (Style)Application.Current.Resources["HintTextStyle"],
                VerticalAlignment = VerticalAlignment.Center,
            };
            Grid.SetColumn(tip, 1);
            head.Children.Add(tip);
        }

        sp.Children.Add(head);

        // 三栏
        var cols = new Grid();
        for (var i = 0; i < 3; i++)
        {
            cols.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        }

        var today = _statsAvailable ? _stats.Today : 0;
        var session = _statsAvailable ? _stats.Session : 0;
        var total = _statsAvailable ? _stats.Total : 0;

        AddTriple(cols, 0, "今日", today, _stats.TodayUp, _stats.TodayDown, "BrandBrush");
        AddTriple(cols, 1, "本次运行", session, _stats.SessionUp ?? 0, _stats.SessionDown ?? 0, "GoodBrush");
        AddTriple(cols, 2, "累计", total, _stats.TotalUp, _stats.TotalDown, "TextPrimaryBrush");

        sp.Children.Add(cols);
        card.Child = sp;
        return card;
    }

    private void AddTriple(Grid grid, int col, string title, long total,
                           long up, long down, string brushKey)
    {
        var sp = new StackPanel { Spacing = 4 };

        sp.Children.Add(new TextBlock
        {
            Text = title,
            FontSize = 11.5,
            Foreground = (Brush)Application.Current.Resources["TextTertiaryBrush"],
        });

        sp.Children.Add(new TextBlock
        {
            Text = _statsAvailable ? MainWindow.FormatBytes(total) : "—",
            FontSize = 21,
            FontWeight = FontWeights.SemiBold,
            Foreground = (Brush)Application.Current.Resources[brushKey],
        });

        sp.Children.Add(new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Spacing = 10,
            Children =
            {
                ArrowStat("\uE74A", _statsAvailable ? MainWindow.FormatBytes(up) : "—", "GoodBrush"),
                ArrowStat("\uE74B", _statsAvailable ? MainWindow.FormatBytes(down) : "—", "BrandBrush"),
            },
        });

        Grid.SetColumn(sp, col);
        grid.Children.Add(sp);
    }

    private static UIElement ArrowStat(string glyph, string text, string brushKey)
    {
        var sp = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 3 };
        sp.Children.Add(new FontIcon
        {
            Glyph = glyph,
            FontSize = 9,
            Foreground = (Brush)Application.Current.Resources[brushKey],
            VerticalAlignment = VerticalAlignment.Center,
        });
        sp.Children.Add(new TextBlock
        {
            Text = text,
            FontSize = 11,
            VerticalAlignment = VerticalAlignment.Center,
            Foreground = (Brush)Application.Current.Resources["TextSecondaryBrush"],
        });
        return sp;
    }

    // ---- 24 小时趋势图 ----

    private UIElement BuildTrendCard()
    {
        var card = new Border { Style = (Style)Application.Current.Resources["CardStyle"] };
        var sp = new StackPanel { Spacing = 10 };

        var headRow = new Grid();
        headRow.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        headRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        headRow.Children.Add(new TextBlock
        {
            Text = "24 小时趋势",
            Style = (Style)Application.Current.Resources["SectionTitleStyle"],
            Margin = new Thickness(0),
        });

        var hourly = _stats.Hourly ?? new List<HourlyTraffic>();
        var peak = hourly.Count > 0 ? hourly.Max(h => h.Total) : 0;

        if (_statsAvailable && peak > 0)
        {
            var peakText = new TextBlock
            {
                Text = $"峰值 {MainWindow.FormatRate(peak)}",
                Style = (Style)Application.Current.Resources["HintTextStyle"],
                VerticalAlignment = VerticalAlignment.Center,
            };
            Grid.SetColumn(peakText, 1);
            headRow.Children.Add(peakText);
        }

        sp.Children.Add(headRow);

        if (!_statsAvailable)
        {
            sp.Children.Add(BuildPlaceholder("内核服务尚未提供流量统计，趋势图暂不可用"));
        }
        else if (hourly.Count == 0 || peak == 0)
        {
            sp.Children.Add(BuildPlaceholder("还没有流量数据，运行一段时间后这里会出现趋势曲线"));
        }
        else
        {
            sp.Children.Add(BuildChart(hourly, peak));
        }

        card.Child = sp;
        return card;
    }

    private static UIElement BuildPlaceholder(string text)
    {
        var box = new Border
        {
            Height = 132,
            CornerRadius = new CornerRadius(6),
            Background = (Brush)Application.Current.Resources["NavBgBrush"],
            Child = new TextBlock
            {
                Text = text,
                FontSize = 12,
                HorizontalAlignment = HorizontalAlignment.Center,
                VerticalAlignment = VerticalAlignment.Center,
                TextWrapping = TextWrapping.Wrap,
                TextAlignment = TextAlignment.Center,
                Foreground = (Brush)Application.Current.Resources["TextTertiaryBrush"],
            },
        };
        return box;
    }

    /// <summary>
    /// 原生手绘折线图：Canvas + Polyline。
    ///
    /// 不引第三方图表库——只有一条曲线和一层渐变填充，
    /// 用 Shape 直接画既轻量又能跟随主题色。
    /// </summary>
    private UIElement BuildChart(List<HourlyTraffic> hourly, long peak)
    {
        const double chartH = 132;
        const double padLeft = 4;
        const double padRight = 4;
        const double padTop = 10;
        const double padBottom = 18;

        var container = new Grid();

        // 用固定高度 + 自适应宽度：外层包一个 Canvas，尺寸变化时重画
        var canvas = new Canvas
        {
            Height = chartH,
            Background = new SolidColorBrush(Colors.Transparent),
        };

        var gridLines = new Canvas { Height = chartH };
        container.Children.Add(gridLines);
        container.Children.Add(canvas);

        // 底部小时标签
        var labels = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            HorizontalAlignment = HorizontalAlignment.Stretch,
            Margin = new Thickness(0, 2, 0, 0),
        };

        var brand = (Brush)Application.Current.Resources["BrandBrush"];
        var brandColor = ((SolidColorBrush)brand).Color;
        var divider = (Brush)Application.Current.Resources["DividerBrush"];

        void Redraw(double w)
        {
            if (w <= 1) w = 560;

            canvas.Children.Clear();
            gridLines.Children.Clear();

            var plotW = w - padLeft - padRight;
            var plotH = chartH - padTop - padBottom;

            // 三条水平参考线
            for (var i = 0; i <= 2; i++)
            {
                var y = padTop + plotH * i / 2.0;
                gridLines.Children.Add(new Line
                {
                    X1 = padLeft,
                    Y1 = y,
                    X2 = w - padRight,
                    Y2 = y,
                    Stroke = divider,
                    StrokeThickness = 0.8,
                    Opacity = i == 2 ? 0.9 : 0.45,
                });
            }

            var n = hourly.Count;
            if (n == 0) return;

            double X(int i) => padLeft + (n == 1 ? plotW / 2 : plotW * i / (n - 1.0));
            double Y(long v) => padTop + plotH * (1 - (double)v / peak);

            // 面积填充
            var area = new Polygon
            {
                Fill = new LinearGradientBrush
                {
                    StartPoint = new Point(0, 0),
                    EndPoint = new Point(0, 1),
                    GradientStops =
                    {
                        new GradientStop { Color = Color.FromArgb(90, brandColor.R, brandColor.G, brandColor.B), Offset = 0 },
                        new GradientStop { Color = Color.FromArgb(0, brandColor.R, brandColor.G, brandColor.B), Offset = 1 },
                    },
                },
            };
            var areaPts = new PointCollection();
            for (var i = 0; i < n; i++) areaPts.Add(new Point(X(i), Y(hourly[i].Total)));
            areaPts.Add(new Point(X(n - 1), padTop + plotH));
            areaPts.Add(new Point(X(0), padTop + plotH));
            area.Points = areaPts;
            canvas.Children.Add(area);

            // 折线
            var line = new Polyline
            {
                Stroke = brand,
                StrokeThickness = 2,
                StrokeLineJoin = PenLineJoin.Round,
            };
            var pts = new PointCollection();
            for (var i = 0; i < n; i++) pts.Add(new Point(X(i), Y(hourly[i].Total)));
            line.Points = pts;
            canvas.Children.Add(line);

            // 当前点高亮
            var lastEllipse = new Ellipse
            {
                Width = 7,
                Height = 7,
                Fill = brand,
            };
            Canvas.SetLeft(lastEllipse, X(n - 1) - 3.5);
            Canvas.SetTop(lastEllipse, Y(hourly[n - 1].Total) - 3.5);
            canvas.Children.Add(lastEllipse);
        }

        // 尺寸变化时重画（首次布局也走这里）
        container.SizeChanged += (_, e) => Redraw(e.NewSize.Width);
        Redraw(560);

        // 底部标签：起点 / 中点 / 终点
        var times = new List<string>();
        if (hourly.Count > 0)
        {
            times.Add(hourly[0].HourLabel);
            if (hourly.Count > 2) times.Add(hourly[hourly.Count / 2].HourLabel);
            times.Add(hourly[^1].HourLabel);
        }

        var labelGrid = new Grid { Margin = new Thickness(padLeft, 2, padRight, 0) };
        for (var i = 0; i < times.Count; i++)
        {
            labelGrid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        }

        for (var i = 0; i < times.Count; i++)
        {
            var tb = new TextBlock
            {
                Text = times[i],
                FontSize = 10,
                Foreground = (Brush)Application.Current.Resources["TextTertiaryBrush"],
                HorizontalAlignment = i == 0
                    ? HorizontalAlignment.Left
                    : i == times.Count - 1 && times.Count > 1
                        ? HorizontalAlignment.Right
                        : HorizontalAlignment.Center,
            };
            Grid.SetColumn(tb, i);
            labelGrid.Children.Add(tb);
        }

        var root = new StackPanel { Spacing = 0 };
        root.Children.Add(container);
        if (times.Count > 0) root.Children.Add(labelGrid);
        return root;
    }
}
