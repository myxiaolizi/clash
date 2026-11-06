using ClashZhengl.Core;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;

namespace ClashZhengl.Pages;

/// <summary>
/// 规则页：自定义分流规则的增删改查与排序。
///
/// 后端命令：rule_list / rule_add / rule_update / rule_remove。
/// 0.1.1 后端并行开发中，命令未实现时整页降级成「功能开发中」，
/// 不显示红色报错，也不影响其它页面。
/// </summary>
public sealed class RulesPage
{
    private readonly StackPanel _host;

    private List<Rule> _rules = new();
    private bool _available = true;
    private bool _backendMissing;
    private string? _unavailableReason;

    public RulesPage(StackPanel host) => _host = host;

    public Task InitializeAsync() => RefreshAsync();

    public async Task RefreshAsync()
    {
        try
        {
            var list = await App.Daemon.CallAsync<List<Rule>>("rule_list");
            _rules = (list ?? new List<Rule>())
                .OrderBy(r => r.SortOrder)
                .ToList();
            _available = true;
            _backendMissing = false;
            _unavailableReason = null;
        }
        catch (DaemonException e) when (e.IsUnknownCommand)
        {
            // 后端还没实现 rule_*：这是「功能未上线」，安心降级
            _available = false;
            _backendMissing = true;
            _unavailableReason = "规则管理功能正在开发中，下个版本可用";
            _rules = new List<Rule>();
            Log.Warn("rule_list 未实现，规则页降级");
        }
        catch (DaemonException e)
        {
            // 连不上 daemon / 其它错误：是「暂时连不上」，不能说成功能没做
            // ——这两种情况用户该做的事完全不同（等更新 vs 检查内核服务）。
            Log.Error($"加载规则失败 code={e.Code}", e);
            _available = false;
            _backendMissing = false;
            _unavailableReason = e.FriendlyMessage;
            _rules = new List<Rule>();
        }

        Render();
    }

    // ========================================================================
    //  渲染
    // ========================================================================

    private void Render()
    {
        _host.Children.Clear();

        _host.Children.Add(BuildHeader());

        if (!_available)
        {
            _host.Children.Add(BuildUnavailableCard());
            return;
        }

        _host.Children.Add(BuildIntroCard());

        if (_rules.Count == 0)
        {
            _host.Children.Add(BuildEmptyCard());
            return;
        }

        for (var i = 0; i < _rules.Count; i++)
        {
            _host.Children.Add(BuildRuleCard(_rules[i], i));
        }
    }

    private UIElement BuildHeader()
    {
        var grid = new Grid();
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var left = new StackPanel();
        left.Children.Add(new TextBlock
        {
            Text = "规则",
            Style = (Style)Application.Current.Resources["PageTitleStyle"],
        });
        left.Children.Add(new TextBlock
        {
            Text = _available
                ? (_rules.Count == 0
                    ? "自定义分流规则，按从上到下的顺序匹配"
                    : $"共 {_rules.Count} 条规则，已启用 {_rules.Count(r => r.Enabled)} 条")
                : "自定义分流规则",
            Style = (Style)Application.Current.Resources["PageSubtitleStyle"],
        });
        grid.Children.Add(left);

        if (_available)
        {
            var btnAdd = new Button
            {
                Content = "添加规则",
                Style = (Style)Application.Current.Resources["PrimaryButtonStyle"],
                VerticalAlignment = VerticalAlignment.Top,
            };
            btnAdd.Click += async (_, _) => await ShowAddDialogAsync();
            Grid.SetColumn(btnAdd, 1);
            grid.Children.Add(btnAdd);
        }

        return grid;
    }

