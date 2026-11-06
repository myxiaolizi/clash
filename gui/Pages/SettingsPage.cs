using ClashZhengl.Core;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;

namespace ClashZhengl.Pages;

/// <summary>
/// 设置页：通用 / 内核 / 代理 / 订阅更新 / 测速 / 关于。
/// 所有开关即时写回 daemon，无「保存」按钮（Fluent 惯例：改了就是改了）。
/// </summary>
public sealed class SettingsPage
{
    private readonly StackPanel _host;
    private Settings? _settings;
    private bool _loading;

    public Settings? Settings => _settings;

    public SettingsPage(StackPanel host) => _host = host;

    public async Task InitializeAsync(Settings settings)
    {
        _settings = settings;
        Render();
        await Task.CompletedTask;
    }

    public async Task RefreshAsync()
    {
        try
        {
            _settings = await App.Daemon.CallAsync<Settings>("settings_get");
            Render();
        }
        catch (DaemonException e)
        {
            Log.Error($"加载设置失败 code={e.Code}", e);
            _host.Children.Clear();
            _host.Children.Add(new TextBlock
            {
                Text = e.FriendlyMessage,
                Foreground = (Brush)Application.Current.Resources["BadBrush"],
            });
        }
    }

    private void Render()
    {
        if (_settings is null) return;
        _loading = true;

        _host.Children.Clear();

        _host.Children.Add(new TextBlock
        {
            Text = "设置",
            Style = (Style)Application.Current.Resources["PageTitleStyle"],
        });
        _host.Children.Add(new TextBlock
        {
            Text = "修改立即生效",
            Style = (Style)Application.Current.Resources["PageSubtitleStyle"],
            Margin = new Thickness(0, 4, 0, 6),
        });

        _host.Children.Add(BuildGeneralCard());
        _host.Children.Add(BuildCoreCard());
        _host.Children.Add(BuildPolicyCard());
        _host.Children.Add(BuildAboutCard());

        _loading = false;
    }

    // ========================================================================
    //  通用
    // ========================================================================

    private UIElement BuildGeneralCard()
    {
        var s = _settings!;
        var card = NewCard("通用", out var sp);

        // 主题
        var themeCombo = new ComboBox
        {
            HorizontalAlignment = HorizontalAlignment.Right,
            MinWidth = 140,
        };
        themeCombo.Items.Add("跟随系统");
        themeCombo.Items.Add("浅色");
        themeCombo.Items.Add("深色");
        themeCombo.SelectedIndex = s.Theme switch
        {
            "light" => 1,
            "dark" => 2,
            _ => 0,
        };
        themeCombo.SelectionChanged += async (_, _) =>
        {
            if (_loading) return;
            var theme = themeCombo.SelectedIndex switch
            {
                1 => "light",
                2 => "dark",
                _ => "system",
            };
            await PatchAsync(new { theme });
            ApplyTheme(theme);
        };
        sp.Children.Add(Row("主题", "浅色 / 深色 / 跟随系统", themeCombo));

        // 关闭窗口时最小化到托盘
        var trayToggle = Toggle(s.CloseToTray, async v =>
        {
            await PatchAsync(new { closeToTray = v });
        });
        sp.Children.Add(Row("关闭时最小化到托盘", "关掉后点 × 会直接退出程序", trayToggle));

        // 开机自启
        var autoToggle = Toggle(s.AutoLaunch, async v =>
        {
            await PatchAsync(new { autoLaunch = v });
        });
        sp.Children.Add(Row("开机自动启动", "登录 Windows 后自动运行", autoToggle));

        // 静默启动
        var silentToggle = Toggle(s.SilentStart, async v =>
        {
            await PatchAsync(new { silentStart = v });
        });
        sp.Children.Add(Row("静默启动", "启动时不显示主窗口，只在托盘常驻", silentToggle));

        return card;
    }

    // ========================================================================
    //  内核
    // ========================================================================

