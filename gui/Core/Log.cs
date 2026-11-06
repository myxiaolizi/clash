using System.Text;

namespace ClashZhengl.Core;

/// <summary>
/// 极简文件日志。GUI 侧日志写 <c>%LOCALAPPDATA%\ClashZhengl\logs\gui-YYYYMMDD.log</c>，
/// 与 daemon 的 <c>daemon-*.log</c> 分文件，方便排障时区分是 UI 还是内核出的问题。
///
/// 不引第三方日志库：需求只是"写行文本 + 按天滚动 + 限制大小"，
/// 引 Serilog/NLog 会让最终包体增加 1MB+，与轻量目标冲突。
/// </summary>
public static class Log
{
    private static readonly object Gate = new();
    private static string _dir = "";
    private static string _file = "";
    private static long _written;
    private const long MaxBytesPerDay = 4 * 1024 * 1024; // 4MB 上限，超了就停写

    public static void Init()
    {
        _dir = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            "ClashZhengl", "logs");
        Directory.CreateDirectory(_dir);
        Roll();
        CleanupOld();
    }

    public static void Info(string msg) => Write("INFO", msg);
    public static void Warn(string msg) => Write("WARN", msg);
    public static void Error(string msg) => Write("ERROR", msg);

    public static void Error(string msg, Exception e)
        => Write("ERROR", $"{msg} | {e.GetType().Name}: {e.Message}\n{e.StackTrace}");

    private static void Write(string level, string msg)
    {
        if (string.IsNullOrEmpty(_dir)) return;

        var line = $"{DateTime.Now:yyyy-MM-dd HH:mm:ss.fff} [{level}] {msg}";
        lock (Gate)
        {
            try
            {
                if (_written > MaxBytesPerDay) return;

                var today = Path.Combine(_dir, $"gui-{DateTime.Now:yyyyMMdd}.log");
                if (today != _file) Roll();

                var bytes = Encoding.UTF8.GetBytes(line + Environment.NewLine);
                using var fs = new FileStream(_file, FileMode.Append, FileAccess.Write,
                    FileShare.ReadWrite, 4096);
                fs.Write(bytes, 0, bytes.Length);
                _written += bytes.Length;
            }
            catch
            {
                // 日志失败绝不能影响主流程
            }
        }
    }

    private static void Roll()
    {
        _file = Path.Combine(_dir, $"gui-{DateTime.Now:yyyyMMdd}.log");
        _written = File.Exists(_file) ? new FileInfo(_file).Length : 0;
    }

    private static void CleanupOld()
    {
        try
        {
            var cutoff = DateTime.Now.AddDays(-3);
            foreach (var f in Directory.GetFiles(_dir, "gui-*.log"))
            {
                if (File.GetLastWriteTime(f) < cutoff) File.Delete(f);
            }
        }
        catch
        {
            // 清理失败无所谓
        }
    }

    public static string LogDirectory => _dir;
}