    private UIElement BuildUnavailableCard()
    {
        var card = new Border
        {
            Style = (Style)Application.Current.Resources["CardStyle"],
            Padding = new Thickness(28),
        };

        var sp = new StackPanel
        {
            HorizontalAlignment = HorizontalAlignment.Center,
            Spacing = 10,
        };

        sp.Children.Add(new FontIcon
        {
            Glyph = "\uE9CE",
            FontSize = 30,
            HorizontalAlignment = HorizontalAlignment.Center,
            Foreground = (Brush)Application.Current.Resources["TextTertiaryBrush"],
        });

        sp.Children.Add(new TextBlock
        {
            Text = _unavailableReason ?? "功能开发中",
            FontSize = 14,
            HorizontalAlignment = HorizontalAlignment.Center,
            TextAlignment = TextAlignment.Center,
            TextWrapping = TextWrapping.Wrap,
            Foreground = (Brush)Application.Current.Resources["TextSecondaryBrush"],
        });

        sp.Children.Add(new TextBlock
        {
            Text = _backendMissing
                ? "这个版本先支持首页、订阅与设置；规则编辑会在后端就绪后开放。"
                : "请确认内核服务正在运行，或稍后重试。",
            Style = (Style)Application.Current.Resources["HintTextStyle"],
            HorizontalAlignment = HorizontalAlignment.Center,
            TextAlignment = TextAlignment.Center,
        });

        if (!_backendMissing)
        {
            var retry = new Button
            {
                Content = "重试",
                FontSize = 12.5,
                HorizontalAlignment = HorizontalAlignment.Center,
                Margin = new Thickness(0, 6, 0, 0),
            };
            retry.Click += async (_, _) => await RefreshAsync();
            sp.Children.Add(retry);
        }

        card.Child = sp;
        return card;
    }

    private UIElement BuildIntroCard()
    {
        var card = new Border
        {
            Style = (Style)Application.Current.Resources["CardStyle"],
            Padding = new Thickness(14, 12, 14, 12),
        };

        var sp = new StackPanel { Spacing = 4 };
        sp.Children.Add(new TextBlock
        {
            Text = "规则从上到下依次匹配，命中即停止；都没命中时走主策略组。",
            FontSize = 12,
            TextWrapping = TextWrapping.Wrap,
            Foreground = (Brush)Application.Current.Resources["TextSecondaryBrush"],
        });
        sp.Children.Add(new TextBlock
        {
            Text = "改动会在下次应用配置时写入内核。",
            Style = (Style)Application.Current.Resources["HintTextStyle"],
        });

        card.Child = sp;
        return card;
    }

    private UIElement BuildEmptyCard()
    {
        var card = new Border
        {
            Style = (Style)Application.Current.Resources["CardStyle"],
            Padding = new Thickness(28),
        };

        var sp = new StackPanel
        {
            HorizontalAlignment = HorizontalAlignment.Center,
            Spacing = 10,
        };

        sp.Children.Add(new FontIcon
        {
            Glyph = "\uE71C",
            FontSize = 28,
            HorizontalAlignment = HorizontalAlignment.Center,
            Foreground = (Brush)Application.Current.Resources["TextTertiaryBrush"],
        });

        sp.Children.Add(new TextBlock
        {
            Text = "还没有自定义规则",
            FontSize = 14,
            HorizontalAlignment = HorizontalAlignment.Center,
            Foreground = (Brush)Application.Current.Resources["TextPrimaryBrush"],
        });

        sp.Children.Add(new TextBlock
        {
            Text = "点击右上角「添加规则」，可以把指定域名设为直连或阻止",
            Style = (Style)Application.Current.Resources["HintTextStyle"],
            HorizontalAlignment = HorizontalAlignment.Center,
            TextAlignment = TextAlignment.Center,
        });

        card.Child = sp;
        return card;
    }

    private UIElement BuildRuleCard(Rule r, int index)
    {
        var card = new Border { Style = (Style)Application.Current.Resources["CardStyle"] };

        var root = new Grid();
        root.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        root.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        root.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        // 序号
        root.Children.Add(new TextBlock
        {
            Text = $"{index + 1}",
            FontSize = 12,
            Width = 26,
            VerticalAlignment = VerticalAlignment.Center,
            Foreground = (Brush)Application.Current.Resources["TextTertiaryBrush"],
        });

        // 主体
        var body = new StackPanel { Spacing = 5 };

        var titleRow = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        titleRow.Children.Add(new TextBlock
        {
            Text = r.Value,
            FontSize = 14,
            FontWeight = FontWeights.SemiBold,
            TextTrimming = TextTrimming.CharacterEllipsis,
            MaxWidth = 420,
            Foreground = (Brush)Application.Current.Resources["TextPrimaryBrush"],
        });

        titleRow.Children.Add(Chip(r.KindText, "NavBgBrush", "TextSecondaryBrush"));
        titleRow.Children.Add(PolicyChip(r.Policy));
        if (!r.Enabled) titleRow.Children.Add(Chip("已停用", "NavBgBrush", "TextTertiaryBrush"));

        body.Children.Add(titleRow);

        // 后端生成的规则行原文，等宽字体，帮用户确认最终写进内核的是什么
        if (r.HasLine)
        {
            body.Children.Add(new TextBlock
            {
                Text = r.Line,
                FontFamily = new FontFamily("Consolas, Cascadia Mono, monospace"),
                FontSize = 11,
                TextTrimming = TextTrimming.CharacterEllipsis,
                MaxWidth = 420,
                Foreground = (Brush)Application.Current.Resources["TextTertiaryBrush"],
            });
        }
        Grid.SetColumn(body, 1);
        root.Children.Add(body);

        // 右侧操作
        var actions = new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Spacing = 6,
            VerticalAlignment = VerticalAlignment.Center,
        };