    private UIElement BuildCoreCard()
    {
        var s = _settings!;
        var card = NewCard("内核", out var sp);

        var startToggle = Toggle(s.StartCoreOnLaunch, async v =>
        {
            await PatchAsync(new { startCoreOnLaunch = v });
        });
        sp.Children.Add(Row("启动时自动运行内核", "关闭后需要手动点「启动内核」", startToggle));

        // 内核状态 + 重启按钮
        var statusText = new TextBlock
        {
            Text = s.CoreVersion is { Length: > 0 } cv ? $"当前版本 {cv}" : "未安装内核",
            FontSize = 12.5,
            Foreground = (Brush)Application.Current.Resources["TextSecondaryBrush"],
            VerticalAlignment = VerticalAlignment.Center,
        };

        var btnRestart = new Button { Content = "重启内核", FontSize = 12.5 };
        btnRestart.Click += async (_, _) =>
        {
            btnRestart.IsEnabled = false;
            btnRestart.Content = "重启中…";
            try
            {
                await App.Daemon.CallAsync("core_restart", timeoutSec: 60);
                await RefreshAsync();
            }
            catch (DaemonException e)
            {
                Log.Error($"重启内核失败：{e.FriendlyMessage}");
            }
            finally
            {
                btnRestart.IsEnabled = true;
                btnRestart.Content = "重启内核";
            }
        };

        var coreRow = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Spacing = 10,
            HorizontalAlignment = HorizontalAlignment.Right,
        };
        coreRow.Children.Add(statusText);
        coreRow.Children.Add(btnRestart);
        sp.Children.Add(Row("内核状态", null, coreRow));

        // 打开目录
        var btnData = new Button { Content = "打开数据目录", FontSize = 12.5 };
        btnData.Click += async (_, _) =>
        {
            try { await App.Daemon.CallAsync("app_open_data_dir"); }
            catch (DaemonException e) { Log.Warn(e.Message); }
        };

        var btnLog = new Button { Content = "打开日志目录", FontSize = 12.5 };
        btnLog.Click += async (_, _) =>
        {
            try { await App.Daemon.CallAsync("app_open_log_dir"); }
            catch (DaemonException e) { Log.Warn(e.Message); }
        };

