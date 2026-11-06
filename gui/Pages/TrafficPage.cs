using System.Text.Json;
using ClashZhengl.Core;
using Microsoft.UI;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;

namespace ClashZhengl.Pages;

/// <summary>
/// 流量页：总量进度环 + 各订阅明细 + 到期提醒。
///
/// 数据来源是 daemon 的 traffic summary，其中 total / usedPercent 可空 ——
/// 订阅没提供 userinfo 时显示「未提供」，不能当成 0 处理。
/// </summary>
public sealed class TrafficPage
{
    private readonly StackPanel _host;
    private TrafficSummary _traffic = new();

    public TrafficPage(StackPanel host) => _host = host;

    public Task InitializeAsync() => RefreshAsync();

    public async Task RefreshAsync()
    {
        try
        {
            _traffic = await App.Daemon.CallAsync<TrafficSummary>("subscription_traffic")
                       ?? new TrafficSummary();
            Render();
        }
        catch (DaemonException e)
        {
            Log.Error($"加载流量失败 code={e.Code}", e);
            _host.Children.Clear();
            _host.Children.Add(new TextBlock
            {
                Text = e.FriendlyMessage,
                Style = (Style)Application.Current.Resources["PageTitleStyle"],
            });
            _host.Children.Add(new TextBlock
            {
                Text = e.Message,
                Foreground = (Brush)Application.Current.Resources["TextSecondaryBrush"],
                TextWrapping = TextWrapping.Wrap,
            });
        }
    }

    private void Render()
    {
        _host.Children.Clear();

        _host.Children.Add(new TextBlock
        {
            Text = "流量",
            Style = (Style)Application.Current.Resources["PageTitleStyle"],
        });

        var t = _traffic;
        var subtitle = t.HasTotal
            ? $"已使用 {MainWindow.FormatBytes(t.Used)}，剩余 {MainWindow.FormatBytes(Math.Max(0, t.Total!.Value - t.Used))}"
            : t.SubscriptionCount == 0
                ? "还没有订阅，添加后这里会显示流量"
                : "当前订阅未提供流量信息";

        _host.Children.Add(new TextBlock
        {
            Text = subtitle,
            Style = (Style)Application.Current.Resources["PageSubtitleStyle"],
            Margin = new Thickness(0, 4, 0, 6),
        });

        _host.Children.Add(BuildOverviewCard());

        if (t.DaysLeft is long days && days <= 7)
        {
            _host.Children.Add(BuildExpireWarning((int)days));
        }

        if (t.HasMissing)
        {
            _host.Children.Add(BuildNotice("部分订阅没有返回流量信息，统计可能不完整。"));
        }

        if (t.Items.Count > 0)
        {
            _host.Children.Add(BuildItemsCard());
        }
    }

