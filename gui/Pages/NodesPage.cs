using System.Text.Json;
using ClashZhengl.Core;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;

namespace ClashZhengl.Pages;

/// <summary>
/// 节点页：分组列表 + 测速 + 手动切换。
/// 注意组类型判断使用与 Rust 侧相同的严格比较（Selector 大写 S），
/// 不可用忽略大小写的方式，否则 URLTest/Fallback 组会被误判成可手动选择。
/// </summary>
public sealed class NodesPage
{
    private const string KindSelector = "Selector";

    private readonly StackPanel _host;
    private List<NodeGroup> _groups = new();
    private SpeedtestStatus _speedtest = new();
    private bool _coreRunning;
    private bool _busy;

    private TextBlock? _speedStatusText;
    private ProgressBar? _speedBar;
    private Button? _speedBtn;

    public NodesPage(StackPanel host) => _host = host;

    public Task InitializeAsync() => RefreshAsync();

    public async Task RefreshAsync()
    {
        try
        {
            _groups = await App.Daemon.CallAsync<List<NodeGroup>>("node_groups")
                      ?? new List<NodeGroup>();

            // GroupDto 不带组类型，逐个补拉；失败不影响整页
            foreach (var g in _groups)
            {
                try
                {
                    g.Kind = await App.Daemon
                        .CallAsync<string>("node_group_raw_kind", new { group = g.Name })
                        ?? "";
                }
                catch (DaemonException)
                {
                    g.Kind = "";
                }
            }

            try
            {
                _speedtest = await App.Daemon.CallAsync<SpeedtestStatus>("speedtest_status")
                             ?? new SpeedtestStatus();
            }
            catch (DaemonException)
            {
                // 内核没起来时查不到测速状态，不算错误
            }

            Render();
        }
        catch (DaemonException e) when (e.Code == "E_CORE_NOT_RUNNING")
        {
            // 内核没起来不是错误：启动时序上必然先连上 daemon、内核稍后才好。
            // 降级成普通提示，等 core:state 事件或下一次刷新自然恢复。
            _groups.Clear();
            _coreRunning = false;
            Render();
        }
        catch (DaemonException e)
        {
            Log.Error($"加载节点失败 code={e.Code}", e);
            _host.Children.Clear();
            _host.Children.Add(BuildHeader());
            _host.Children.Add(BuildNotice(e.FriendlyMessage, isError: true));
        }
    }

    private void Render()
    {
        _host.Children.Clear();
        _host.Children.Add(BuildHeader());

        if (!_coreRunning)
        {
            _host.Children.Add(BuildNotice("内核未运行，启动后才能看到节点列表。", isError: false));
            return;
        }

        if (_groups.Count == 0)
        {
            _host.Children.Add(BuildNotice("还没有可用的代理组，请先添加并更新订阅。", isError: false));
            return;
        }

        foreach (var g in _groups)
        {
            _host.Children.Add(BuildGroupCard(g));
        }
    }

    private UIElement BuildHeader()
    {
        var grid = new Grid();
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var left = new StackPanel();
        left.Children.Add(new TextBlock
        {
            Text = "节点",
            Style = (Style)Application.Current.Resources["PageTitleStyle"],
        });
        left.Children.Add(new TextBlock
        {
            Text = _groups.Count > 0
                ? $"{_groups.Count} 个代理组，共 {_groups.Sum(g => g.NodeCount)} 个节点"
                : "测速后可按延迟排序选择最快节点",
            Style = (Style)Application.Current.Resources["PageSubtitleStyle"],
        });
        grid.Children.Add(left);

        // 测速区
        var right = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Spacing = 10,
            VerticalAlignment = VerticalAlignment.Top,
        };

