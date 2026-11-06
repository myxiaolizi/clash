using ClashZhengl.Core;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;

namespace ClashZhengl.Pages;

/// <summary>
/// 订阅页：列表 + 添加/编辑/更新/删除/应用配置。
///
/// 0.1.1 增强：卡片展示月度流量进度条、到期警示、用户名、机场地址、备注；
/// 「编辑」对话框可改全部管理信息；添加时可展开「更多信息」。
/// </summary>
public sealed class SubscriptionsPage
{
    private readonly StackPanel _host;

    public SubscriptionsPage(StackPanel host) => _host = host;

    public Task InitializeAsync() => RefreshAsync();

    public async Task RefreshAsync()
    {
        try
        {
            var subs = await App.Daemon.CallAsync<List<Subscription>>("subscription_list");

            _host.Children.Clear();
            _host.Children.Add(BuildHeader(subs));

            if (subs.Count == 0)
            {
                _host.Children.Add(BuildEmptyState());
                return;
            }

            foreach (var s in subs)
            {
                _host.Children.Add(BuildCard(s));
            }
        }
        catch (DaemonException e)
        {
            Log.Error($"加载订阅失败 code={e.Code}", e);
            ShowError(_host, e.FriendlyMessage);
        }
    }

    // ========================================================================
    //  头部
    // ========================================================================

    private UIElement BuildHeader(List<Subscription> subs)
    {
        var grid = new Grid();
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var left = new StackPanel();
        left.Children.Add(new TextBlock
        {
            Text = "订阅",
            Style = (Style)Application.Current.Resources["PageTitleStyle"],
        });
        left.Children.Add(new TextBlock
        {
            Text = subs.Count == 0
                ? "添加订阅链接后即可拉取节点"
                : $"共 {subs.Count} 个订阅，已启用 {subs.Count(s => s.Enabled)} 个",
            Style = (Style)Application.Current.Resources["PageSubtitleStyle"],
        });
        grid.Children.Add(left);

        var btns = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Spacing = 8,
            VerticalAlignment = VerticalAlignment.Top,
        };

        var btnAdd = new Button
        {
            Content = "添加订阅",
            Style = (Style)Application.Current.Resources["PrimaryButtonStyle"],
        };
        btnAdd.Click += async (_, _) => await ShowAddDialogAsync();
        btns.Children.Add(btnAdd);

        var btnRefresh = new Button { Content = "全部更新" };
        btnRefresh.Click += async (_, _) =>
        {
            btnRefresh.IsEnabled = false;
            btnRefresh.Content = "更新中…";
            try
            {
                await App.Daemon.CallAsync("subscription_fetch_all", timeoutSec: 120);
                await RefreshAsync();
            }
            catch (DaemonException e)
            {
                Log.Error($"更新全部订阅失败：{e.FriendlyMessage}");
                await ShowInfoAsync(e.FriendlyMessage);
            }
            finally
            {
                btnRefresh.IsEnabled = true;
                btnRefresh.Content = "全部更新";
            }
        };
        btns.Children.Add(btnRefresh);

        Grid.SetColumn(btns, 1);
        grid.Children.Add(btns);

