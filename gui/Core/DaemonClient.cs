using System.Diagnostics;
using System.Net;
using System.Net.Http;
using System.Net.Http.Headers;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace ClashZhengl.Core;

/// <summary>
/// daemon 返回的业务错误。码值原样透传 C.6 契约里的 <c>E_*</c>。
/// </summary>
public sealed class DaemonException : Exception
{
    public string Code { get; }
    public string? Detail { get; }
    public bool Retryable { get; }

    public DaemonException(string code, string message, string? detail, bool retryable)
        : base(message)
    {
        Code = code;
        Detail = detail;
        Retryable = retryable;
    }

    /// <summary>把错误码翻译成给用户看的中文提示（对照 C.6 错误表）。</summary>
    public string FriendlyMessage => Code switch
    {
        "E_NET_OFFLINE" => "网络不可用，请检查连接",
        "E_NET_TIMEOUT" => "网络请求超时",
        "E_NET_DNS" => "域名解析失败",
        "E_NET_TLS" => "TLS 握手失败",
        "E_NET_PROXY" => "代理不可用",
        "E_HTTP_STATUS" => "服务器返回错误",
        "E_SUB_URL_INVALID" => "订阅链接无效",
        "E_SUB_EMPTY" => "订阅内容为空",
        "E_PARSE_YAML" => "订阅格式无法解析",
        "E_PARSE_EMPTY" => "订阅里没有可用节点",
        "E_USERINFO_MISSING" => "订阅未提供流量信息",
        "E_NO_SUBSCRIPTION" => "还没有添加订阅",
        "E_CORE_NOT_RUNNING" => "内核未运行",
        "E_CORE_CRASHED" => "内核意外退出",
        "E_CORE_DOWNLOAD" => "内核下载失败",
        "E_CORE_VERIFY" => "内核校验失败",
        "E_SWITCH_GROUP_MISSING" => "节点组不存在",
        "E_SWITCH_NODE_MISSING" => "节点不存在",
        "E_SPEEDTEST_RUNNING" => "测速正在进行中",
        "E_SPEEDTEST_NO_NODE" => "没有可测速的节点",
        "E_SYSPROXY_SET" => "设置系统代理失败",
        "E_SYSPROXY_VERIFY" => "系统代理设置未生效",
        "E_BUSY" => "操作正在进行中",
        "E_CANCELLED" => "操作已取消",
        "E_UNAUTHORIZED" => "内部鉴权失败，请重启应用",
        _ => Message,
    };
}