        var dirRow = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Spacing = 8,
            HorizontalAlignment = HorizontalAlignment.Right,
        };
        dirRow.Children.Add(btnData);
        dirRow.Children.Add(btnLog);
        sp.Children.Add(Row("文件位置", "排查问题时需要查看", dirRow));

        return card;
    }

    // ========================================================================
    //  自动切换策略
    // ========================================================================

    private UIElement BuildPolicyCard()
    {
        var s = _settings!;
        var card = NewCard("自动切换策略", out var sp);

        sp.Children.Add(new TextBlock
        {
            Text = "内核会定期测速，当当前节点明显变慢时自动切到更快的节点。",
            Style = (Style)Application.Current.Resources["HintTextStyle"],
            Margin = new Thickness(0, 0, 0, 12),
        });

        var enableToggle = Toggle(s.SwitchPolicy.Enabled, async v =>
        {
            await PatchAsync(new { switchPolicy = new { enabled = v } });
        });
        sp.Children.Add(Row("启用自动切换", null, enableToggle));

        var speedToggle = Toggle(s.SpeedtestEnabled, async v =>
        {
            await PatchAsync(new { speedtestEnabled = v });
        });
        sp.Children.Add(Row("启用自动测速", "关闭后不会定时测速，也不会自动切换", speedToggle));

        // 测速间隔
        var intervalBox = new NumberBox
        {
            Value = s.SpeedtestIntervalMin,
            Minimum = 5,
            Maximum = 720,
            SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Compact,
            Width = 130,
            HorizontalAlignment = HorizontalAlignment.Right,
        };
        intervalBox.ValueChanged += async (_, e) =>
        {
            if (_loading) return;
            var v = (int)e.NewValue;
            if (v < 5) return;
            await PatchAsync(new { speedtestIntervalMin = v });
        };
        sp.Children.Add(Row("测速间隔", "单位：分钟，最小 5", intervalBox));

        // 容差
        var toleranceBox = new NumberBox
        {
            Value = s.SwitchPolicy.ToleranceMs,
            Minimum = 0,
            Maximum = 2000,
            SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Compact,
            Width = 130,
            HorizontalAlignment = HorizontalAlignment.Right,
        };
        toleranceBox.ValueChanged += async (_, e) =>
        {
            if (_loading) return;
            await PatchAsync(new
            {
                switchPolicy = new { toleranceMs = (int)e.NewValue },
            });
        };
        sp.Children.Add(Row("切换容差", "新节点比当前快多少毫秒才切换", toleranceBox));

        // 最短驻留
        var dwellBox = new NumberBox
        {
            Value = s.SwitchPolicy.MinDwellSec / 60.0,
            Minimum = 1,
            Maximum = 120,
            SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Compact,
            Width = 130,
            HorizontalAlignment = HorizontalAlignment.Right,
        };
        dwellBox.ValueChanged += async (_, e) =>
        {
            if (_loading) return;
            await PatchAsync(new
            {
                switchPolicy = new { minDwellSec = (int)(e.NewValue * 60) },
            });
        };
        sp.Children.Add(Row("最短驻留时间", "切过去后至少多久不再切换（分钟）", dwellBox));

        return card;
    }

    // ========================================================================
    //  关于
    // ========================================================================

    private UIElement BuildAboutCard()
    {
        var card = NewCard("关于", out var sp);

        var info = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Spacing = 14,
        };

        info.Children.Add(new Image
        {
            Source = new Microsoft.UI.Xaml.Media.Imaging.BitmapImage(
                new Uri("ms-appx:///Assets/app-128.png")),
            Width = 48,
            Height = 48,
        });

        var textSp = new StackPanel { Spacing = 3, VerticalAlignment = VerticalAlignment.Center };
        textSp.Children.Add(new TextBlock
        {
            Text = "Clash Zhengl",
            FontSize = 15,
            FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
            Foreground = (Brush)Application.Current.Resources["TextPrimaryBrush"],
        });
        textSp.Children.Add(new TextBlock
        {
            Text = "版本 0.1.0 · WinUI 3",
            FontSize = 11.5,
            Foreground = (Brush)Application.Current.Resources["TextTertiaryBrush"],
        });
        info.Children.Add(textSp);

        sp.Children.Add(info);

        sp.Children.Add(new TextBlock
        {
            Text = "本软件仅提供代理内核的图形界面，不提供任何服务器或加速服务。" +
                   "请使用合法获得的订阅，并遵守当地法律法规。",
            Style = (Style)Application.Current.Resources["HintTextStyle"],
            Margin = new Thickness(0, 14, 0, 0),
            LineHeight = 19,
        });

        return card;
    }

    // ========================================================================
    //  工具
    // ========================================================================

    private static Border NewCard(string title, out StackPanel content)
    {
        var sp = new StackPanel { Spacing = 0 };

        sp.Children.Add(new TextBlock
        {
            Text = title,
            Style = (Style)Application.Current.Resources["SectionTitleStyle"],
        });

        content = sp;

        return new Border
        {
            Style = (Style)Application.Current.Resources["CardStyle"],
            Child = sp,
        };
    }

    private static UIElement Row(string title, string? subtitle, FrameworkElement control)
    {
        var grid = new Grid { Margin = new Thickness(0, 6, 0, 6) };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var left = new StackPanel { Spacing = 2, VerticalAlignment = VerticalAlignment.Center };
        left.Children.Add(new TextBlock
        {
            Text = title,
            FontSize = 13.5,
            Foreground = (Brush)Application.Current.Resources["TextPrimaryBrush"],
        });

        if (!string.IsNullOrWhiteSpace(subtitle))
        {
            left.Children.Add(new TextBlock
            {
                Text = subtitle,
                FontSize = 11.5,
                Foreground = (Brush)Application.Current.Resources["TextTertiaryBrush"],
                TextWrapping = TextWrapping.Wrap,
            });
        }

        grid.Children.Add(left);

        Grid.SetColumn(control, 1);
        control.VerticalAlignment = VerticalAlignment.Center;
        grid.Children.Add(control);

        return grid;
    }

    private static ToggleSwitch Toggle(bool initial, Func<bool, Task> onChange)
    {
        var t = new ToggleSwitch
        {
            IsOn = initial,
            OnContent = null,
            OffContent = null,
            MinWidth = 0,
            HorizontalAlignment = HorizontalAlignment.Right,
        };

        t.Toggled += async (_, _) =>
        {
            try { await onChange(t.IsOn); }
            catch (DaemonException e)
            {
                Log.Error($"保存设置失败：{e.FriendlyMessage}");
                t.IsOn = !t.IsOn; // 回滚 UI
            }
        };

        return t;
    }

    private async Task PatchAsync(object patch)
    {
        try
        {
            _settings = await App.Daemon.CallAsync<Settings>("settings_update", patch);
        }
        catch (DaemonException e)
        {
            Log.Error($"保存设置失败 code={e.Code}", e);
            throw;
        }
    }

    private void ApplyTheme(string theme)
    {
        var mw = App.MainWindowRef;
        if (mw?.Content is FrameworkElement fe)
        {
            fe.RequestedTheme = theme switch
            {
                "light" => ElementTheme.Light,
                "dark" => ElementTheme.Dark,
                _ => ElementTheme.Default,
            };
        }
    }

    public void PersistCurrentSize()
    {
        // 尺寸由 MainWindow 的 AppWindow.Changed 自动保存，这里仅保留钩子
    }
}