        if (_speedtest.Running)
        {
            var prog = new StackPanel { Spacing = 4, Width = 150 };
            _speedStatusText = new TextBlock
            {
                Text = $"测速中 {_speedtest.Finished}/{_speedtest.Total}",
                FontSize = 11.5,
                Foreground = (Brush)Application.Current.Resources["TextSecondaryBrush"],
            };
            prog.Children.Add(_speedStatusText);

            _speedBar = new ProgressBar
            {
                Value = _speedtest.Total > 0
                    ? _speedtest.Finished * 100.0 / _speedtest.Total
                    : 0,
                Height = 4,
            };
            prog.Children.Add(_speedBar);
            right.Children.Add(prog);
        }

        _speedBtn = new Button
        {
            Content = _speedtest.Running ? "测速中…" : "开始测速",
            Style = (Style)Application.Current.Resources["PrimaryButtonStyle"],
            IsEnabled = !_speedtest.Running && _coreRunning,
        };
        _speedBtn.Click += async (_, _) => await RunSpeedtestAsync();
        right.Children.Add(_speedBtn);

        Grid.SetColumn(right, 1);
        grid.Children.Add(right);

        return grid;
    }

    private UIElement BuildNotice(string msg, bool isError)
    {
        return new Border
        {
            Style = (Style)Application.Current.Resources["CardStyle"],
            Child = new StackPanel
            {
                Orientation = Orientation.Horizontal,
                Spacing = 10,
                Children =
                {
                    new FontIcon
                    {
                        Glyph = isError ? "\uE783" : "\uE946",
                        FontSize = 15,
                        Foreground = (Brush)Application.Current.Resources[
                            isError ? "BadBrush" : "TextTertiaryBrush"],
                        VerticalAlignment = VerticalAlignment.Center,
                    },
                    new TextBlock
                    {
                        Text = msg,
                        FontSize = 13,
                        Foreground = (Brush)Application.Current.Resources["TextSecondaryBrush"],
                        VerticalAlignment = VerticalAlignment.Center,
                        TextWrapping = TextWrapping.Wrap,
                    },
                },
            },
        };
    }

    private UIElement BuildGroupCard(NodeGroup g)
    {
        var card = new Border { Style = (Style)Application.Current.Resources["CardStyle"] };
        var sp = new StackPanel { Spacing = 0 };

        // ---- 组头 ----
        var head = new Grid { Margin = new Thickness(0, 0, 0, 10) };
        head.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        head.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var titleRow = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        titleRow.Children.Add(new TextBlock
        {
            Text = g.Name,
            Style = (Style)Application.Current.Resources["SectionTitleStyle"],
            Margin = new Thickness(0),
        });

        // 组类型徽章（GroupDto 不带 kind，是单独拉的；拿不到就不显示）
        if (!string.IsNullOrEmpty(g.Kind))
        {
            titleRow.Children.Add(new Border
            {
                Background = (Brush)Application.Current.Resources["NavSelectedBrush"],
                CornerRadius = new CornerRadius(4),
                Padding = new Thickness(6, 1, 6, 1),
                VerticalAlignment = VerticalAlignment.Center,
                Child = new TextBlock
                {
                    Text = g.Kind,
                    FontSize = 10,
                    Foreground = (Brush)Application.Current.Resources["TextTertiaryBrush"],
                },
            });
        }
        head.Children.Add(titleRow);

        if (!string.IsNullOrWhiteSpace(g.Now))
        {
            var nowSp = new StackPanel
            {
                Orientation = Orientation.Horizontal,
                Spacing = 6,
                VerticalAlignment = VerticalAlignment.Center,
            };
            nowSp.Children.Add(new TextBlock
            {
                Text = "当前：",
                FontSize = 11.5,
                Foreground = (Brush)Application.Current.Resources["TextTertiaryBrush"],
            });
            nowSp.Children.Add(new TextBlock
            {
                Text = g.Now,
                FontSize = 12,
                Foreground = (Brush)Application.Current.Resources["BrandBrush"],
            });
            Grid.SetColumn(nowSp, 1);
            head.Children.Add(nowSp);
        }

        sp.Children.Add(head);

        // ---- 节点列表 ----
        // 只对 Selector 组开放手动切换；URLTest/Fallback 由内核自动决定
        var canSelect = g.CanSelect;

        var listPanel = new StackPanel { Spacing = 2 };
        var sorted = g.Nodes
            .OrderBy(n => n.Rank ?? int.MaxValue)
            .ThenBy(n => n.DelayMs ?? int.MaxValue)
            .Take(60);

        foreach (var node in sorted)
        {
            listPanel.Children.Add(BuildNodeRow(g, node, canSelect));
        }

        if (g.Nodes.Count > 60)
        {
            listPanel.Children.Add(new TextBlock
            {
                Text = $"另有 {g.Nodes.Count - 60} 个节点未显示（可搜索或测速后查看）",
                FontSize = 11,
                Margin = new Thickness(10, 6, 0, 0),
                Foreground = (Brush)Application.Current.Resources["TextTertiaryBrush"],
            });
        }

        sp.Children.Add(listPanel);
        card.Child = sp;
        return card;
    }

    private UIElement BuildNodeRow(NodeGroup g, NodeItem n, bool canSelect)
    {
        var row = new Grid
        {
            Padding = new Thickness(10, 7, 10, 7),
            CornerRadius = new CornerRadius(6),
            Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent),
        };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        // ---- 左侧：选中标记 + 名称 ----
        var left = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };

        left.Children.Add(new Border
        {
            Width = 3,
            Height = 16,
            CornerRadius = new CornerRadius(2),
            VerticalAlignment = VerticalAlignment.Center,
            // "Transparent" 不是主题资源键，Application.Current.Resources["Transparent"]
            // 会抛 KeyNotFound（.NET 里报成「未指定的错误」），整页渲染中断。
            // 未选中的标记条必须用真正透明的画刷。
            Background = n.Selected
                ? (Brush)Application.Current.Resources["BrandBrush"]
                : new SolidColorBrush(Microsoft.UI.Colors.Transparent),
        });

        left.Children.Add(new TextBlock
        {
            Text = n.Name,
            FontSize = 13,
            VerticalAlignment = VerticalAlignment.Center,
            Foreground = (Brush)Application.Current.Resources[
                n.Selected ? "BrandBrush" : "TextPrimaryBrush"],
            TextTrimming = TextTrimming.CharacterEllipsis,
            MaxWidth = 420,
        });

        row.Children.Add(left);

        // ---- 右侧：延迟 ----
        var right = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };

        if (n.Error is { Length: > 0 })
        {
            right.Children.Add(new TextBlock
            {
                Text = "失败",
                FontSize = 11.5,
                Foreground = (Brush)Application.Current.Resources["BadBrush"],
                VerticalAlignment = VerticalAlignment.Center,
            });
        }
        else
        {
            right.Children.Add(new TextBlock
            {
                Text = MainWindow.FormatDelay(n.DelayMs),
                FontSize = 11.5,
                Foreground = MainWindow.DelayBrush(n.DelayMs),
                VerticalAlignment = VerticalAlignment.Center,
                MinWidth = 56,
                TextAlignment = TextAlignment.Right,
            });
        }

        if (n.Ok && n.DelayMs is int d)
        {
            right.Children.Add(BuildDelayBar(d));
        }

        Grid.SetColumn(right, 1);
        row.Children.Add(right);

        // ---- 交互 ----
        if (canSelect && !n.Selected && !_busy)
        {
            row.Tapped += async (_, _) => await SelectNodeAsync(g, n);
            row.PointerEntered += (_, _) =>
                row.Background = (Brush)Application.Current.Resources["CardBgHoverBrush"];
            row.PointerExited += (_, _) =>
                row.Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        }

        return row;
    }

    /// <summary>延迟小柱状条：4 格，按延迟档位点亮。</summary>
    private static UIElement BuildDelayBar(int ms)
    {
        var level = ms switch
        {
            < 200 => 4,
            < 500 => 3,
            < 1000 => 2,
            _ => 1,
        };

        var sp = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Spacing = 2,
            VerticalAlignment = VerticalAlignment.Center,
        };

        for (var i = 0; i < 4; i++)
        {
            sp.Children.Add(new Border
            {
                Width = 3,
                Height = 6 + i * 3,
                CornerRadius = new CornerRadius(1.5),
                VerticalAlignment = VerticalAlignment.Bottom,
                Background = (Brush)Application.Current.Resources[
                    i < level ? MainWindow.DelayKey(ms) : "DividerBrush"],
            });
        }

        return sp;
    }

    private async Task SelectNodeAsync(NodeGroup g, NodeItem n)
    {
        if (_busy) return;
        _busy = true;

        try
        {
            await App.Daemon.CallAsync("switch_manual",
                new { group = g.Name, node = n.Name }, timeoutSec: 20);
            await RefreshAsync();
        }
        catch (DaemonException e)
        {
            Log.Error($"切换节点失败 code={e.Code}", e);
        }
        finally
        {
            _busy = false;
        }
    }

    private async Task RunSpeedtestAsync()
    {
        if (_speedBtn is null) return;
        _speedBtn.IsEnabled = false;
        _speedBtn.Content = "启动中…";

        try
        {
            await App.Daemon.CallAsync("speedtest_run", timeoutSec: 20);
            // 后续进度靠 speedtest:progress 事件推送，这里只做兜底轮询
            _ = PollSpeedtestAsync();
        }
        catch (DaemonException e)
        {
            Log.Error($"启动测速失败：{e.FriendlyMessage}");
            _speedBtn.Content = "开始测速";
            _speedBtn.IsEnabled = true;
        }
    }

    private async Task PollSpeedtestAsync()
    {
        for (var i = 0; i < 120; i++) // 最多等 4 分钟
        {
            await Task.Delay(2000);

            try
            {
                _speedtest = await App.Daemon.CallAsync<SpeedtestStatus>("speedtest_status")
                             ?? new SpeedtestStatus();

                if (!_speedtest.Running)
                {
                    await RefreshAsync();
                    return;
                }

                if (_speedStatusText is not null)
                {
                    _speedStatusText.Text = $"测速中 {_speedtest.Finished}/{_speedtest.Total}";
                }
                if (_speedBar is not null && _speedtest.Total > 0)
                {
                    _speedBar.Value = _speedtest.Finished * 100.0 / _speedtest.Total;
                }
            }
            catch (DaemonException)
            {
                // 内核重启中，继续等
            }
        }
    }

    /// <summary>主窗口状态轮询时同步内核运行态。</summary>
    public void ApplyRuntime(RuntimeState rt)
    {
        var wasRunning = _coreRunning;
        _coreRunning = rt.CoreRunning;

        // 只在状态变化时重绘，避免每 2 秒重建整个页面
        if (wasRunning != _coreRunning)
        {
            _ = RefreshAsync();
        }
    }

    public void HandleEvent(string name, JsonElement data)
    {
        if (name == "speedtest:progress")
        {
            try
            {
                var st = data.Deserialize<SpeedtestStatus>();
                if (st is null) return;
                _speedtest = st;

                if (_speedStatusText is not null) 
                    _speedStatusText.Text = $"测速中 {st.Finished}/{st.Total}";
                if (_speedBar is not null && st.Total > 0)
                    _speedBar.Value = st.Finished * 100.0 / st.Total;

                if (!st.Running) _ = RefreshAsync();
            }
            catch (Exception e)
            {
                Log.Warn($"处理测速进度失败：{e.Message}");
            }
        }
    }

    public void OnSwitchApplied(JsonElement data)
    {
        _ = RefreshAsync();
    }
}