/// <summary>
/// daemon 客户端。负责进程发现/启动、HTTP 调用、SSE 事件订阅。
///
/// 设计要点：
/// - **复用已有 daemon**：GUI 崩溃重启后不会打断正在运行的代理。
/// - **token 每次重读**：daemon 重启会换 token，缓存 token 会导致 401。
/// - **业务错误走 200 + ok=false**：只有鉴权失败才是 HTTP 401。
/// </summary>
public sealed class DaemonClient : IAsyncDisposable
{
    private static readonly JsonSerializerOptions JsonOpts = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
        Converters = { new JsonStringEnumConverter(JsonNamingPolicy.SnakeCaseLower) },
    };

    private readonly HttpClient _http;
    private readonly string _runtimeDir;
    private Process? _ownedProcess;
    private string? _token;
    private int _port;

    public int Port => _port;

    public DaemonClient()
    {
        _runtimeDir = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            "ClashZhengl", "runtime");

        _http = new HttpClient(new SocketsHttpHandler
        {
            ConnectTimeout = TimeSpan.FromSeconds(5),
            // daemon 在本机，**绝对不能走系统代理**，否则代理本身没起来时会自锁
            UseProxy = false,
            PooledConnectionLifetime = TimeSpan.FromMinutes(5),
        })
        {
            Timeout = Timeout.InfiniteTimeSpan, // 每个请求单独设超时（订阅拉取可能很久）
        };
    }

    // ========================================================================
    //  启动 / 发现
    // ========================================================================

    /// <summary>
    /// 确保有一个可用的 daemon：优先复用已在跑的，否则启动子进程。
    /// </summary>
    public async Task EnsureStartedAsync(IProgress<string>? progress = null,
                                         CancellationToken ct = default)
    {
        // 1) 尝试复用
        var existing = ReadPort();
        if (existing is > 0 && await TryHealthAsync(existing.Value, ct).ConfigureAwait(false))
        {
            _port = existing.Value;
            progress?.Report("已连接到正在运行的内核服务");
        }
        else
        {
            // 2) 启动新进程
            progress?.Report("正在启动内核服务…");
            StartProcess();
            _port = await WaitForReadyAsync(ct).ConfigureAwait(false);
            progress?.Report("内核服务已就绪");
        }

        _token = ReadToken();
        if (string.IsNullOrWhiteSpace(_token))
        {
            throw new DaemonException("E_UNAUTHORIZED", "无法读取 daemon token", null, false);
        }
    }

    private void StartProcess()
    {
        var exe = ResolveDaemonPath();
        if (!File.Exists(exe))
        {
            throw new DaemonException("E_CORE_DOWNLOAD",
                $"找不到内核服务程序：{exe}", exe, false);
        }

        // 清掉可能残留的端口文件，避免误判为"已运行"
        TryDelete(Path.Combine(_runtimeDir, "daemon.port"));

        var psi = new ProcessStartInfo
        {
            FileName = exe,
            UseShellExecute = false,
            CreateNoWindow = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            WorkingDirectory = Path.GetDirectoryName(exe)!,
        };

        _ownedProcess = Process.Start(psi)
            ?? throw new DaemonException("E_INTERNAL", "无法启动内核服务进程", null, false);

        // daemon 会把端口打到 stdout；同时也写文件，这里只做兜底日志
        _ownedProcess.OutputDataReceived += (_, e) =>
        {
            if (!string.IsNullOrWhiteSpace(e.Data))
                Log.Info($"[daemon] {e.Data}");
        };
        _ownedProcess.ErrorDataReceived += (_, e) =>
        {
            if (!string.IsNullOrWhiteSpace(e.Data))
                Log.Warn($"[daemon] {e.Data}");
        };
        _ownedProcess.BeginOutputReadLine();
        _ownedProcess.BeginErrorReadLine();
    }

    /// <summary>
    /// daemon 与 GUI 同目录（打包后）或在 src-tauri/target/release（开发时）。
    /// </summary>
    private static string ResolveDaemonPath()
    {
        var local = Path.Combine(AppContext.BaseDirectory, "clash-zhengl-daemon.exe");
        if (File.Exists(local)) return local;

        // 开发目录兜底
        var dev = Path.GetFullPath(Path.Combine(
            AppContext.BaseDirectory, "..", "..", "..", "..",
            "src-tauri", "target", "release", "clash-zhengl-daemon.exe"));
        if (File.Exists(dev)) return dev;

        return local;
    }

    private async Task<int> WaitForReadyAsync(CancellationToken ct)
    {
        var deadline = DateTime.UtcNow.AddSeconds(20);
        while (DateTime.UtcNow < deadline)
        {
            ct.ThrowIfCancellationRequested();

            if (_ownedProcess is { HasExited: true })
            {
                throw new DaemonException("E_INTERNAL",
                    $"内核服务启动后立即退出（code={_ownedProcess.ExitCode}）", null, true);
            }

            var port = ReadPort();
            if (port is > 0 && await TryHealthAsync(port.Value, ct).ConfigureAwait(false))
            {
                return port.Value;
            }

            await Task.Delay(200, ct).ConfigureAwait(false);
        }

        throw new DaemonException("E_INTERNAL", "等待内核服务就绪超时（20 秒）", null, true);
    }

    private async Task<bool> TryHealthAsync(int port, CancellationToken ct)
    {
        try
        {
            using var cts = CancellationTokenSource.CreateLinkedTokenSource(ct);
            cts.CancelAfter(TimeSpan.FromMilliseconds(800));
            using var resp = await _http.GetAsync($"http://127.0.0.1:{port}/health", cts.Token)
                .ConfigureAwait(false);
            return resp.IsSuccessStatusCode;
        }
        catch
        {
            return false;
        }
    }

    private int? ReadPort()
    {
        try
        {
            var p = Path.Combine(_runtimeDir, "daemon.port");
            if (!File.Exists(p)) return null;
            var txt = File.ReadAllText(p).Trim();
            return int.TryParse(txt, out var v) && v > 0 ? v : null;
        }
        catch
        {
            return null;
        }
    }

    private string? ReadToken()
    {
        try
        {
            var p = Path.Combine(_runtimeDir, "daemon.token");
            return File.Exists(p) ? File.ReadAllText(p).Trim() : null;
        }
        catch
        {
            return null;
        }
    }

    // ========================================================================
    //  RPC
    // ========================================================================

    /// <summary>
    /// 调用一条 daemon 命令。失败抛 <see cref="DaemonException"/>。
    /// </summary>
    /// <param name="cmd">C.6 契约里的命令名，如 <c>subscription_list</c></param>
    /// <param name="args">参数对象，可为 null</param>
    /// <param name="timeoutSec">超时秒数；订阅拉取/内核下载需要更长</param>
    public async Task<T> CallAsync<T>(string cmd, object? args = null,
                                      int timeoutSec = 30,
                                      CancellationToken ct = default)
    {
        var raw = await CallRawAsync(cmd, args, timeoutSec, ct).ConfigureAwait(false);
        try
        {
            var v = JsonSerializer.Deserialize<T>(raw, JsonOpts);
            if (v is null)
                throw new DaemonException("E_INTERNAL", $"{cmd} 返回空结果", null, false);
            return v;
        }
        catch (JsonException e)
        {
            throw new DaemonException("E_INTERNAL",
                $"{cmd} 返回格式无法解析：{e.Message}", raw.Length > 500 ? raw[..500] : raw, false);
        }
    }

    /// <summary>调用并忽略返回体（用于只有 ok 的命令）。</summary>
    public Task CallAsync(string cmd, object? args = null, int timeoutSec = 30,
                          CancellationToken ct = default)
        => CallRawAsync(cmd, args, timeoutSec, ct);

    private async Task<string> CallRawAsync(string cmd, object? args, int timeoutSec,
                                            CancellationToken ct)
    {
        if (_port <= 0 || string.IsNullOrWhiteSpace(_token))
            throw new DaemonException("E_INTERNAL", "daemon 尚未就绪", null, true);

        var payload = JsonSerializer.Serialize(new { cmd, args = args ?? new { } }, JsonOpts);

        using var req = new HttpRequestMessage(HttpMethod.Post,
            $"http://127.0.0.1:{_port}/rpc")
        {
            Content = new StringContent(payload, Encoding.UTF8, "application/json"),
        };
        req.Headers.Authorization = new AuthenticationHeaderValue("Bearer", _token);

        using var cts = CancellationTokenSource.CreateLinkedTokenSource(ct);
        cts.CancelAfter(TimeSpan.FromSeconds(timeoutSec));

        HttpResponseMessage resp;
        try
        {
            resp = await _http.SendAsync(req, cts.Token).ConfigureAwait(false);
        }
        catch (OperationCanceledException) when (!ct.IsCancellationRequested)
        {
            throw new DaemonException("E_NET_TIMEOUT",
                $"{cmd} 超时（{timeoutSec} 秒）", cmd, true);
        }
        catch (HttpRequestException e)
        {
            throw new DaemonException("E_INTERNAL",
                $"无法连接内核服务：{e.Message}", null, true);
        }

        using (resp)
        {
            // 鉴权失效：重读 token 再试一次（daemon 可能刚重启）
            if (resp.StatusCode == HttpStatusCode.Unauthorized)
            {
                var fresh = ReadToken();
                if (!string.IsNullOrWhiteSpace(fresh) && fresh != _token)
                {
                    _token = fresh;
                    return await CallRawAsync(cmd, args, timeoutSec, ct).ConfigureAwait(false);
                }
                throw new DaemonException("E_UNAUTHORIZED", "内核服务鉴权失败", null, false);
            }

            var body = await resp.Content.ReadAsStringAsync(cts.Token).ConfigureAwait(false);

            DaemonEnvelope? env;
            try
            {
                env = JsonSerializer.Deserialize<DaemonEnvelope>(body, JsonOpts);
            }
            catch (JsonException e)
            {
                throw new DaemonException("E_INTERNAL",
                    $"响应无法解析：{e.Message}", body.Length > 300 ? body[..300] : body, false);
            }

            if (env is null || !env.Ok)
            {
                var err = env?.Error;
                throw new DaemonException(
                    err?.Code ?? "E_INTERNAL",
                    err?.Message ?? "未知错误",
                    err?.Detail,
                    err?.Retryable ?? false);
            }

            return env.Data.ValueKind == JsonValueKind.Undefined ? "null" : env.Data.GetRawText();
        }
    }

    // ========================================================================
    //  事件流（SSE）
    // ========================================================================

    /// <summary>
    /// 订阅 daemon 推送的事件。断线会自动重连，直到 <paramref name="ct"/> 取消。
    /// </summary>
    public async Task SubscribeEventsAsync(Func<string, JsonElement, Task> onEvent,
                                           CancellationToken ct)
    {
        var backoff = TimeSpan.FromSeconds(1);
        while (!ct.IsCancellationRequested)
        {
            try
            {
                using var req = new HttpRequestMessage(HttpMethod.Get,
                    $"http://127.0.0.1:{_port}/events");
                req.Headers.Authorization = new AuthenticationHeaderValue("Bearer", _token);
                req.Headers.Accept.Add(new MediaTypeWithQualityHeaderValue("text/event-stream"));

                using var resp = await _http
                    .SendAsync(req, HttpCompletionOption.ResponseHeadersRead, ct)
                    .ConfigureAwait(false);
                resp.EnsureSuccessStatusCode();

                await using var stream = await resp.Content.ReadAsStreamAsync(ct).ConfigureAwait(false);
                using var reader = new StreamReader(stream, Encoding.UTF8);

                backoff = TimeSpan.FromSeconds(1); // 连上了就重置退避

                while (!reader.EndOfStream && !ct.IsCancellationRequested)
                {
                    var line = await reader.ReadLineAsync(ct).ConfigureAwait(false);
                    if (line is null) break;
                    if (!line.StartsWith("data: ", StringComparison.Ordinal)) continue;

                    var json = line[6..];
                    try
                    {
                        using var doc = JsonDocument.Parse(json);
                        var root = doc.RootElement;
                        var name = root.TryGetProperty("event", out var n) ? n.GetString() : null;
                        if (string.IsNullOrEmpty(name)) continue;

                        var data = root.TryGetProperty("data", out var d) ? d : default;
                        await onEvent(name, data.Clone()).ConfigureAwait(false);
                    }
                    catch (JsonException)
                    {
                        // 单条事件解析失败不应中断整个流
                    }
                }
            }
            catch (OperationCanceledException) when (ct.IsCancellationRequested)
            {
                return;
            }
            catch (Exception e)
            {
                Log.Warn($"事件流断开，{backoff.TotalSeconds:0} 秒后重连：{e.Message}");
            }

            try
            {
                await Task.Delay(backoff, ct).ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                return;
            }

            backoff = TimeSpan.FromSeconds(Math.Min(backoff.TotalSeconds * 2, 15));
        }
    }

    // ========================================================================
    //  停止
    // ========================================================================

    /// <summary>
    /// 停止 daemon。只终止**本进程启动的** daemon；
    /// 复用别人启动的 daemon 时不杀（用户可能还有别的窗口在用）。
    /// </summary>
    public async Task StopOwnedAsync()
    {
        if (_ownedProcess is null) return;

        try
        {
            if (!_ownedProcess.HasExited)
            {
                // 先礼后兵：给它 3 秒自己清理（关系统代理、停内核）
                _ownedProcess.CloseMainWindow();
                using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(3));
                try
                {
                    await _ownedProcess.WaitForExitAsync(cts.Token).ConfigureAwait(false);
                }
                catch (OperationCanceledException)
                {
                    _ownedProcess.Kill(entireProcessTree: true);
                }
            }
        }
        catch (Exception e)
        {
            Log.Warn($"停止 daemon 时出错：{e.Message}");
        }
        finally
        {
            _ownedProcess.Dispose();
            _ownedProcess = null;
        }
    }

    public async ValueTask DisposeAsync()
    {
        await StopOwnedAsync().ConfigureAwait(false);
        _http.Dispose();
    }

    private static void TryDelete(string path)
    {
        try { if (File.Exists(path)) File.Delete(path); } catch { }
    }

    // ========================================================================
    //  响应外壳
    // ========================================================================

    private sealed class DaemonEnvelope
    {
        public bool Ok { get; set; }
        public JsonElement Data { get; set; }
        public DaemonError? Error { get; set; }
    }

    private sealed class DaemonError
    {
        public string Code { get; set; } = "E_INTERNAL";
        public string Message { get; set; } = "";
        public string? Detail { get; set; }
        public bool Retryable { get; set; }
    }
}
