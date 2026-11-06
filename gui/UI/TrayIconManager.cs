using System.Runtime.InteropServices;
using ClashZhengl.Core;

namespace ClashZhengl.UI;

/// <summary>
/// 系统托盘图标。**全局唯一**实例，由 <see cref="App.OnLaunched"/> 创建一次。
///
/// # 为什么手写 P/Invoke 而不用 H.NotifyIcon
///
/// 原 Tauri 版出现「两个托盘图标」的 bug。第三方托盘库通常在
/// 「包引用 + XAML 控件 + 代码创建」三条路径上都可能注册图标，
/// 一旦有一条被重复执行就多一个图标，排查成本高。
/// 这里直接调 <c>Shell_NotifyIcon</c>，创建与销毁点唯一且显式，
/// 配合 <see cref="Dispose"/> 里的 <c>NIM_DELETE</c>，不会残留。
///
/// 需要一个隐藏的 message-only 窗口来接收托盘消息。
/// </summary>
public sealed class TrayIconManager : IDisposable
{
    // ---- Win32 常量 ----
    private const int WM_USER = 0x0400;
    private const int WM_TRAYICON = WM_USER + 1;
    private const int WM_LBUTTONUP = 0x0202;
    private const int WM_RBUTTONUP = 0x0205;
    private const int WM_COMMAND = 0x0111;

    private const int NIM_ADD = 0x00000000;
    private const int NIM_MODIFY = 0x00000001;
    private const int NIM_DELETE = 0x00000002;

    private const int NIF_MESSAGE = 0x00000001;
    private const int NIF_ICON = 0x00000002;
    private const int NIF_TIP = 0x00000004;

    private const int IDI_APPLICATION = 32512;

    private const int MF_STRING = 0x00000000;
    private const int MF_SEPARATOR = 0x00000800;
    private const int MF_GRAYED = 0x00000001;

    private const int TPM_RIGHTBUTTON = 0x0002;
    private const int TPM_RETURNCMD = 0x0100;

    // 菜单命令 ID
    private const int CMD_SHOW = 1001;
    private const int CMD_TOGGLE_PROXY = 1002;
    private const int CMD_TOGGLE_CORE = 1003;
    private const int CMD_QUIT = 1004;

    private readonly MainWindow _window;
    private readonly WndProcDelegate _wndProc;
    private IntPtr _hwnd;
    private IntPtr _hIcon;
    private bool _added;
    private bool _disposed;

    public TrayIconManager(MainWindow window)
    {
        _window = window;
        _wndProc = WndProc; // 必须持有引用，否则回调会被 GC 回收
    }

    /// <summary>创建托盘图标。重复调用会被忽略（幂等），从根上杜绝多图标。</summary>
    public void Setup()
    {
        if (_added) return;

        try
        {
            CreateMessageWindow();
            if (_hwnd == IntPtr.Zero) return;

            if (_hIcon == IntPtr.Zero)
                _hIcon = LoadIconW(IntPtr.Zero, IDI_APPLICATION);

            var data = new NOTIFYICONDATAW
            {
                cbSize = Marshal.SizeOf<NOTIFYICONDATAW>(),
                hWnd = _hwnd,
                uID = 1,
                uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP,
                uCallbackMessage = WM_TRAYICON,
                hIcon = _hIcon,
                szTip = "Clash Zhengl",
            };

            _added = Shell_NotifyIconW(NIM_ADD, ref data);
            Log.Info($"托盘图标已添加: {_added}");
        }
        catch (Exception e)
        {
            Log.Error("创建托盘图标失败", e);
        }
    }

    private void CreateMessageWindow()
    {
        var hInstance = GetModuleHandleW(null);

        var wc = new WNDCLASSEXW
        {
            cbSize = Marshal.SizeOf<WNDCLASSEXW>(),
            lpfnWndProc = Marshal.GetFunctionPointerForDelegate(_wndProc),
            hInstance = hInstance,
            lpszClassName = "ClashZhenglTrayWnd",
        };

        var atom = RegisterClassExW(ref wc);
        if (atom == 0)
        {
            var err = Marshal.GetLastWin32Error();
            // 1410 = 类已注册（重复调用），不是错误
            if (err != 1410)
            {
                Log.Warn($"注册托盘窗口类失败 err={err}");
            }
        }

        _hwnd = CreateWindowExW(0, "ClashZhenglTrayWnd", "ClashZhenglTray",
            0, 0, 0, 0, 0, IntPtr.Zero, IntPtr.Zero, hInstance, IntPtr.Zero);

        if (_hwnd == IntPtr.Zero)
        {
            Log.Warn($"创建托盘窗口失败 err={Marshal.GetLastWin32Error()}");
        }
    }

    private IntPtr WndProc(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam)
    {
        switch (msg)
        {
            case WM_TRAYICON:
                var evt = lParam.ToInt32();
                if (evt == WM_LBUTTONUP)
                {
                    _window.DispatcherQueue.TryEnqueue(() => _window.ShowFromTray());
                }
                else if (evt == WM_RBUTTONUP)
                {
                    ShowMenu();
                }
                return IntPtr.Zero;

            case WM_COMMAND:
                var cmd = (int)(wParam.ToInt64() & 0xFFFF);
                HandleMenu(cmd);
                return IntPtr.Zero;
        }

        return DefWindowProcW(hwnd, msg, wParam, lParam);
    }