        var toggle = new ToggleSwitch
        {
            IsOn = r.Enabled,
            OnContent = null,
            OffContent = null,
            MinWidth = 0,
            Margin = new Thickness(0, 0, 6, 0),
        };
        AutomationProperties.SetName(toggle, $"启用规则 {r.Value}");
        toggle.Toggled += async (_, _) => await UpdateEnabledAsync(r, toggle);
        actions.Children.Add(toggle);

        actions.Children.Add(IconButton("\uE70E", "上移", index > 0, async () => await MoveAsync(index, -1)));
        actions.Children.Add(IconButton("\uE70D", "下移", index < _rules.Count - 1, async () => await MoveAsync(index, 1)));

        var del = new Button { Content = "删除", FontSize = 12, Padding = new Thickness(10, 4, 10, 4) };
        AutomationProperties.SetName(del, $"删除规则 {r.Value}");
        del.Click += async (_, _) => await ConfirmDeleteAsync(r);
        actions.Children.Add(del);

        Grid.SetColumn(actions, 2);
        root.Children.Add(actions);

        card.Child = root;
        return card;
    }

    private static Button IconButton(string glyph, string tip, bool enabled, Func<Task> onClick)
    {
        var btn = new Button
        {
            Content = new FontIcon { Glyph = glyph, FontSize = 11 },
            IsEnabled = enabled,
            Padding = new Thickness(8, 4, 8, 4),
            FontSize = 12,
        };

        // 图标按钮没有文字，读屏软件只能读出「按钮」。
        // 必须显式给一个可访问名称和提示，否则视障用户完全不知道这是干什么的。
        AutomationProperties.SetName(btn, tip);
        ToolTipService.SetToolTip(btn, tip);

        btn.Click += async (_, _) => await onClick();
        return btn;
    }

    private static Border Chip(string text, string bgKey, string fgKey)
    {
        return new Border
        {
            Background = (Brush)Application.Current.Resources[bgKey],
            CornerRadius = new CornerRadius(4),
            Padding = new Thickness(6, 1, 6, 1),
            VerticalAlignment = VerticalAlignment.Center,
            Child = new TextBlock
            {
                Text = text,
                FontSize = 10.5,
                Foreground = (Brush)Application.Current.Resources[fgKey],
            },
        };
    }

    private static Border PolicyChip(string policy)
    {
        var (bg, fg) = policy switch
        {
            "direct" => ("GoodBrush", (string?)null),
            "block" => ("BadBrush", (string?)null),
            _ => ("BrandBrush", (string?)null),
        };

        return new Border
        {
            Background = (Brush)Application.Current.Resources[bg],
            CornerRadius = new CornerRadius(4),
            Padding = new Thickness(6, 1, 6, 1),
            VerticalAlignment = VerticalAlignment.Center,
            Child = new TextBlock
            {
                Text = Rule.PolicyLabel(policy),
                FontSize = 10.5,
                Foreground = new SolidColorBrush(Microsoft.UI.Colors.White),
            },
        };
    }

    // ========================================================================
    //  操作
    // ========================================================================

    private async Task UpdateEnabledAsync(Rule r, ToggleSwitch toggle)
    {
        try
        {
            await App.Daemon.CallAsync("rule_update", new
            {
                id = r.Id,
                enabled = toggle.IsOn,
            });
            r.Enabled = toggle.IsOn;
        }
        catch (DaemonException e)
        {
            Log.Error($"切换规则状态失败：{e.FriendlyMessage}");
            toggle.IsOn = !toggle.IsOn;
        }
    }

    /// <summary>
    /// 上下移动。
    ///
    /// 后端提供 <c>rule_reorder {ids:[...]}</c>，一次提交整份顺序，
    /// 比逐条 rule_update 改 sortOrder 更稳：中途失败不会留下半个顺序，
    /// 也不会出现两条规则 sortOrder 相同的情况。
    /// </summary>
    private async Task MoveAsync(int index, int delta)
    {
        var target = index + delta;
        if (target < 0 || target >= _rules.Count) return;

        // 先本地换位，界面上立刻有反馈
        var snapshot = _rules.ToList();
        (_rules[index], _rules[target]) = (_rules[target], _rules[index]);

        try
        {
            var ids = _rules.Select(r => r.Id).ToArray();
            var reordered = await App.Daemon.CallAsync<List<Rule>>("rule_reorder", new { ids });
            if (reordered is { Count: > 0 })
                _rules = reordered.OrderBy(r => r.SortOrder).ToList();

            Render();
        }
        catch (DaemonException e)
        {
            Log.Error($"调整规则顺序失败：{e.FriendlyMessage}");
            _rules = snapshot;   // 回滚
            Render();
            await ShowInfoAsync(e.FriendlyMessage);
        }
    }

    private async Task ConfirmDeleteAsync(Rule r)
    {
        if (_host.XamlRoot is null) return;

        var dlg = new ContentDialog
        {
            XamlRoot = _host.XamlRoot,
            Title = "删除规则？",
            Content = $"将删除规则「{r.Value}」，此操作不可撤销。",
            PrimaryButtonText = "删除",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Close,
        };

        if (await dlg.ShowAsync() != ContentDialogResult.Primary) return;

        try
        {
            await App.Daemon.CallAsync("rule_remove", new { id = r.Id });
            await RefreshAsync();
        }
        catch (DaemonException e)
        {
            await ShowInfoAsync(e.FriendlyMessage);
        }
    }

    // ========================================================================
    //  添加对话框
    // ========================================================================

    private async Task ShowAddDialogAsync()
    {
        if (_host.XamlRoot is null) return;

        // 类型下拉，显示中文，值用后端枚举字符串
        var kindCombo = new ComboBox
        {
            Header = "规则类型",
            HorizontalAlignment = HorizontalAlignment.Stretch,
            SelectedIndex = 1,
            ItemsSource = new[]
            {
                "域名（精确匹配）",
                "域名后缀（含子域名）",
                "域名关键词（包含即命中）",
                "IP / IP 段",
                "进程名",
            },
        };

        var valueBox = new TextBox
        {
            Header = "匹配内容",
            PlaceholderText = "例如 example.com 或 192.168.0.0/16",
        };

        var policyCombo = new ComboBox
        {
            Header = "处理方式",
            HorizontalAlignment = HorizontalAlignment.Stretch,
            SelectedIndex = 0,
            ItemsSource = new[]
            {
                "代理（走主策略组）",
                "直连（不走代理）",
                "阻止（直接拒绝）",
            },
        };

        var hint = new TextBlock
        {
            Style = (Style)Application.Current.Resources["HintTextStyle"],
        };

        void UpdateHint()
        {
            var raw = valueBox.Text.Trim();
            if (raw.Length == 0)
            {
                hint.Text = "填写要匹配的内容，保存后立即生效。";
                hint.Foreground = (Brush)Application.Current.Resources["TextTertiaryBrush"];
                return;
            }

            // 后端要求 value 里不能有逗号、空白、换行——这些字符会把
            // 生成的 mihomo 规则行切错位，所以这里提前拦下并说明原因。
            var bad = ValidateValue(raw, kindCombo.SelectedIndex);
            if (bad is not null)
            {
                hint.Text = bad;
                hint.Foreground = (Brush)Application.Current.Resources["BadBrush"];
                return;
            }

            hint.Text = $"预览：{PreviewLine(kindCombo.SelectedIndex, raw, policyCombo.SelectedIndex)}";
            hint.Foreground = (Brush)Application.Current.Resources["TextTertiaryBrush"];
        }

        kindCombo.SelectionChanged += (_, _) => UpdateHint();
        policyCombo.SelectionChanged += (_, _) => UpdateHint();
        valueBox.TextChanged += (_, _) => UpdateHint();
        UpdateHint();

        // Header 只是视觉标签，读屏软件识别不到，这里补可访问名称
        AutomationProperties.SetName(kindCombo, "规则类型");
        AutomationProperties.SetName(policyCombo, "处理方式");
        AutomationProperties.SetName(valueBox, "匹配内容");

        var panel = new StackPanel { Spacing = 12, Width = 420 };
        panel.Children.Add(kindCombo);
        panel.Children.Add(valueBox);
        panel.Children.Add(policyCombo);
        panel.Children.Add(hint);

        var dlg = new ContentDialog
        {
            XamlRoot = _host.XamlRoot,
            Title = "添加规则",
            Content = panel,
            PrimaryButtonText = "添加",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Primary,
        };

        if (await dlg.ShowAsync() != ContentDialogResult.Primary) return;

        var value = valueBox.Text.Trim();
        if (string.IsNullOrWhiteSpace(value))
        {
            await ShowInfoAsync("请填写要匹配的内容");
            return;
        }

        var problem = ValidateValue(value, kindCombo.SelectedIndex);
        if (problem is not null)
        {
            await ShowInfoAsync(problem);
            return;
        }

        var kind = kindCombo.SelectedIndex switch
        {
            0 => "domain",
            2 => "domain_keyword",
            3 => "ip",
            4 => "process",
            _ => "domain_suffix",
        };

        var policy = policyCombo.SelectedIndex switch
        {
            1 => "direct",
            2 => "block",
            _ => "proxy",
        };

        try
        {
            // rule_add 只接受 kind/value/policy/enabled，sortOrder 由后端追加到末尾
            await App.Daemon.CallAsync("rule_add", new
            {
                kind,
                value,
                policy,
                enabled = true,
            });
            await RefreshAsync();
        }
        catch (DaemonException e)
        {
            Log.Error($"添加规则失败：{e.FriendlyMessage}");
            await ShowInfoAsync(e.FriendlyMessage);
        }
    }

    /// <summary>
    /// 校验规则值，返回 null 表示合法，否则返回给用户看的原因。
    ///
    /// 后端会把规则拼成 <c>TYPE,value,POLICY</c> 这一行，
    /// 所以 value 里不能出现逗号、空白、换行，否则字段会被切错位。
    /// IP 类还要检查前缀长度（v4 ≤ 32 / v6 ≤ 128）。
    /// 在这里拦下比让后端报错更友好——用户能立刻看到哪里不对。
    /// </summary>
    private static string? ValidateValue(string value, int kindIndex)
    {
        if (value.Contains(',')) return "内容里不能包含逗号（会把规则字段切开）";
        if (value.Any(char.IsWhiteSpace)) return "内容里不能包含空格或换行";
        if (value.Contains('\n') || value.Contains('\r')) return "内容里不能包含换行";

        // IP / IP 段
        if (kindIndex == 3)
        {
            var slash = value.IndexOf('/');
            if (slash < 0) return null; // 单个 IP，交给后端判断

            var addr = value[..slash];
            var prefixText = value[(slash + 1)..];

            if (!int.TryParse(prefixText, out var prefix) || prefix < 0)
                return "IP 段的掩码位数必须是数字，例如 /24";

            var max = addr.Contains(':') ? 128 : 32;
            if (prefix > max)
                return $"IP 段掩码超出范围：{(max == 128 ? "IPv6" : "IPv4")} 最多 /{max}";

            return null;
        }

        // 域名类：不能带协议头或路径
        if (kindIndex is 0 or 1 or 2)
        {
            if (value.Contains("://")) return "只填域名，不要带 http:// 或 https://";
            if (value.Contains('/')) return "只填域名，不要带路径";
        }

        return null;
    }

    /// <summary>生成一条 mihomo 规则预览，让用户在保存前看清效果。</summary>
    private static string PreviewLine(int kindIndex, string value, int policyIndex)
    {
        var type = kindIndex switch
        {
            0 => "DOMAIN",
            2 => "DOMAIN-KEYWORD",
            3 => "IP-CIDR",
            4 => "PROCESS-NAME",
            _ => "DOMAIN-SUFFIX",
        };
        var target = policyIndex switch
        {
            1 => "DIRECT",
            2 => "REJECT",
            _ => "Proxy",
        };
        var extra = kindIndex == 3 ? ",no-resolve" : "";
        return $"{type},{value},{target}{extra}";
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
}