        return grid;
    }

    private UIElement BuildEmptyState()
    {
        var card = new Border
        {
            Style = (Style)Application.Current.Resources["CardStyle"],
            Padding = new Thickness(32),
        };

        var sp = new StackPanel
        {
            HorizontalAlignment = HorizontalAlignment.Center,
            Spacing = 12,
        };

        sp.Children.Add(new FontIcon
        {
            Glyph = "\uE8A5",
            FontSize = 32,
            Foreground = (Brush)Application.Current.Resources["TextTertiaryBrush"],
            HorizontalAlignment = HorizontalAlignment.Center,
        });

        sp.Children.Add(new TextBlock
        {
            Text = "还没有订阅",
            FontSize = 15,
            HorizontalAlignment = HorizontalAlignment.Center,
            Foreground = (Brush)Application.Current.Resources["TextPrimaryBrush"],
        });

        sp.Children.Add(new TextBlock
        {
            Text = "点击右上角「添加订阅」粘贴你的订阅链接",
            Style = (Style)Application.Current.Resources["HintTextStyle"],
            HorizontalAlignment = HorizontalAlignment.Center,
        });

        card.Child = sp;
        return card;
    }

    // ========================================================================
    //  卡片
    // ========================================================================

    private UIElement BuildCard(Subscription s)
    {
        var card = new Border
        {
            Style = (Style)Application.Current.Resources["CardStyle"],
        };

        var root = new StackPanel { Spacing = 0 };

        root.Children.Add(BuildCardHeader(s));

        // 月度流量进度条（有总量数据才显示）
        if (s.Traffic is { } t && (t.HasTotal || t.Unlimited))
        {
            root.Children.Add(BuildTrafficBar(t));
        }

        root.Children.Add(BuildCardStats(s));

        // 备注
        if (s.Remark is { Length: > 0 } remark)
        {
            root.Children.Add(new TextBlock
            {
                Text = remark,
                FontSize = 12,
                Margin = new Thickness(0, 10, 0, 0),
                TextWrapping = TextWrapping.Wrap,
                Foreground = (Brush)Application.Current.Resources["TextSecondaryBrush"],
            });
        }

        root.Children.Add(BuildCardActions(s));

        card.Child = root;
        return card;
    }

    private UIElement BuildCardHeader(Subscription s)
    {
        var row = new Grid();
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var left = new StackPanel { Spacing = 3 };

        var nameRow = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        nameRow.Children.Add(new TextBlock
        {
            Text = s.Name,
            FontSize = 14.5,
            FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
            Foreground = (Brush)Application.Current.Resources["TextPrimaryBrush"],
        });

        if (s.Username is { Length: > 0 } un)
        {
            nameRow.Children.Add(Badge($"@{un}", "NavBgBrush", "TextSecondaryBrush"));
        }

        if (s.LastError is { Length: > 0 })
        {
            nameRow.Children.Add(Badge("更新失败", "BadBrush", null, white: true));
        }
        else if (s.Traffic is { Stale: true })
        {
            nameRow.Children.Add(Badge("流量数据过期", "SlowBrush", null, white: true));
        }

        left.Children.Add(nameRow);

        // 地址行：脱敏地址 + 机场地址链接
        var addrRow = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10 };

        addrRow.Children.Add(new TextBlock
        {
            Text = s.UrlHint,
            FontSize = 11.5,
            Foreground = (Brush)Application.Current.Resources["TextTertiaryBrush"],
            TextTrimming = TextTrimming.CharacterEllipsis,
            MaxWidth = 380,
        });

        if (s.Homepage is { Length: > 0 } home)
        {
            var link = new HyperlinkButton
            {
                Content = "机场地址",
                FontSize = 11.5,
                Padding = new Thickness(0),
                NavigateUri = TryUri(home),
            };
            addrRow.Children.Add(link);
        }

        left.Children.Add(addrRow);
        row.Children.Add(left);

        var toggle = new ToggleSwitch
        {
            IsOn = s.Enabled,
            OnContent = null,
            OffContent = null,
            MinWidth = 0,
        };
        toggle.Toggled += async (_, _) =>
        {
            try
            {
                await App.Daemon.CallAsync("subscription_update",
                    new { uid = s.Uid, enabled = toggle.IsOn });
            }
            catch (DaemonException e)
            {
                Log.Error($"切换订阅状态失败：{e.FriendlyMessage}");
            }
        };
        Grid.SetColumn(toggle, 1);
        row.Children.Add(toggle);

        return row;
    }

    /// <summary>月度流量进度条：已用 / 总量 + 百分比 + 到期信息。</summary>
    private UIElement BuildTrafficBar(SubscriptionTraffic t)
    {
        var wrap = new StackPanel { Spacing = 6, Margin = new Thickness(0, 12, 0, 0) };

        var used = t.Used ?? ((t.Upload ?? 0) + (t.Download ?? 0));
        var pct = t.UsedPercent ?? (t.HasTotal ? used * 100.0 / t.Total!.Value : 0);

        // 文案行
        var textRow = new Grid();
        textRow.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        textRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var usedText = t.Unlimited
            ? $"已用 {MainWindow.FormatBytes(used)} · 不限量"
            : $"已用 {MainWindow.FormatBytes(used)} / {MainWindow.FormatBytes(t.Total!.Value)}";

        textRow.Children.Add(new TextBlock
        {
            Text = usedText,
            FontSize = 12,
            Foreground = (Brush)Application.Current.Resources["TextSecondaryBrush"],
        });

        var right = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Spacing = 10,
        };

        if (!t.Unlimited)
        {
            right.Children.Add(new TextBlock
            {
                Text = $"{pct:0.#}%",
                FontSize = 12,
                FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
                Foreground = TrafficBrush(pct),
            });
        }

        if (t.DaysLeft is long dl)
        {
            right.Children.Add(new TextBlock
            {
                Text = dl <= 0 ? "已到期" : $"剩余 {dl} 天",
                FontSize = 12,
                FontWeight = dl <= 7
                    ? Microsoft.UI.Text.FontWeights.SemiBold
                    : Microsoft.UI.Text.FontWeights.Normal,
                Foreground = ExpireBrush(dl),
            });
        }
        else if (t.ExpireAt is long ea)
        {
            var dt = DateTimeOffset.FromUnixTimeSeconds(ea).LocalDateTime;
            right.Children.Add(new TextBlock
            {
                Text = dt.ToString("yyyy-MM-dd 到期"),
                FontSize = 12,
                Foreground = (Brush)Application.Current.Resources["TextSecondaryBrush"],
            });
        }

        Grid.SetColumn(right, 1);
        textRow.Children.Add(right);
        wrap.Children.Add(textRow);

        // 进度条
        if (!t.Unlimited)
        {
            var bar = new ProgressBar
            {
                Minimum = 0,
                Maximum = 100,
                Value = Math.Clamp(pct, 0, 100),
                Height = 4,
                Background = (Brush)Application.Current.Resources["DividerBrush"],
                Foreground = TrafficBrush(pct),
            };
            wrap.Children.Add(bar);
        }

        return wrap;
    }

    private UIElement BuildCardStats(Subscription s)
    {
        var stats = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Spacing = 20,
            Margin = new Thickness(0, 12, 0, 0),
        };

        stats.Children.Add(StatItem("节点", $"{s.NodeCount} 个"));
        stats.Children.Add(StatItem("间隔",
            s.UpdateIntervalMin >= 60
                ? $"{s.UpdateIntervalMin / 60} 小时"
                : $"{s.UpdateIntervalMin} 分钟"));

        if (s.LastUpdateAt is long lf)
        {
            var dt = DateTimeOffset.FromUnixTimeSeconds(lf).ToLocalTime();
            stats.Children.Add(StatItem("上次更新", dt.ToString("MM-dd HH:mm")));
        }
        else
        {
            stats.Children.Add(StatItem("上次更新", "从未"));
        }

        return stats;
    }

    private UIElement BuildCardActions(Subscription s)
    {
        var actions = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Spacing = 8,
            Margin = new Thickness(0, 14, 0, 0),
        };

        var btnUpdate = new Button { Content = "立即更新", FontSize = 12.5 };
        btnUpdate.Click += async (_, _) =>
        {
            btnUpdate.IsEnabled = false;
            btnUpdate.Content = "更新中…";
            try
            {
                await App.Daemon.CallAsync("subscription_fetch",
                    new { uid = s.Uid, force = true }, timeoutSec: 120);
                await RefreshAsync();
            }
            catch (DaemonException e)
            {
                Log.Error($"更新订阅失败：{e.FriendlyMessage}");
                btnUpdate.Content = "更新失败";
                await Task.Delay(1500);
                btnUpdate.Content = "立即更新";
            }
            finally
            {
                btnUpdate.IsEnabled = true;
            }
        };
        actions.Children.Add(btnUpdate);

        var btnEdit = new Button { Content = "编辑", FontSize = 12.5 };
        btnEdit.Click += async (_, _) => await ShowEditDialogAsync(s);
        actions.Children.Add(btnEdit);

        var btnDelete = new Button { Content = "删除", FontSize = 12.5 };
        btnDelete.Click += async (_, _) => await ConfirmDeleteAsync(s);
        actions.Children.Add(btnDelete);

        return actions;
    }

    // ========================================================================
    //  小组件
    // ========================================================================

    private static Border Badge(string text, string bgKey, string? fgKey, bool white = false)
    {
        return new Border
        {
            Background = (Brush)Application.Current.Resources[bgKey],
            CornerRadius = new CornerRadius(4),
            Padding = new Thickness(6, 1, 6, 1),
            Child = new TextBlock
            {
                Text = text,
                FontSize = 10.5,
                Foreground = white
                    ? new SolidColorBrush(Microsoft.UI.Colors.White)
                    : (Brush)Application.Current.Resources[fgKey ?? "TextSecondaryBrush"],
            },
        };
    }

    private static UIElement StatItem(string label, string value)
    {
        var sp = new StackPanel { Spacing = 2 };
        sp.Children.Add(new TextBlock
        {
            Text = label,
            FontSize = 11,
            Foreground = (Brush)Application.Current.Resources["TextTertiaryBrush"],
        });
        sp.Children.Add(new TextBlock
        {
            Text = value,
            FontSize = 13,
            Foreground = (Brush)Application.Current.Resources["TextPrimaryBrush"],
        });
        return sp;
    }

    /// <summary>用量越低越绿，超过 80% 转橙，超过 95% 转红。</summary>
    private static Brush TrafficBrush(double pct) => (Brush)Application.Current.Resources[
        pct >= 95 ? "BadBrush" : pct >= 80 ? "SlowBrush" : "BrandBrush"];

    /// <summary>到期提醒：≤3 天红色，≤7 天橙色，其余次要色。</summary>
    private static Brush ExpireBrush(long daysLeft) => (Brush)Application.Current.Resources[
        daysLeft <= 3 ? "BadBrush" : daysLeft <= 7 ? "SlowBrush" : "TextSecondaryBrush"];

    private static Uri? TryUri(string s)
    {
        if (Uri.TryCreate(s, UriKind.Absolute, out var u) &&
            (u.Scheme == Uri.UriSchemeHttp || u.Scheme == Uri.UriSchemeHttps))
        {
            return u;
        }
        return null;
    }

    // ========================================================================
    //  对话框
    // ========================================================================

    /// <summary>添加对话框，含可展开的「更多信息」（用户名/密码/机场地址/备注）。</summary>
    private async Task ShowAddDialogAsync()
    {
        var nameBox = new TextBox { PlaceholderText = "订阅名称（可留空自动生成）" };
        var urlBox = new TextBox { PlaceholderText = "https://example.com/sub?token=..." };

        var userBox = new TextBox { PlaceholderText = "机场登录用户名（可选）" };
        var passBox = new PasswordBox { PlaceholderText = "机场登录密码（可选，仅本地加密保存）" };
        var homeBox = new TextBox { PlaceholderText = "https://example.com（可选）" };
        var remarkBox = new TextBox
        {
            PlaceholderText = "备注（可选）",
            AcceptsReturn = true,
            TextWrapping = TextWrapping.Wrap,
            Height = 60,
        };

        var morePanel = new StackPanel { Spacing = 10, Visibility = Visibility.Collapsed };
        morePanel.Children.Add(SectionLabel("机场登录信息"));
        morePanel.Children.Add(userBox);
        morePanel.Children.Add(passBox);
        morePanel.Children.Add(homeBox);
        morePanel.Children.Add(SectionLabel("备注"));
        morePanel.Children.Add(remarkBox);

        var moreToggle = new HyperlinkButton
        {
            Content = "更多信息（可选）▾",
            FontSize = 12,
            Padding = new Thickness(0),
            HorizontalAlignment = HorizontalAlignment.Left,
        };
        moreToggle.Click += (_, _) =>
        {
            var show = morePanel.Visibility == Visibility.Collapsed;
            morePanel.Visibility = show ? Visibility.Visible : Visibility.Collapsed;
            moreToggle.Content = show ? "收起 ▴" : "更多信息（可选）▾";
        };

        var panel = new StackPanel { Spacing = 12, Width = 440 };
        panel.Children.Add(new TextBlock
        {
            Text = "订阅链接通常以 http(s) 开头，包含你的个人 token，请勿分享给他人。",
            Style = (Style)Application.Current.Resources["HintTextStyle"],
        });
        panel.Children.Add(nameBox);
        panel.Children.Add(urlBox);
        panel.Children.Add(moreToggle);
        panel.Children.Add(morePanel);

        var dlg = new ContentDialog
        {
            XamlRoot = _host.XamlRoot,
            Title = "添加订阅",
            Content = new ScrollViewer { Content = panel, MaxHeight = 380 },
            PrimaryButtonText = "添加",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Primary,
        };

        if (await dlg.ShowAsync() != ContentDialogResult.Primary) return;

        var url = urlBox.Text.Trim();
        if (string.IsNullOrWhiteSpace(url))
        {
            await ShowInfoAsync("请填写订阅链接");
            return;
        }

        var name = nameBox.Text.Trim();
        if (string.IsNullOrWhiteSpace(name)) name = "订阅";

        // 有附加信息就走 add_ex；后端没实现时自动回退到 subscription_add
        var hasExtra = userBox.Text.Trim().Length > 0
                       || passBox.Password.Length > 0
                       || homeBox.Text.Trim().Length > 0
                       || remarkBox.Text.Trim().Length > 0;

        try
        {
            if (hasExtra)
            {
                try
                {
                    await App.Daemon.CallAsync<Subscription>("subscription_add_ex", new
                    {
                        name,
                        url,
                        username = NullIfEmpty(userBox.Text),
                        password = NullIfEmpty(passBox.Password),
                        homepage = NullIfEmpty(homeBox.Text),
                        remark = NullIfEmpty(remarkBox.Text),
                    }, timeoutSec: 60);
                }
                catch (DaemonException ex) when (ex.IsUnknownCommand)
                {
                    // 后端还没实现扩展命令：先按基础方式添加，再提示补填
                    await App.Daemon.CallAsync<Subscription>("subscription_add",
                        new { name, url }, timeoutSec: 60);
                    await ShowInfoAsync("当前内核服务尚不支持附加信息，已先添加订阅。请稍后在「编辑」里补填。");
                }
            }
            else
            {
                await App.Daemon.CallAsync<Subscription>("subscription_add",
                    new { name, url }, timeoutSec: 60);
            }
            await RefreshAsync();
        }
        catch (DaemonException e)
        {
            Log.Error($"添加订阅失败 code={e.Code}", e);
            await ShowInfoAsync(e.FriendlyMessage);
        }
    }

    /// <summary>编辑对话框：改名称/启用/间隔 + 全部管理信息。</summary>
    private async Task ShowEditDialogAsync(Subscription s)
    {
        var nameBox = new TextBox { Text = s.Name, Header = "名称" };

        var enabledToggle = new ToggleSwitch
        {
            IsOn = s.Enabled,
            Header = "启用",
            OnContent = "启用",
            OffContent = "停用",
        };

        var intervalBox = new NumberBox
        {
            Value = s.UpdateIntervalMin,
            Minimum = 0,
            Maximum = 10080,
            SmallChange = 60,
            LargeChange = 360,
            SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Inline,
            Header = "自动更新间隔（分钟，0 表示不自动更新）",
        };

        var userBox = new TextBox { Text = s.Username ?? "", Header = "机场登录用户名" };
        var passBox = new PasswordBox
        {
            PlaceholderText = s.HasPassword ? "已设置，留空则不修改" : "未设置",
            Header = "机场登录密码",
        };
        var homeBox = new TextBox { Text = s.Homepage ?? "", Header = "机场地址" };
        var remarkBox = new TextBox
        {
            Text = s.Remark ?? "",
            Header = "备注",
            AcceptsReturn = true,
            TextWrapping = TextWrapping.Wrap,
            Height = 64,
        };

        var panel = new StackPanel { Spacing = 12, Width = 440 };
        panel.Children.Add(nameBox);
        panel.Children.Add(enabledToggle);
        panel.Children.Add(intervalBox);
        panel.Children.Add(new TextBlock
        {
            Text = "机场登录信息",
            Style = (Style)Application.Current.Resources["SectionTitleStyle"],
            Margin = new Thickness(0, 6, 0, 0),
        });
        panel.Children.Add(userBox);
        panel.Children.Add(passBox);
        panel.Children.Add(homeBox);
        panel.Children.Add(remarkBox);
        panel.Children.Add(new TextBlock
        {
            Text = $"地址：{s.UrlHint}（订阅链接不可修改，如需更换请删除后重新添加）",
            Style = (Style)Application.Current.Resources["HintTextStyle"],
        });

        var dlg = new ContentDialog
        {
            XamlRoot = _host.XamlRoot,
            Title = "编辑订阅",
            // 窗口默认 660 高，ContentDialog 还要留出标题栏与按钮区，
            // 内容区超过 380 就会把对话框顶出屏幕外（标题和「名称」看不见）。
            Content = new ScrollViewer { Content = panel, MaxHeight = 380 },
            PrimaryButtonText = "保存",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Primary,
        };

        if (await dlg.ShowAsync() != ContentDialogResult.Primary) return;

        var name = nameBox.Text.Trim();
        if (string.IsNullOrWhiteSpace(name)) name = s.Name;

        var interval = (int)Math.Round(double.IsNaN(intervalBox.Value) ? s.UpdateIntervalMin : intervalBox.Value);

        try
        {
            // 1. 基础字段：name / enabled / intervalMin（这一步后端一直支持）
            await App.Daemon.CallAsync("subscription_update", new
            {
                uid = s.Uid,
                name,
                enabled = enabledToggle.IsOn,
                intervalMin = interval,
            });

            // 2. 管理信息：走 subscription_update_meta，未实现则提示
            var metaChanged = (userBox.Text.Trim() != (s.Username ?? ""))
                              || passBox.Password.Length > 0
                              || (homeBox.Text.Trim() != (s.Homepage ?? ""))
                              || (remarkBox.Text.Trim() != (s.Remark ?? ""));

            if (metaChanged)
            {
                try
                {
                    await App.Daemon.CallAsync("subscription_update_meta", new
                    {
                        uid = s.Uid,
                        username = NullIfEmpty(userBox.Text),
                        password = NullIfEmpty(passBox.Password),
                        homepage = NullIfEmpty(homeBox.Text),
                        remark = NullIfEmpty(remarkBox.Text),
                    });
                }
                catch (DaemonException ex) when (ex.IsUnknownCommand)
                {
                    await ShowInfoAsync("当前内核服务尚不支持保存机场登录信息，名称与启用状态已保存。");
                }
            }

            await RefreshAsync();
        }
        catch (DaemonException e)
        {
            Log.Error($"编辑订阅失败 code={e.Code}", e);
            await ShowInfoAsync(e.FriendlyMessage);
        }
    }

    private async Task ConfirmDeleteAsync(Subscription s)
    {
        var dlg = new ContentDialog
        {
            XamlRoot = _host.XamlRoot,
            Title = $"删除「{s.Name}」？",
            Content = "该订阅及其已下载的节点数据会被移除，此操作不可撤销。",
            PrimaryButtonText = "删除",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Close,
        };

        if (await dlg.ShowAsync() != ContentDialogResult.Primary) return;

        try
        {
            await App.Daemon.CallAsync("subscription_remove", new { uid = s.Uid });
            await RefreshAsync();
        }
        catch (DaemonException e)
        {
            await ShowInfoAsync(e.FriendlyMessage);
        }
    }

    private async Task ShowInfoAsync(string msg)
    {
        if (_host.XamlRoot is null) return;
        var dlg = new ContentDialog
        {
            XamlRoot = _host.XamlRoot,
            Title = "提示",
            Content = msg,
            CloseButtonText = "知道了",
        };
        try { await dlg.ShowAsync(); } catch { }
    }

    private static TextBlock SectionLabel(string text) => new()
    {
        Text = text,
        Style = (Style)Application.Current.Resources["SectionTitleStyle"],
    };

    private static string? NullIfEmpty(string? s)
    {
        var v = s?.Trim();
        return string.IsNullOrWhiteSpace(v) ? null : v;
    }

    private static void ShowError(StackPanel host, string msg)
    {
        host.Children.Clear();
        host.Children.Add(new TextBlock
        {
            Text = msg,
            Foreground = (Brush)Application.Current.Resources["BadBrush"],
        });
    }
}