    private UIElement BuildOverviewCard()
    {
        var card = new Border { Style = (Style)Application.Current.Resources["CardStyle"] };
        var grid = new Grid();
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });

        var t = _traffic;
        var pct = t.UsedPercent is float up ? Math.Clamp(up, 0, 100) : 0;

        // ---- 左侧进度环 ----
        var ringGrid = new Grid { Width = 128, Height = 128, Margin = new Thickness(4, 4, 22, 4) };

        ringGrid.Children.Add(new Ellipse
        {
            Stroke = (Brush)Application.Current.Resources["DividerBrush"],
            StrokeThickness = 10,
        });

        if (t.HasTotal && pct > 0)
        {
            var ring = new Ellipse
            {
                Stroke = (Brush)Application.Current.Resources[UsageBrushKey(pct)],
                StrokeThickness = 10,
                StrokeDashCap = PenLineCap.Round,
                StrokeStartLineCap = PenLineCap.Round,
                StrokeEndLineCap = PenLineCap.Round,
                RenderTransformOrigin = new Windows.Foundation.Point(0.5, 0.5),
                RenderTransform = new RotateTransform { Angle = -90 },
            };
            var circumference = Math.PI * 108; // 直径约 108
            ring.StrokeDashArray = new DoubleCollection
            {
                pct / 100 * circumference / 10,
                1000,
            };
            ringGrid.Children.Add(ring);
        }

        var center = new StackPanel
        {
            HorizontalAlignment = HorizontalAlignment.Center,
            VerticalAlignment = VerticalAlignment.Center,
            Spacing = 0,
        };

        if (t.HasTotal)
        {
            center.Children.Add(new TextBlock
            {
                Text = $"{pct:0}",
                FontSize = 26,
                FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
                Foreground = (Brush)Application.Current.Resources["TextPrimaryBrush"],
                HorizontalAlignment = HorizontalAlignment.Center,
            });
            center.Children.Add(new TextBlock
            {
                Text = "%",
                FontSize = 11,
                Foreground = (Brush)Application.Current.Resources["TextTertiaryBrush"],
                HorizontalAlignment = HorizontalAlignment.Center,
            });
        }
        else
        {
            center.Children.Add(new FontIcon
            {
                Glyph = "\uE9D9",
                FontSize = 24,
                Foreground = (Brush)Application.Current.Resources["TextTertiaryBrush"],
                HorizontalAlignment = HorizontalAlignment.Center,
            });
        }

        ringGrid.Children.Add(center);
        grid.Children.Add(ringGrid);

        // ---- 右侧明细 ----
        var info = new StackPanel
        {
            VerticalAlignment = VerticalAlignment.Center,
            Spacing = 14,
        };

        if (t.HasTotal)
        {
            info.Children.Add(BuildUsageBar(pct));
        }

        var rows = new Grid();
        rows.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        rows.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        rows.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        rows.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });

        AddInfo(rows, 0, 0, "已用", MainWindow.FormatBytes(t.Used));
        AddInfo(rows, 0, 1, "总量", t.HasTotal ? MainWindow.FormatBytes(t.Total!.Value) : "未提供");
        AddInfo(rows, 1, 0, "上传", MainWindow.FormatBytes(t.Upload));
        AddInfo(rows, 1, 1, "下载", MainWindow.FormatBytes(t.Download));

        info.Children.Add(rows);

        if (t.ExpireAt is long exp)
        {
            var dt = DateTimeOffset.FromUnixTimeSeconds(exp).ToLocalTime();
            var left = t.DaysLeft ?? 0;
            info.Children.Add(new TextBlock
            {
                Text = $"到期时间：{dt:yyyy-MM-dd}（剩 {left} 天）",
                FontSize = 12.5,
                Foreground = (Brush)Application.Current.Resources[
                    left <= 3 ? "BadBrush" : left <= 7 ? "SlowBrush" : "TextSecondaryBrush"],
            });
        }

        Grid.SetColumn(info, 1);
        grid.Children.Add(info);

        card.Child = grid;
        return card;
    }

    private static string UsageBrushKey(double pct) => pct switch
    {
        >= 95 => "BadBrush",
        >= 80 => "SlowBrush",
        _ => "BrandBrush",
    };

    private static UIElement BuildUsageBar(double pct)
    {
        var outer = new Grid { Height = 8 };

        outer.Children.Add(new Border
        {
            Background = (Brush)Application.Current.Resources["DividerBrush"],
            CornerRadius = new CornerRadius(4),
        });

        var fill = new Border
        {
            Background = (Brush)Application.Current.Resources[UsageBrushKey(pct)],
            CornerRadius = new CornerRadius(4),
            HorizontalAlignment = HorizontalAlignment.Left,
        };
        fill.SizeChanged += (_, e) =>
        {
            // 按父容器实际宽度百分比设置，避免硬编码
            if (outer.ActualWidth > 0)
                fill.Width = Math.Max(4, outer.ActualWidth * pct / 100);
        };
        outer.Children.Add(fill);

        return outer;
    }

    private static void AddInfo(Grid grid, int row, int col, string label, string value)
    {
        var sp = new StackPanel { Spacing = 2, Margin = new Thickness(0, 0, 0, 8) };

        sp.Children.Add(new TextBlock
        {
            Text = label,
            FontSize = 11.5,
            Foreground = (Brush)Application.Current.Resources["TextTertiaryBrush"],
        });

        sp.Children.Add(new TextBlock
        {
            Text = value,
            FontSize = 14,
            Foreground = (Brush)Application.Current.Resources["TextPrimaryBrush"],
        });

        Grid.SetRow(sp, row);
        Grid.SetColumn(sp, col);
        grid.Children.Add(sp);
    }

    private UIElement BuildItemsCard()
    {
        var card = new Border { Style = (Style)Application.Current.Resources["CardStyle"] };
        var sp = new StackPanel { Spacing = 12 };

        sp.Children.Add(new TextBlock
        {
            Text = "订阅明细",
            Style = (Style)Application.Current.Resources["SectionTitleStyle"],
        });

        foreach (var it in _traffic.Items)
        {
            sp.Children.Add(BuildItemRow(it));
        }

        card.Child = sp;
        return card;
    }

    private UIElement BuildItemRow(ItemTraffic it)
    {
        var grid = new Grid();
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var left = new StackPanel { Spacing = 3 };
        left.Children.Add(new TextBlock
        {
            Text = it.Name,
            FontSize = 13,
            Foreground = (Brush)Application.Current.Resources["TextPrimaryBrush"],
        });

        var detail = it.Total is > 0
            ? $"{MainWindow.FormatBytes(it.Used ?? 0)} / {MainWindow.FormatBytes(it.Total.Value)}"
            : it.Used is > 0
                ? $"已用 {MainWindow.FormatBytes(it.Used.Value)}（总量未知）"
                : "未提供流量信息";

        left.Children.Add(new TextBlock
        {
            Text = detail,
            FontSize = 11.5,
            Foreground = (Brush)Application.Current.Resources["TextTertiaryBrush"],
        });

        grid.Children.Add(left);

        if (it.DaysLeft is int d)
        {
            var badge = new Border
            {
                Background = (Brush)Application.Current.Resources[
                    d <= 3 ? "BadBrush" : d <= 7 ? "SlowBrush" : "NavSelectedBrush"],
                CornerRadius = new CornerRadius(5),
                Padding = new Thickness(8, 3, 8, 3),
                VerticalAlignment = VerticalAlignment.Center,
                Child = new TextBlock
                {
                    Text = d <= 0 ? "已过期" : $"剩 {d} 天",
                    FontSize = 11,
                    Foreground = d <= 7
                        ? new SolidColorBrush(Colors.White)
                        : (Brush)Application.Current.Resources["TextSecondaryBrush"],
                },
            };
            Grid.SetColumn(badge, 1);
            grid.Children.Add(badge);
        }

        return grid;
    }

    private static UIElement BuildNotice(string msg)
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
                        Glyph = "\uE946",
                        FontSize = 15,
                        Foreground = (Brush)Application.Current.Resources["TextTertiaryBrush"],
                        VerticalAlignment = VerticalAlignment.Center,
                    },
                    new TextBlock
                    {
                        Text = msg,
                        FontSize = 12.5,
                        Foreground = (Brush)Application.Current.Resources["TextSecondaryBrush"],
                        VerticalAlignment = VerticalAlignment.Center,
                        TextWrapping = TextWrapping.Wrap,
                    },
                },
            },
        };
    }

    private static UIElement BuildExpireWarning(int days)
    {
        return new Border
        {
            Background = (Brush)Application.Current.Resources[
                days <= 3 ? "BadBrush" : "SlowBrush"],
            CornerRadius = new CornerRadius(8),
            Padding = new Thickness(14, 10, 14, 10),
            Child = new StackPanel
            {
                Orientation = Orientation.Horizontal,
                Spacing = 10,
                Children =
                {
                    new FontIcon
                    {
                        Glyph = "\uE7BA",
                        FontSize = 16,
                        Foreground = new SolidColorBrush(Colors.White),
                        VerticalAlignment = VerticalAlignment.Center,
                    },
                    new TextBlock
                    {
                        Text = days <= 0
                            ? "订阅已过期，请及时续费或更换"
                            : $"订阅将在 {days} 天后到期",
                        FontSize = 13,
                        Foreground = new SolidColorBrush(Colors.White),
                        VerticalAlignment = VerticalAlignment.Center,
                    },
                },
            },
        };
    }

    /// <summary>处理 traffic:updated 事件。</summary>
    public void OnTrafficEvent(JsonElement data)
    {
        try
        {
            var t = data.Deserialize<TrafficSummary>();
            if (t is null) return;
            _traffic = t;
            Render();
        }
        catch (Exception e)
        {
            Log.Warn($"处理流量事件失败：{e.Message}");
        }
    }
}
