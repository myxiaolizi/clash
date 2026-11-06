using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;

namespace ClashZhengl;

/// <summary>
/// WinUI 3 未打包模式的进程入口。
///
/// 为什么需要手写 Main：打包模式由框架生成入口，而 <c>WindowsPackageType=None</c>
/// （免 MSIX、可直接双击 exe）必须自己初始化 COM 公寓与 <see cref="WinRT.ComWrappersSupport"/>，
/// 遗漏任一步都会在创建窗口时抛 <c>COMException 0x80010106</c>。
/// </summary>
public static class Program
{
    [STAThread]
    public static int Main(string[] args)
    {
        // 静默启动：带 --silent 时窗口不自动显示（登录自启场景）
        var silent = args.Any(a =>
            a.Equals("--silent", StringComparison.OrdinalIgnoreCase) ||
            a.Equals("/silent", StringComparison.OrdinalIgnoreCase));

        Core.Log.Init();
        Core.Log.Info($"GUI 启动 args=[{string.Join(" ", args)}] silent={silent}");

        // 单实例：第二个实例只负责把已有窗口调到前台，然后退出
        var acquired = SingleInstance.TryAcquire(() => App.ActivateMainWindow());
        if (!acquired)
        {
            Core.Log.Info("已有实例在运行，退出");
            return 0;
        }

        try
        {
            WinRT.ComWrappersSupport.InitializeComWrappers();

            Application.Start(p =>
            {
                var ctx = new DispatcherQueueSynchronizationContext(
                    DispatcherQueue.GetForCurrentThread());
                SynchronizationContext.SetSynchronizationContext(ctx);

                _ = new App(silent);
            });

            return App.ExitCode;
        }
        catch (Exception e)
        {
            Core.Log.Error("GUI 启动失败", e);
            return 1;
        }
        finally
        {
            SingleInstance.Release();
        }
    }
}
