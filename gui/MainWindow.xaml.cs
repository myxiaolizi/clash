using System.Text.Json;
using ClashZhengl.Core;
using ClashZhengl.UI;
using Microsoft.UI;
using Microsoft.UI.Composition.SystemBackdrops;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Windows.Graphics;
using WinRT.Interop;

namespace ClashZhengl;

public sealed partial class MainWindow : Window
{
    private readonly Pages.HomePage _pageHome;
    private readonly Pages.SubscriptionsPage _pageSubs;
    private readonly Pages.TrafficPage _pageTraffic;
    private readonly Pages.NodesPage _pageNodes;
    private readonly Pages.RulesPage _pageRules;
    private readonly Pages.SettingsPage _pageSettings;

    private readonly DispatcherTimer _statusTimer;
    private readonly DispatcherTimer _homeTimer;
    private bool _allowClose;
    private bool _bootDone;
    private string _currentPage = "home";

    public MainWindow()
    {
        InitializeComponent();

        Title = "Clash Zhengl";
        ExtendsContentIntoTitleBar = true;
        SetTitleBar(DragArea);

        ApplyBackdrop();
        ConfigureWindowSize();

        _pageHome = new Pages.HomePage(HomeHost);
        _pageSubs = new Pages.SubscriptionsPage(SubscriptionsHost);
        _pageTraffic = new Pages.TrafficPage(TrafficHost);
        _pageNodes = new Pages.NodesPage(NodesHost);
        _pageRules = new Pages.RulesPage(RulesHost);
        _pageSettings = new Pages.SettingsPage(SettingsHost);

        // 窗口关闭：默认最小化到托盘，只有"真退出"才真关
        AppWindow.Closing += OnAppWindowClosing;

        // 状态栏低频刷新（事件不可靠时兜底）
        _statusTimer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(2) };
        _statusTimer.Tick += async (_, _) => await RefreshStatusAsync();
        _statusTimer.Start();