    private void ShowMenu()
    {
        try
        {
            var hMenu = CreatePopupMenu();
            if (hMenu == IntPtr.Zero) return;

            try
            {
                AppendMenuW(hMenu, MF_STRING, CMD_SHOW, "显示主窗口");
                AppendMenuW(hMenu, MF_SEPARATOR, 0, null);
                AppendMenuW(hMenu, MF_STRING, CMD_TOGGLE_PROXY, "切换系统代理");
                AppendMenuW(hMenu, MF_STRING, CMD_TOGGLE_CORE, "重启内核");
                AppendMenuW(hMenu, MF_SEPARATOR, 0, null);
                AppendMenuW(hMenu, MF_STRING, CMD_QUIT, "退出");

                GetCursorPos(out var pt);
                // 必须置前台，否则点菜单外部菜单不消失
                SetForegroundWindow(_hwnd);

                var cmd = TrackPopupMenu(hMenu,
                    TPM_RIGHTBUTTON | TPM_RETURNCMD,
                    pt.X, pt.Y, 0, _hwnd, IntPtr.Zero);

                if (cmd > 0) HandleMenu(cmd);
            }
            finally
            {
                DestroyMenu(hMenu);
            }
        }
        catch (Exception e)
        {
            Log.Error("显示托盘菜单失败", e);
        }
    }

    private void HandleMenu(int cmd)
    {
        switch (cmd)
        {
            case CMD_SHOW:
                _window.DispatcherQueue.TryEnqueue(() => _window.ShowFromTray());
                break;
            case CMD_TOGGLE_PROXY:
                _ = _window.ToggleSysProxyAsync();
                break;
            case CMD_TOGGLE_CORE:
                _ = _window.RestartCoreAsync();
                break;
            case CMD_QUIT:
                App.QuitNow();
                break;
        }
    }

    public void Dispose()
    {
        if (_disposed) return;
        _disposed = true;

        try
        {
            if (_added)
            {
                var data = new NOTIFYICONDATAW
                {
                    cbSize = Marshal.SizeOf<NOTIFYICONDATAW>(),
                    hWnd = _hwnd,
                    uID = 1,
                };
                Shell_NotifyIconW(NIM_DELETE, ref data);
                _added = false;
                Log.Info("托盘图标已移除");
            }

            if (_hwnd != IntPtr.Zero)
            {
                DestroyWindow(_hwnd);
                _hwnd = IntPtr.Zero;
            }

            UnregisterClassW("ClashZhenglTrayWnd", GetModuleHandleW(null));
        }
        catch (Exception e)
        {
            Log.Warn($"释放托盘资源出错：{e.Message}");
        }
    }

    // ========================================================================
    //  P/Invoke
    // ========================================================================

    private delegate IntPtr WndProcDelegate(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam);

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct NOTIFYICONDATAW
    {
        public int cbSize;
        public IntPtr hWnd;
        public uint uID;
        public uint uFlags;
        public uint uCallbackMessage;
        public IntPtr hIcon;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)]
        public string szTip;
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct WNDCLASSEXW
    {
        public int cbSize;
        public uint style;
        public IntPtr lpfnWndProc;
        public int cbClsExtra;
        public int cbWndExtra;
        public IntPtr hInstance;
        public IntPtr hIcon;
        public IntPtr hCursor;
        public IntPtr hbrBackground;
        [MarshalAs(UnmanagedType.LPWStr)] public string? lpszMenuName;
        [MarshalAs(UnmanagedType.LPWStr)] public string lpszClassName;
        public IntPtr hIconSm;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct POINT { public int X; public int Y; }

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern ushort RegisterClassExW(ref WNDCLASSEXW wc);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool UnregisterClassW(string className, IntPtr hInstance);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr CreateWindowExW(int exStyle, string className, string windowName,
        int style, int x, int y, int w, int h, IntPtr parent, IntPtr menu, IntPtr hInstance, IntPtr param);

    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool DestroyWindow(IntPtr hwnd);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern IntPtr DefWindowProcW(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam);

    [DllImport("shell32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool Shell_NotifyIconW(int msg, ref NOTIFYICONDATAW data);

    [DllImport("user32.dll")]
    private static extern IntPtr LoadIconW(IntPtr hInstance, int iconId);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
    private static extern IntPtr GetModuleHandleW(string? moduleName);

    [DllImport("user32.dll")]
    private static extern IntPtr CreatePopupMenu();

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern bool AppendMenuW(IntPtr hMenu, int flags, int id, string? item);

    [DllImport("user32.dll")]
    private static extern bool DestroyMenu(IntPtr hMenu);

    [DllImport("user32.dll")]
    private static extern int TrackPopupMenu(IntPtr hMenu, int flags, int x, int y,
        int reserved, IntPtr hwnd, IntPtr rect);

    [DllImport("user32.dll")]
    private static extern bool GetCursorPos(out POINT pt);

    [DllImport("user32.dll")]
    private static extern bool SetForegroundWindow(IntPtr hwnd);
}
