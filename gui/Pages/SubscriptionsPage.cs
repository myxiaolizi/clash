using ClashZhengl.Core;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;

namespace ClashZhengl.Pages;

/// <summary>
/// 订阅页：列表 + 添加/更新/删除/应用配置。
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

            // 顶部标题 + 操作按钮
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
            try
            {
                await App.Daemon.CallAsync("subscription_fetch_all", timeoutSec: 120);
                await RefreshAsync();
            }
            catch (DaemonException e)
            {
                Log.Error($"更新全部订阅失败：{e.FriendlyMessage}");
            }
            finally
            {
                btnRefresh.IsEnabled = true;
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

    private UIElement BuildCard(Subscription s)
    {
        var card = new Border
        {
            Style = (Style)Application.Current.Resources["CardStyle"],
        };

        var root = new Grid();
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });

        // ---- 第一行：名称 + 开关 ----
        var row1 = new Grid();
        row1.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row1.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var nameSp = new StackPanel { Spacing = 3 };
        var nameRow = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        nameRow.Children.Add(new TextBlock
        {
            Text = s.Name,
            FontSize = 14.5,
            FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
            Foreground = (Brush)Application.Current.Resources["TextPrimaryBrush"],
        });

        if (s.LastError is { Length: > 0 })
        {
            nameRow.Children.Add(new Border
            {
                Background = (Brush)Application.Current.Resources["BadBrush"],
                CornerRadius = new CornerRadius(4),
                Padding = new Thickness(6, 1, 6, 1),
                Child = new TextBlock
                {
                    Text = "更新失败",
                    FontSize = 10.5,
                    Foreground = new SolidColorBrush(Microsoft.UI.Colors.White),
                },
            });
        }
        nameSp.Children.Add(nameRow);

        nameSp.Children.Add(new TextBlock
        {
            Text = s.UrlMasked,
            FontSize = 11.5,
            Foreground = (Brush)Application.Current.Resources["TextTertiaryBrush"],
            TextTrimming = TextTrimming.CharacterEllipsis,
        });

        row1.Children.Add(nameSp);

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
        row1.Children.Add(toggle);

        root.Children.Add(row1);

        // ---- 第二行：统计信息 ----
        var stats = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Spacing = 20,
            Margin = new Thickness(0, 12, 0, 0),
        };

        stats.Children.Add(StatItem("节点", s.NodeCount is int n ? $"{n} 个" : "—"));
        stats.Children.Add(StatItem("间隔",
            s.IntervalMin >= 60 ? $"{s.IntervalMin / 60} 小时" : $"{s.IntervalMin} 分钟"));

        if (s.LastFetchAt is long lf)
        {
            var dt = DateTimeOffset.FromUnixTimeSeconds(lf).ToLocalTime();
            stats.Children.Add(StatItem("上次更新", dt.ToString("MM-dd HH:mm")));
        }

        if (s.Userinfo is { Total: > 0 } ui)
        {
            var used = (ui.Upload ?? 0) + (ui.Download ?? 0);
            var pct = ui.Total is > 0 ? used * 100.0 / ui.Total.Value : 0;
            stats.Children.Add(StatItem("已用流量",
                $"{MainWindow.FormatBytes(used)} / {MainWindow.FormatBytes(ui.Total.Value)} ({pct:0}%)"));
        }

        Grid.SetRow(stats, 1);
        root.Children.Add(stats);

        // ---- 第三行：操作 ----
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

        var btnRename = new Button { Content = "重命名", FontSize = 12.5 };
        btnRename.Click += async (_, _) => await ShowRenameDialogAsync(s);
        actions.Children.Add(btnRename);

        var btnDelete = new Button { Content = "删除", FontSize = 12.5 };
        btnDelete.Click += async (_, _) => await ConfirmDeleteAsync(s);
        actions.Children.Add(btnDelete);

        Grid.SetRow(actions, 2);
        root.Children.Add(actions);

        card.Child = root;
        return card;
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

    // ========================================================================
    //  对话框
    // ========================================================================

    private async Task ShowAddDialogAsync()
    {
        var nameBox = new TextBox { PlaceholderText = "订阅名称（可留空自动生成）" };
        var urlBox = new TextBox { PlaceholderText = "https://example.com/sub?token=..." };

        var panel = new StackPanel { Spacing = 12, Width = 420 };
        panel.Children.Add(new TextBlock
        {
            Text = "订阅链接通常以 http(s) 开头，包含你的个人 token，请勿分享给他人。",
            Style = (Style)Application.Current.Resources["HintTextStyle"],
        });
        panel.Children.Add(nameBox);
        panel.Children.Add(urlBox);

        var dlg = new ContentDialog
        {
            XamlRoot = _host.XamlRoot,
            Title = "添加订阅",
            Content = panel,
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

        try
        {
            await App.Daemon.CallAsync<Subscription>("subscription_add",
                new { name, url }, timeoutSec: 60);
            await RefreshAsync();
        }
        catch (DaemonException e)
        {
            Log.Error($"添加订阅失败 code={e.Code}", e);
            await ShowInfoAsync(e.FriendlyMessage);
        }
    }

    private async Task ShowRenameDialogAsync(Subscription s)
    {
        var box = new TextBox { Text = s.Name };

        var dlg = new ContentDialog
        {
            XamlRoot = _host.XamlRoot,
            Title = "重命名订阅",
            Content = box,
            PrimaryButtonText = "保存",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Primary,
        };

        if (await dlg.ShowAsync() != ContentDialogResult.Primary) return;

        var name = box.Text.Trim();
        if (string.IsNullOrWhiteSpace(name)) return;

        try
        {
            await App.Daemon.CallAsync("subscription_update",
                new { uid = s.Uid, name });
            await RefreshAsync();
        }
        catch (DaemonException e)
        {
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
        var dlg = new ContentDialog
        {
            XamlRoot = _host.XamlRoot,
            Title = "提示",
            Content = msg,
            CloseButtonText = "知道了",
        };
        try { await dlg.ShowAsync(); } catch { }
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