        // 首页数据兜底刷新。事件覆盖不到的状态变化（内核自动重启、
        // 系统代理被别的程序改动）靠这个保证不长期停在旧值。
        // 5 秒一次：首页要显示实时速率，太慢会显得卡顿，太快又白耗 CPU。
        _homeTimer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(5) };
        _homeTimer.Tick += async (_, _) => await _pageHome.TickAsync();
        _homeTimer.Start();

        // 主题跟随系统
        if (Content is FrameworkElement fe)
        {
            fe.ActualThemeChanged += (_, _) => ApplyBackdrop();
        }
    }

    // ========================================================================
    //  外观
    // ========================================================================

    /// <summary>
    /// 应用 Mica 材质。Windows 11 22H2+ 用 Mica Alt，更早的版本退到 Acrylic，
    /// 都不支持时退回纯色，保证任何机器上都有背景。
    /// </summary>
    private void ApplyBackdrop()
    {
        try
        {
            var dark = Content is FrameworkElement fe
                && fe.ActualTheme == ElementTheme.Dark;

            SystemBackdrop = new MicaBackdrop
            {
                Kind = MicaKind.BaseAlt,
            };
        }
        catch (Exception e)
        {
            Log.Warn($"Mica 不可用，退回纯色：{e.Message}");
            try
            {
                SystemBackdrop = new DesktopAcrylicBackdrop();
            }
            catch
            {
                if (Content is Grid g)
                    g.Background = (Brush)Application.Current.Resources["AppBgBrush"];
            }
        }
    }

    private void ConfigureWindowSize()
    {
        var hwnd = WindowNative.GetWindowHandle(this);
        var id = Win32Interop.GetWindowIdFromWindow(hwnd);
        var appWindow = AppWindow.GetFromWindowId(id);

        // 记住上次尺寸
        var (w, h) = WindowStateStore.LoadSize(960, 660);
        appWindow.Resize(new SizeInt32(w, h));

        var area = DisplayArea.GetFromWindowId(id, DisplayAreaFallback.Primary);
        var wa = area.WorkArea;
        appWindow.Move(new PointInt32(
            wa.X + (wa.Width - w) / 2,
            wa.Y + (wa.Height - h) / 2));

        if (WindowStateStore.WasMaximized())
        {
            if (appWindow.Presenter is OverlappedPresenter p)
                p.Maximize();
        }

        appWindow.Changed += (_, e) =>
        {
            if (e.DidSizeChange)
            {
                var s = appWindow.Size;
                if (s.Width > 300 && s.Height > 300)
                    WindowStateStore.SaveSize(s.Width, s.Height);
            }

            // 窗口按钮改由系统提供后，最大/还原不再经过我们的 Click 回调，
            // 只能在这里记录最大化状态。
            if (e.DidPresenterChange && appWindow.Presenter is OverlappedPresenter op)
            {
                WindowStateStore.SaveMaximized(op.State == OverlappedPresenterState.Maximized);
            }
        };
    }

    // ========================================================================
    //  关闭语义
    // ========================================================================

    /// <summary>
    /// 关闭时「最小化到托盘」还是「真正退出」。
    ///
    /// 关键点：点 × 只隐藏窗口，**绝不**取消/停止 daemon。
    /// 只有托盘菜单的「退出」会调 <see cref="AllowClose"/> 再 Close()，
    /// 这时才允许 App 做退出清理并把 daemon 一起停掉。
    /// </summary>
    private void OnAppWindowClosing(AppWindow sender, AppWindowClosingEventArgs args)
    {
        if (_allowClose) return;

        var closeToTray = _pageSettings?.Settings?.CloseToTray ?? true;
        if (closeToTray)
        {
            // 取消真实关闭，只藏窗口：托盘图标和内核服务都继续活着
            args.Cancel = true;
            HideToTray();
            _pageSettings?.PersistCurrentSize();
        }
        else
        {
            // 用户明确选择了「关闭即退出」，放行这次关闭
            _allowClose = true;
        }
    }

    /// <summary>是否已被显式放行关闭（供 App 判断要不要做退出清理）。</summary>
    public bool CloseAllowed => _allowClose;

    private void HideToTray()
    {
        AppWindow.Hide();
        Log.Info("窗口已最小化到托盘");
    }

    /// <summary>托盘点击 / 第二实例激活时调用。</summary>
    public void ShowFromTray()
    {
        try
        {
            AppWindow.Show();
            Activate();

            if (AppWindow.Presenter is OverlappedPresenter { State: OverlappedPresenterState.Minimized } p)
                p.Restore();
        }
        catch (Exception e)
        {
            Log.Warn($"显示窗口失败：{e.Message}");
        }
    }

    public void AllowClose()
    {
        _allowClose = true;
        _statusTimer.Stop();
        _homeTimer.Stop();
    }

    // ========================================================================
    //  启动 / 致命错误
    // ========================================================================

    public void SetBootStatus(string text)
    {
        if (DispatcherQueue.HasThreadAccess)
            BootStatusText.Text = text;
        else
            DispatcherQueue.TryEnqueue(() => BootStatusText.Text = text);
    }

    public void ShowFatal(string message, string? detail)
    {
        DispatcherQueue.TryEnqueue(async () =>
        {
            BootOverlay.Visibility = Visibility.Collapsed;
            var dlg = new ContentDialog
            {
                XamlRoot = Content.XamlRoot,
                Title = "启动失败",
                Content = string.IsNullOrWhiteSpace(detail)
                    ? message
                    : $"{message}\n\n{detail}",
                CloseButtonText = "关闭",
                DefaultButton = ContentDialogButton.Close,
            };
            try { await dlg.ShowAsync(); } catch { }
        });
    }

    public async Task OnDaemonReadyAsync()
    {
        try
        {
            // 合规提示（首次启动）
            var settings = await App.Daemon.CallAsync<Settings>("settings_get")
                .ConfigureAwait(true);

            if (!settings.ComplianceAccepted)
            {
                await ShowComplianceAsync().ConfigureAwait(true);
            }

            // 主题与语言
            ApplyTheme(settings.Theme);

            await _pageSettings.InitializeAsync(settings).ConfigureAwait(true);
            await _pageHome.InitializeAsync().ConfigureAwait(true);
            await _pageSubs.InitializeAsync().ConfigureAwait(true);
            await _pageTraffic.InitializeAsync().ConfigureAwait(true);
            await _pageNodes.InitializeAsync().ConfigureAwait(true);
            await _pageRules.InitializeAsync().ConfigureAwait(true);
            await RefreshStatusAsync().ConfigureAwait(true);

            _bootDone = true;
            BootOverlay.Visibility = Visibility.Collapsed;
        }
        catch (DaemonException e)
        {
            Log.Error($"页面初始化失败 code={e.Code}", e);
            ShowFatal(e.FriendlyMessage, e.Detail);
        }
    }

    private async Task ShowComplianceAsync()
    {
        var dlg = new ContentDialog
        {
            XamlRoot = Content.XamlRoot,
            Title = "使用须知",
            Content = new TextBlock
            {
                Text = "本软件是代理内核的图形界面，本身不提供任何代理服务器或网络加速服务。\n\n" +
                       "您需要使用自己合法获得的订阅链接。请遵守当地法律法规以及服务提供商的使用条款。\n\n" +
                       "作者不对因使用本软件产生的任何后果承担责任。继续使用即表示您已理解并接受上述说明。",
                TextWrapping = TextWrapping.Wrap,
                LineHeight = 22,
            },
            PrimaryButtonText = "我已了解并同意",
            DefaultButton = ContentDialogButton.Primary,
        };

        var result = await dlg.ShowAsync();
        if (result == ContentDialogResult.Primary)
        {
            await App.Daemon.CallAsync("app_accept_compliance").ConfigureAwait(true);
        }
    }

    private void ApplyTheme(string theme)
    {
        if (Content is not FrameworkElement fe) return;

        fe.RequestedTheme = theme switch
        {
            "light" => ElementTheme.Light,
            "dark" => ElementTheme.Dark,
            _ => ElementTheme.Default,
        };
    }

    // ========================================================================
    //  导航
    // ========================================================================

    private void OnNavSelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (NavList.SelectedItem is not ListViewItem item) return;
        var tag = item.Tag as string ?? "home";
        if (tag == _currentPage) return;

        _currentPage = tag;

        PageHome.Visibility = tag == "home" ? Visibility.Visible : Visibility.Collapsed;
        PageSubscriptions.Visibility = tag == "subscriptions" ? Visibility.Visible : Visibility.Collapsed;
        PageTraffic.Visibility = tag == "traffic" ? Visibility.Visible : Visibility.Collapsed;
        PageNodes.Visibility = tag == "nodes" ? Visibility.Visible : Visibility.Collapsed;
        PageRules.Visibility = tag == "rules" ? Visibility.Visible : Visibility.Collapsed;
        PageSettings.Visibility = tag == "settings" ? Visibility.Visible : Visibility.Collapsed;

        // 只有首页在前台时才跑定时刷新，后台页不空转
        _pageHome.SetActive(tag == "home");

        _ = tag switch
        {
            "home" => _pageHome.RefreshAsync(),
            "subscriptions" => _pageSubs.RefreshAsync(),
            "traffic" => _pageTraffic.RefreshAsync(),
            "nodes" => _pageNodes.RefreshAsync(),
            "rules" => _pageRules.RefreshAsync(),
            "settings" => _pageSettings.RefreshAsync(),
            _ => Task.CompletedTask,
        };
    }

    // ========================================================================
    //  状态栏
    // ========================================================================

    private async Task RefreshStatusAsync()
    {
        if (!_bootDone) return;

        try
        {
            var rt = await App.Daemon.CallAsync<RuntimeState>("runtime_get")
                .ConfigureAwait(true);

            // 内核状态
            if (rt.CoreRunning)
            {
                CoreDot.Fill = (Brush)Application.Current.Resources["GoodBrush"];
                CoreStatusText.Text = rt.MixedPort is int mp
                    ? $"内核：运行中 · 127.0.0.1:{mp}"
                    : "内核：运行中";
                StatusChipText.Text = "运行中";
            }
            else
            {
                CoreDot.Fill = (Brush)Application.Current.Resources["TextTertiaryBrush"];
                CoreStatusText.Text = "内核：未启动";
                StatusChipText.Text = "已停止";
            }

            // 系统代理（runtime_get 里是扁平字段）
            if (rt.SysproxyEnabled && rt.SysproxyManaged)
            {
                ProxyStatusText.Text = $"系统代理：已启用 {rt.SysproxyServer}";
            }
            else if (rt.SysproxyEnabled)
            {
                ProxyStatusText.Text = "系统代理：已启用（非本程序设置）";
            }
            else
            {
                ProxyStatusText.Text = "系统代理：未启用";
            }

            // 流量：优先显示实时速率，没有速率时显示累计
            SpeedText.Text =
                (rt.UpRate > 0 || rt.DownRate > 0)
                    ? $"↑ {FormatRate(rt.UpRate)}  ↓ {FormatRate(rt.DownRate)}"
                    : $"↑ {FormatBytes(rt.TrafficUp)}  ↓ {FormatBytes(rt.TrafficDown)}";

            _pageNodes.ApplyRuntime(rt);
        }
        catch (DaemonException e) when (e.Code is "E_INTERNAL" or "E_UNAUTHORIZED")
        {
            // daemon 可能重启中，静默忽略，下一轮再试
        }
        catch (Exception e)
        {
            Log.Warn($"刷新状态失败：{e.Message}");
        }
    }

    // ========================================================================
    //  事件分发
    // ========================================================================

    /// <summary>
    /// daemon 事件分发。
    ///
    /// 事件名必须逐字对齐 Rust 侧实际 emit 的字符串——
    /// 0.1.0 监听的是 <c>sysproxy:state</c>，而 Rust 实际发的是
    /// <c>sysproxy:changed</c>，所以系统代理状态一直不跟随。
    /// 这里两种名字都接住，后端改名也不至于失效。
    /// </summary>
    public void HandleDaemonEvent(string name, JsonElement data)
    {
        try
        {
            switch (name)
            {
                case "core:state":
                    _pageHome.OnCoreEvent(data);
                    _pageNodes.HandleEvent(name, data);
                    _ = RefreshStatusAsync();
                    break;

                case "core:installed":
                case "core:installFailed":
                case "core:downloadProgress":
                case "core:install-progress":
                    _pageNodes.HandleEvent(name, data);
                    _pageSettings.OnCoreEvent(name, data);
                    break;

                case "speedtest:started":
                case "speedtest:done":
                case "speedtest:progress":
                    _pageNodes.HandleEvent(name, data);
                    break;

                case "subscription:added":
                case "subscription:updated":
                case "subscription:removed":
                case "subscription:fetched":
                case "subscription:fetchedAll":
                case "subscription:applied":
                    _ = _pageSubs.RefreshAsync();
                    _ = _pageHome.RefreshAsync();
                    break;

                // 兼容两种命名：Rust 实际发的是 sysproxy:changed
                case "sysproxy:changed":
                case "sysproxy:state":
                    _pageHome.OnSysProxyEvent(data);
                    _ = RefreshStatusAsync();
                    break;

                case "settings:changed":
                    _pageHome.OnSettingsEvent(data);
                    break;

                case "switch:applied":
                case "switch:manual":
                    _pageNodes.OnSwitchApplied(data);
                    break;

                case "traffic:updated":
                    _pageHome.OnTrafficEvent(data);
                    _pageTraffic.OnTrafficEvent(data);
                    break;

                // 规则变更后立刻重载内核，首页与规则页都要刷新
                case "rule:applied":
                    _ = _pageRules.RefreshAsync();
                    _ = _pageHome.RefreshAsync();
                    break;

                // 分流模式被改了（可能来自托盘或别处），首页的选择器要跟上
                case "mode:changed":
                    _ = _pageHome.RefreshAsync();
                    break;

                // 流量统计被重置（如「重置今日」），刷新趋势图
                case "traffic:reset":
                    _ = _pageHome.RefreshAsync();
                    _ = _pageTraffic.RefreshAsync();
                    break;

                // daemon 自己要退出了：这里的意图是「整个程序退出」，
                // 不该触发自动重连把它再拉起来。让 GUI 跟着退出。
                case "daemon:shutdown":
                    Log.Info("daemon 通知退出，程序随之关闭");
                    App.QuitNow();
                    break;

                default:
                    Log.Info($"未处理事件: {name}");
                    break;
            }
        }
        catch (Exception e)
        {
            Log.Warn($"处理事件 {name} 失败：{e.Message}");
        }
    }

    // ========================================================================
    //  托盘动作（供 TrayIconManager 调用）
    // ========================================================================

    public string TrayProxyText => ProxyStatusText.Text;
    public string TrayCoreText => CoreStatusText.Text;

    public async Task ToggleSysProxyAsync()
    {
        try
        {
            var rt = await App.Daemon.CallAsync<RuntimeState>("runtime_get")
                .ConfigureAwait(true);

            if (rt.SysproxyEnabled && rt.SysproxyManaged)
                await App.Daemon.CallAsync("sysproxy_disable").ConfigureAwait(true);
            else
                await App.Daemon.CallAsync("sysproxy_enable").ConfigureAwait(true);

            await RefreshStatusAsync().ConfigureAwait(true);
        }
        catch (DaemonException e)
        {
            Log.Error($"切换系统代理失败：{e.FriendlyMessage}");
        }
    }

    public async Task RestartCoreAsync()
    {
        try
        {
            SetBootStatus("正在重启内核…");
            await App.Daemon.CallAsync("core_restart", timeoutSec: 60).ConfigureAwait(true);
            await RefreshStatusAsync().ConfigureAwait(true);
        }
        catch (DaemonException e)
        {
            Log.Error($"重启内核失败：{e.FriendlyMessage}");
        }
    }

    // ========================================================================
    //  工具
    // ========================================================================

    public static string FormatBytes(long bytes)
    {
        if (bytes <= 0) return "0 B";
        string[] units = ["B", "KB", "MB", "GB", "TB"];
        double v = bytes;
        int i = 0;
        while (v >= 1024 && i < units.Length - 1) { v /= 1024; i++; }
        return i == 0 ? $"{v:0} {units[i]}" : $"{v:0.##} {units[i]}";
    }

    /// <summary>速率格式化：daemon 给的 upRate/downRate 单位是字节/秒。</summary>
    public static string FormatRate(long bytesPerSec) => $"{FormatBytes(bytesPerSec)}/s";

    public static string FormatDelay(int? ms) => ms switch
    {
        null => "—",
        < 200 => $"{ms} ms",
        _ => $"{ms} ms",
    };

    /// <summary>延迟分档对应的资源键，供柱状条等着色使用。</summary>
    public static string DelayKey(int? ms) => ms switch
    {
        null => "TextTertiaryBrush",
        < 200 => "GoodBrush",
        < 500 => "OkBrush",
        < 1000 => "SlowBrush",
        _ => "BadBrush",
    };

    /// <summary>延迟分档着色：绿/黄/橙/红。</summary>
    public static Brush DelayBrush(int? ms)
        => (Brush)Application.Current.Resources[DelayKey(ms)];
}
