using ClashZhengl.Core;
using ClashZhengl.UI;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;

namespace ClashZhengl;

/// <summary>
/// 应用主体。负责：创建主窗口、拉起 daemon、装配托盘、管理退出清理。
/// </summary>
public partial class App : Application
{
    public static int ExitCode { get; private set; }

    private static MainWindow? _window;
    private static App? _instance;

    private readonly bool _silentStart;
    private readonly DaemonClient _daemon = new();
    private TrayIconManager? _tray;
    private CancellationTokenSource? _lifetime;

    public static DaemonClient Daemon => _instance!._daemon;

    /// <summary>主窗口引用，供设置页切换主题时使用。</summary>
    public static MainWindow? MainWindowRef => _window;

    public App(bool silentStart)
    {
        _instance = this;
        _silentStart = silentStart;
        InitializeComponent();

        UnhandledException += (_, e) =>
        {
            Log.Error("未处理异常", e.Exception);
            e.Handled = true; // 尽量不让 UI 因为一次异常直接死掉
        };
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        _lifetime = new CancellationTokenSource();

        _window = new MainWindow();
        _window.Closed += OnWindowClosed;

        // 托盘是**唯一**一处创建，避免出现两个图标
        _tray = new TrayIconManager(_window);
        _tray.Setup();

        if (!_silentStart)
        {
            _window.Activate();
        }

        // daemon 启动放后台，窗口先出来，避免启动白屏等太久
        _ = BootstrapAsync();
    }

    private async Task BootstrapAsync()
    {
        try
        {
            _window?.SetBootStatus("正在连接内核服务…");

            var progress = new Progress<string>(msg =>
                _window?.SetBootStatus(msg));

            await _daemon.EnsureStartedAsync(progress, _lifetime!.Token)
                .ConfigureAwait(true);

            Log.Info($"daemon 就绪 port={_daemon.Port}");

            // 事件流常驻
            _ = _daemon.SubscribeEventsAsync(OnDaemonEvent, _lifetime.Token);

            // daemon 意外退出后自动拉起，拉起成功后让主窗口重载数据
            _daemon.Recovered += OnDaemonRecovered;

            await _window!.OnDaemonReadyAsync().ConfigureAwait(true);
        }
        catch (DaemonException e)
        {
            Log.Error($"daemon 启动失败 code={e.Code}", e);
            _window?.ShowFatal(e.FriendlyMessage, e.Detail);
        }
        catch (Exception e)
        {
            Log.Error("bootstrap 失败", e);
            _window?.ShowFatal("启动失败", e.Message);
        }
    }

    /// <summary>daemon 重新连上后，把界面数据整体刷一遍。</summary>
    private Task OnDaemonRecovered()
    {
        var w = _window;
        if (w is null) return Task.CompletedTask;

        w.DispatcherQueue.TryEnqueue(async () =>
        {
            try { await w.OnDaemonReadyAsync(); }
            catch (Exception e) { Log.Warn($"恢复后刷新界面失败：{e.Message}"); }
        });

        return Task.CompletedTask;
    }

    private Task OnDaemonEvent(string name, System.Text.Json.JsonElement data)
    {
        // 事件可能来自后台线程，必须切回 UI 线程再碰控件
        _window?.DispatcherQueue.TryEnqueue(() => _window?.HandleDaemonEvent(name, data));
        return Task.CompletedTask;
    }

    private void OnWindowClosed(object sender, WindowEventArgs args)
    {
        // 只有「真退出」才会走到这里。
        //
        // ⚠ 0.1.0 的 bug：自绘的关闭按钮直接调 Window.Close()，
        // 而 Window.Close() **不会**触发 AppWindow.Closing，
        // 于是 MainWindow 里「最小化到托盘」的拦截整段被绕过，
        // 一路走到今天的 Closed → ShutdownAsync，托盘和 daemon 一起没了。
        //
        // 现在窗口按钮交给系统，系统的 × 一定先经过 AppWindow.Closing，
        // 由设置决定是隐藏还是退出。这里再加一道保险：没被显式放行
        // 就不做退出清理，避免以后有人又直接 Close() 把 daemon 带走。
        if (sender is MainWindow w && !w.CloseAllowed)
        {
            Log.Warn("窗口在未放行的情况下被关闭，跳过退出清理（保留托盘与内核服务）");
            return;
        }

        _ = ShutdownAsync();
    }

    private async Task ShutdownAsync()
    {
        try
        {
            Log.Info("开始退出清理");
            _lifetime?.Cancel();
            _tray?.Dispose();

            await _daemon.StopOwnedAsync().ConfigureAwait(false);
            Log.Info("退出清理完成");
        }
        catch (Exception e)
        {
            Log.Error("退出清理出错", e);
        }
        finally
        {
            ExitCode = 0;
        }
    }

    /// <summary>由第二个实例的管道请求触发：把窗口亮到前台。</summary>
    public static void ActivateMainWindow()
    {
        var w = _window;
        if (w is null) return;

        w.DispatcherQueue.TryEnqueue(() =>
        {
            try
            {
                w.ShowFromTray();
            }
            catch (Exception e)
            {
                Log.Warn($"激活窗口失败：{e.Message}");
            }
        });
    }

    /// <summary>立即退出（菜单「退出」用）。</summary>
    public static void QuitNow()
    {
        var w = _window;
        if (w is null)
        {
            ExitCode = 0;
            Current.Exit();
            return;
        }

        w.DispatcherQueue.TryEnqueue(() =>
        {
            w.AllowClose();
            w.Close();
        });
    }
}
