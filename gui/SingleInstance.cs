using System.IO.Pipes;
using System.Runtime.InteropServices;
using ClashZhengl.Core;

namespace ClashZhengl;

/// <summary>
/// 进程级单实例闸门。
///
/// # 为什么必须做
///
/// 原 Tauri 版出现过「两个托盘图标」的 bug，根因是 tauri.conf.json 声明了 trayIcon
/// 而代码里又建了一个。WinUI 侧托盘由 <see cref="TrayIconManager"/> 唯一创建，
/// 但**更根本的防线是单实例**：如果允许第二个进程起来，它无论如何都会多建一个托盘。
///
/// 因此这里用**命名 Mutex** 在进程启动最早期拦截，
/// 并通过命名管道把「用户又点了一次」转成「把已有窗口调到前台」。
/// </summary>
public static class SingleInstance
{
    private const string MutexName = @"Local\ClashZhengl.SingleInstance";
    private const string PipeName = "ClashZhengl.Activate";

    private static Mutex? _mutex;
    private static CancellationTokenSource? _pipeCts;

    /// <summary>
    /// 尝试成为唯一实例。返回 false 表示已有实例在跑（此时已通知它激活窗口）。
    /// </summary>
    public static bool TryAcquire(Action onActivate)
    {
        _mutex = new Mutex(initiallyOwned: true, MutexName, out var createdNew);
        if (createdNew)
        {
            StartActivationServer(onActivate);
            return true;
        }

        // 已有实例：把"激活"请求通过管道发过去，然后自己退出
        NotifyExisting();
        return false;
    }

    private static void StartActivationServer(Action onActivate)
    {
        _pipeCts = new CancellationTokenSource();
        var ct = _pipeCts.Token;

        _ = Task.Run(async () =>
        {
            while (!ct.IsCancellationRequested)
            {
                try
                {
                    await using var server = new NamedPipeServerStream(
                        PipeName,
                        PipeDirection.In,
                        NamedPipeServerStream.MaxAllowedServerInstances,
                        PipeTransmissionMode.Byte,
                        PipeOptions.Asynchronous);

                    await server.WaitForConnectionAsync(ct).ConfigureAwait(false);

                    // 内容不重要，连上就说明"再来一次"，直接激活
                    onActivate();
                }
                catch (OperationCanceledException)
                {
                    return;
                }
                catch (Exception e)
                {
                    Log.Warn($"激活管道异常：{e.Message}");
                    try { await Task.Delay(300, ct).ConfigureAwait(false); }
                    catch (OperationCanceledException) { return; }
                }
            }
        }, ct);
    }

    private static void NotifyExisting()
    {
        try
        {
            using var client = new NamedPipeClientStream(
                ".", PipeName, PipeDirection.Out, PipeOptions.None);
            // 给已有实例 2 秒时间响应；它可能还在启动中
            client.Connect(2000);
            client.WriteByte(1);
            client.Flush();
        }
        catch (Exception e)
        {
            Log.Warn($"通知已有实例失败：{e.Message}");
        }
    }

    public static void Release()
    {
        try
        {
            _pipeCts?.Cancel();
            _pipeCts?.Dispose();
            _pipeCts = null;

            if (_mutex is not null)
            {
                try { _mutex.ReleaseMutex(); } catch { }
                _mutex.Dispose();
                _mutex = null;
            }
        }
        catch
        {
            // 退出路径不抛
        }
    }
}
