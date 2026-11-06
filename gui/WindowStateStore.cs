using System.Text.Json;

namespace ClashZhengl;

/// <summary>
/// 窗口尺寸/位置的本地记忆。放在 LocalAppData 而非 Roaming，
/// 因为窗口大小是机器相关的（不同显示器分辨率不同）。
/// </summary>
public static class WindowStateStore
{
    private sealed class State
    {
        public int Width { get; set; } = 960;
        public int Height { get; set; } = 660;
        public bool Maximized { get; set; }
    }

    private static readonly object Gate = new();

    private static string FilePath => Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
        "ClashZhengl", "window.json");

    public static (int Width, int Height) LoadSize(int defW, int defH)
    {
        try
        {
            var s = Load();
            if (s is null) return (defW, defH);

            // 防御：屏幕拔掉后尺寸可能比当前屏幕还大，夹到安全范围
            var w = Math.Clamp(s.Width, 820, 3840);
            var h = Math.Clamp(s.Height, 560, 2160);
            return (w, h);
        }
        catch
        {
            return (defW, defH);
        }
    }

    public static bool WasMaximized() => Load()?.Maximized ?? false;

    public static void SaveSize(int w, int h)
    {
        lock (Gate)
        {
            try
            {
                var s = Load() ?? new State();
                s.Width = w;
                s.Height = h;
                Write(s);
            }
            catch (Exception e)
            {
                Core.Log.Warn($"保存窗口尺寸失败：{e.Message}");
            }
        }
    }

    public static void SaveMaximized(bool maximized)
    {
        lock (Gate)
        {
            try
            {
                var s = Load() ?? new State();
                s.Maximized = maximized;
                Write(s);
            }
            catch (Exception e)
            {
                Core.Log.Warn($"保存最大化状态失败：{e.Message}");
            }
        }
    }

    private static State? Load()
    {
        try
        {
            var p = FilePath;
            if (!File.Exists(p)) return null;
            return JsonSerializer.Deserialize<State>(File.ReadAllText(p));
        }
        catch
        {
            return null;
        }
    }

    private static void Write(State s)
    {
        var p = FilePath;
        Directory.CreateDirectory(Path.GetDirectoryName(p)!);
        File.WriteAllText(p, JsonSerializer.Serialize(s,
            new JsonSerializerOptions { WriteIndented = true }));
    }
}
