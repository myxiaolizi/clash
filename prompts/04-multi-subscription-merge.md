# 04 · 多订阅去重合并与运行配置生成

> **本章用途**：把 N 个已解析的订阅 profile 合并成**一份**交给 mihomo 的 `runtime/config.yaml`：
> 去重与指纹、节点重命名、策略组生成规则（**含 scope 组必须为 `select` 的硬约束**）、
> `rules` 重映射与兜底、原子写、`mihomo -t` 预校验、热更新 vs 重启内核的判定。
>
> **投喂时机**：P2 阶段（与 `03-subscription-fetch-and-parse.md` 同一批），必须在 03 章之后读。
>
> **前置依赖**：
> - `01-master-prompt.md`（铁律 1/2/3/4/7、禁止行为）
> - `appendices/C-conventions.md` **v1.2**（C.3 路径、C.5 数据模型含 `ProbeResult.node_key`、C.8 错误码、C.12 安全）
> - `02-tech-stack-and-architecture.md`（§5 依赖、§4 线程、§3.3 `AppState`）
> - `03-subscription-fetch-and-parse.md`（`ParsedProfile`、原子写、错误码映射）
> - `appendices/A-mihomo-api-and-config.md`（字段表、`PUT /configs` 语义）
>
> **本章不得修改契约**；发现缺口输出 `CONTRACT-CHANGE-REQUEST:`。

---

## 1. 合并总览

```text
SubscriptionManager::build_runtime_config(now)
  │
  ├─ 1. 取 enabled == true 的订阅，按 sort_order 排序（停用的一律跳过）
  ├─ 2. 逐个读 profiles/<uid>.yaml → parse::validate_and_parse()
  │      任一订阅坏 → 跳过该订阅（记 WARN），不影响其它订阅
  ├─ 3. 节点去重：指纹 = sha256(type\0server\0port\0credential) 前 16 hex
  │      ├─ 同指纹 → 保留 sort_order 最小者的节点（先到先得）
  │      └─ 指纹不落明文日志（只打前 8 位）
  ├─ 4. 节点重名处理：跨订阅同名 → 加订阅前缀 "<subName>/<nodeName>"
  │      ├─ 仍冲突（同订阅内重名）→ 追加 " #2"、" #3"
  │      └─ 产出 node_key 映射表 fingerprint → node_key（内存态，供 07 章切换用）
  ├─ 5. 生成 proxy-groups
  │      ├─ R-1 主组 Proxy：type: select（**硬约束**）
  │      ├─ R-2 **只生成这一个组**（不生成每订阅子组，Lead v1.4 裁决）
  │      ├─ R-3 **禁止**生成同名 url-test / fallback / load-balance 组
  │      └─ R-4 组内成员顺序 = 合并后节点稳定顺序（按 sort_order + 文件顺序）
  ├─ 6. rules 合并 + 目标重映射 + 兜底 MATCH
  ├─ 7. 组装完整 config（§5 必填字段表）
  ├─ 8. 序列化 → 原子写 runtime/config.yaml（tmp → fsync → rename）
  ├─ 9. 校验：mihomo -d <runtime> -f <runtime>/config.yaml -t（§7）
  │      └─ 失败 → 保留旧 config、不改内核、返回 E_CORE_START_FAILED（中文可读）
  └─ 10. 应用：热更新 PUT /configs?force=false 或重启内核（§8 判定表）
```

> **幂等要求**：同一组输入（订阅内容 + 设置 + 端口 + secret）必须产出**逐字节相同**的 config。
> 因此成员顺序、组顺序、规则顺序都必须由显式排序键决定，**禁止**依赖 `HashMap` 迭代顺序。
> 这是单元测试可断言的基础（`build_twice_is_byte_identical`）。

---

## 2. 输入：合并上下文

```rust
// src-tauri/src/subscription/manager.rs
use std::collections::HashMap;

use crate::domain::model::{AppSettings, Subscription, SwitchScope};

/// 合并所需的全部输入。显式传入，不用全局单例 —— 便于单测构造。
pub struct MergeInput {
    /// 已启用订阅，已按 sort_order 升序
    pub subs: Vec<SubSource>,
    /// 内核已分配好的 TCP 端口（见 08 章 kernel/ports.rs）
    pub mixed_port: u16,
    /// external-controller 端口（127.0.0.1:此端口）
    pub controller_port: u16,
    /// 32 hex 随机 secret（来自 runtime/secret，见 §5.3）
    pub secret: String,
    pub settings: AppSettings,
}

pub struct SubSource {
    /// 订阅元信息（只读，用于命名与顺序）
    pub sub: Subscription,
    /// 该订阅原始 YAML 字节（profiles/<uid>.yaml 内容）
    pub yaml: Vec<u8>,
}

/// 合并产物：config 全文字节 + 可回查的映射表（不落盘，内存态）
pub struct MergedRuntime {
    pub yaml: Vec<u8>,
    /// fingerprint(16 hex) → 最终 node_key（07 章用它把测速结果映射回内核节点名）
    pub fingerprint_to_key: HashMap<String, String>,
    /// node_key → 所属订阅 uid（UI 分组展示用）
    pub key_to_uid: HashMap<String, String>,
    /// 丢弃的重复节点数（UI/日志提示）
    pub dropped_duplicates: u32,
    /// 被重命名的节点数
    pub renamed_nodes: u32,
    /// 被丢弃的 rules 条数（目标无法重映射）
    pub dropped_rules: u32,
}
```

| 输入来源 | 规则 |
| --- | --- |
| `Subscription.enabled == false` | **跳过**：不参与合并、不参与测速（C.6 `subscription_set_enabled` 语义） |
| `Subscription.last_update_status == Failed` 但本地 yaml 可用 | **照常参与**（离线时用户仍需要能上网；这是"失败不崩"的延伸） |
| 本地 yaml 缺失 | 跳过该订阅，WARN；若**全部**订阅都不可用 → `E_PARSE_EMPTY` |
| 排序 | `sort_order` 升序；相同则按 `uid` 字典序（保证确定性） |

---

## 3. 节点去重

### 3.1 指纹算法

| 项 | 规定 |
| --- | --- |
| 参与字段 | `type`、`server`、`port`、凭据（按类型取：`uuid` / `password` / `auth` / `private-key`） |
| **不**参与 | `name`、`udp`、`skip-cert-verify`、`alpn`、`sni`、`ws-opts` 等可选装饰项 |
| 归一化 | `server` 转小写并去首尾空白；`port` 按整数；`type` 转小写；凭据原样（大小写敏感） |
| 哈希 | `SHA-256(canonical_string)` → hex 小写 → **取前 16 位**作为指纹 |
| 日志 | **只允许**打印前 8 位（`fp=9a3c1d0e`）；**禁止**打印 canonical_string（含凭据） |
| 命中重复 | 保留 `sort_order` 最小订阅里的那个节点，其余丢弃并计入 `dropped_duplicates` |

> 为什么不用「名字」做去重键：同一节点在不同机场的命名完全不同（`🇭🇰香港01` vs `HK-01`），
> 按名字去重会把真正相同的节点留下 2 份，导致列表里出现两个一模一样的选择项，用户无法理解。

```rust
// src-tauri/src/subscription/manager.rs（节选）
use sha2::{Digest, Sha256};

/// 从一条 proxy 的 YAML mapping 中抽取去重指纹所需字段。
/// 返回 None 表示缺少必要字段 → 该节点不可用于去重判断（仍可保留，指纹退化按名处理）。
pub fn node_fingerprint(m: &serde_yaml_ng::Mapping) -> Option<String> {
    use serde_yaml_ng::{Value};

    let get_str = |k: &str| -> Option<String> {
        m.get(Value::String(k.into()))
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };

    let kind = get_str("type")?.to_ascii_lowercase();
    let server = get_str("server")?.to_ascii_lowercase();
    let port = m
        .get(Value::String("port".into()))
        .and_then(|v| v.as_i64())
        .filter(|p| (1..=65535).contains(p))?;

    // 凭据按类型取；缺失时用空串（不能让 None 直接导致指纹缺失，
    // 否则一堆"无密码节点"会被误判为同一指纹而互相吞掉）
    let credential = match kind.as_str() {
        "vmess" | "vless" => get_str("uuid").unwrap_or_default(),
        "trojan" | "hysteria2" | "hy2" | "tuic" => get_str("password").unwrap_or_default(),
        "ss" | "shadowsocks" => {
            // ss 可能是 password 也可能是 cipher:password 形式
            get_str("password")
                .or_else(|| {
                    get_str("cipher")
                        .and_then(|c| c.split_once(':').map(|(_, p)| p.to_string()))
                })
                .unwrap_or_default()
        }
        "socks5" | "http" => format!(
            "{}:{}",
            get_str("username").unwrap_or_default(),
            get_str("password").unwrap_or_default()
        ),
        _ => get_str("password").or_else(|| get_str("uuid")).unwrap_or_default(),
    };

    // 规范化字符串：字段之间用 \0 分隔，避免 "ab"+"c" 与 "a"+"bc" 碰撞
    let canonical = format!("{kind}\0{server}\0{port}\0{credential}");
    let digest = Sha256::digest(canonical.as_bytes());
    // 取前 8 字节 → 16 位小写 hex。手写格式化，**不引入 `hex` crate**（不在 02 章 §5.1 白名单）
    Some(digest.iter().take(8).map(|b| format!("{b:02x}")).collect())
}
```

> 日志侧再截前 8 位：`fp=9a3c1d0e`（即指纹的前一半）。目的只有一个——
> 让开发者能判断「这两条是同一个节点」，同时把「凭据是否泄露」的风险降到零。

### 3.2 去重结果的处理

| 场景 | 处理 | 日志 |
| --- | --- | --- |
| 同指纹，来自不同订阅 | 丢弃后来的 | `INFO [merge] dup dropped uid=… fp=9a3c1d0e sub=sub-b` |
| 同指纹，同订阅内重复 | 丢弃后来的（同订阅重复更可能是机场配置 bug） | `WARN` |
| 指纹无法计算（缺 server/port） | 不做跨订阅去重，仅按 `name` 在同订阅内去重 | `WARN node missing server/port` |
| 全部被去重掉 | `E_PARSE_EMPTY` | — |

---

## 4. 节点命名与 `node_key`

### 4.1 为什么 `node_key` 必须是最终写入 config 的 `name`

`ProbeResult.node_key`（C.5）注释写的是「形如 `HongKong-01` 或 `sub-a/HongKong-01`（冲突时加前缀）」。
内核 API 的路径是 `/proxies/{name}/delay`，`{name}` **就是** YAML 里的 `name` 字段。
所以：**`node_key` ≡ 合并后写入 `proxies[].name` 的字符串**，二者不能有第二套映射，否则测速会 404。

### 4.2 命名规则（按优先级）

| # | 规则 | 示例 |
| --- | --- | --- |
| N-1 | 默认保留机场原名（去首尾空白） | `HongKong-01` |
| N-2 | 跨订阅同名 → 前缀 `<订阅名>/` | `sub-a/HongKong-01` |
| N-3 | 加前缀后仍同名（同订阅内重名）→ 追加 ` #2`、` #3`… | `sub-a/HongKong-01 #2` |
| N-4 | 清洗控制字符（`\n` `\r` `\t`）→ 空格，防止破坏日志单行结构与 YAML | — |
| N-5 | 名字长度上限 96 字符（超出截断 + `…`+序号），避免 UI 与 API URL 过长 | — |
| N-6 | 名字为空 → `node-<fp前8位>` | `node-9a3c1d0e` |
| N-7 | **禁止**生成与保留名冲突的名字：`DIRECT`、`REJECT`、`REJECT-DROP`、`PASS`、`COMPATIBLE`、`GLOBAL`，以及组名 | 冲突时按 N-3 处理 |

> `/` 是允许出现在 mihomo 节点名里的（C.5 自己就用了 `sub-a/HongKong-01`）。
> 但取 URL 路径时需 URL-encode：`/proxies/sub-a%2FHongKong-01/delay`。
> `kernel/api.rs` 必须对节点名做 `percent_encode`，否则含 `/`、`+`、空格的名字会 404。

```rust
// src-tauri/src/subscription/manager.rs（节选）
/// 生成最终 node_key。纯函数：只依赖入参，可 100% 单测。
pub fn plan_node_names(names_by_fp: &[(String, String)]) -> Vec<(String, String)> {
    // 入参：(fingerprint, original_name)，顺序 = 合并顺序
    // 1) 统计原名出现次数
    let mut count: HashMap<&str, usize> = HashMap::new();
    for (_, n) in names_by_fp {
        *count.entry(n.as_str()).or_insert(0) += 1;
    }

    // 2) 同名者一律加前缀（只有在 >=2 个订阅时才可能冲突，去重后同名即跨订阅）
    let mut used: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::with_capacity(names_by_fp.len());
    for (fp, name) in names_by_fp {
        let clean = sanitize_name(name);
        let base = if count.get(clean.as_str()).copied().unwrap_or(0) > 1 {
            format!("{}/{}", prefix_of(fp), clean) // prefix_of 见下：订阅名，不含凭据
        } else {
            clean
        };
        // 3) 兜底唯一化
        let n = used.entry(base.clone()).or_insert(0);
        *n += 1;
        let final_name = if *n == 1 { base } else { format!("{base} #{}", *n) };
        out.push((fp.clone(), final_name));
    }
    out
}

fn sanitize_name(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let trimmed = cleaned.trim();
    let mut s = if trimmed.is_empty() {
        "unnamed".to_string()
    } else {
        trimmed.to_string()
    };
    // 长度上限（按 char 计数，避免切断多字节）
    if s.chars().count() > 96 {
        s = s.chars().take(95).collect::<String>() + "…";
    }
    s
}
```

> `prefix_of(fp)` 的真实实现需要「fingerprint → 所属订阅名」的旁路表（合并时建立），
> 这里只示意签名。**不要**为了省事把订阅 uid 写进节点名（uid 无语义，用户看不懂）。

### 4.3 `node_key` 的稳定性风险（必须记录并缓解）

| 风险 | 触发 | 影响 | 缓解 |
| --- | --- | --- | --- |
| 新增第 2 个订阅导致原有节点被加前缀 | 跨订阅同名 | `store-selected` 缓存里的旧名失效 → 重启后回到组内第一个节点 | 合并后若组内选中项失效，用 fingerprint 映射把旧选择还原并 `PUT /proxies/{group}` 一次 |
| 机场改名节点 | 机场侧变更 | 同上 | 用 fingerprint 找回（fp 不含 name，改名后 fp 不变）→ 自动跟随 |
| 节点被删除 | 订阅变更 | 选中项消失 | 由 07 章 `SwitchReason::GroupMissing` 兜底切到最优 |

> `profile.store-selected: true` 让内核自己记住选择（`cache.db`），**但它是按名字记的**。
> 因此我们额外维护「fingerprint → node_key」映射做兜底，这是 07 章 `switch_apply_best` 之后
> 重建 config 时仍能保持用户选择的关键。

---

## 5. 合并后 config 的完整结构与必填字段

### 5.1 必填字段表（与附录 A 一致）

| 字段 | 取值 | 为什么必须 | 违反后果 |
| --- | --- | --- | --- |
| `mixed-port` | `ports::alloc()` 结果 | HTTP+SOCKS 混合端口，系统代理指向它 | 无端口 → 无法代理 |
| `external-controller` | `127.0.0.1:<controller_port>` | 本应用所有 REST 调用（测速/切换）依赖它 | 监听 `0.0.0.0` = 安全红线（C.12.2） |
| `secret` | 32 hex，运行时生成 | API 鉴权 | 空 secret = 本机任意程序可切换节点（C.12.2） |
| `mode` | `rule` | 规则模式；`global` 会忽略 rules | 用户规则全部失效 |
| `log-level` | `info`（Debug 时 `debug`） | 与 `settings.log_level` 映射 | 过吵的日志放大 IO |
| `allow-lan` | `false` | 仅本机可用 | 开放局域网代理 = 被邻居白嫖 + 安全风险 |
| `ipv6` | `true`（默认） | IPv6 总开关 | 关掉会让部分纯 IPv6 站点不可达 |
| `profile.store-selected` | `true` | 内核记住用户选择 | 每次重启回默认节点（用户困惑） |
| `unified-delay` | `true` | 统一延迟口径（去掉握手/首字节差异） | 不同节点延迟数字不可比，自动切换误判 |
| `tcp-concurrent` | `true` | 多 IP 并发握手取最快 | 部分站点解析到劣质 IP 时明显变慢 |
| `find-process-mode` | `strict`（默认） | 保留机场 `PROCESS-NAME` / `PROCESS-PATH` 规则能力 | **设成 `off` 会让所有进程类规则永久不匹配**（见下方说明） |
| `dns` | 见 §5.2 | 必需段 | 缺失时 TUN 模式无法解析 |
| `proxy-groups` | 见 §6 | 必需 | 无组则无法手动/自动切换 |
| `rules` | 见 §6.4，末条 `MATCH` | 兜底 | 无 `MATCH` → mihomo 启动即报错 |

| **禁止**出现的字段 | 原因 |
| --- | --- |
| `bind-address` | 只在 `allow-lan: true` 时生效；我们恒 `false`，写了会误导后续维护者 |
| `external-controller-cors` / `external-controller-tls` | 本机回环无需 CORS/TLS，徒增攻击面 |
| `external-ui` / `external-ui-url` | 不提供 Web UI；`external-ui-url` 会触发外网下载（违反铁律 1） |
| `port` / `socks-port` / `redir-port` | 只用 `mixed-port`，避免端口翻倍与分配复杂化 |
| 订阅原文里的 `tun` / `sniffer` / `proxy-providers` | 机场配置的这些段不可信，一律丢弃（TUN 由我们按 `proxy_mode` 自行生成） |
| 订阅原文里的 `rules` 之外的路由配置（`sub-rules`） | 未验证的嵌套结构，一律丢弃 |

### 5.2 `dns` 段（按 `proxy_mode` 分档）

| `proxy_mode` | `dns.enable` | 说明 |
| --- | --- | --- |
| `SystemProxy`（默认） | `false` | 不改动用户的 DNS 链路；域名由系统解析，HTTP/CONNECT 仍按域名匹配规则 |
| `Manual` | `false` | 同上 |
| `Tun` | `true` | TUN 必须接管 DNS，否则 fake-ip 无法工作 |

**为什么 `enable: false` 是正确默认**（源码依据，不是偏好）：`hub/executor/executor.go::updateDNS` 里

```go
if !c.Enable {
    // 清空全部 resolver 钩子
    resolver.DefaultResolver = nil
    ...
    dns.ReCreateServer("", nil, nil)   // ← 空地址 = 关闭 53 端口监听
    return
}
...
dns.ReCreateServer(c.Listen, lc, s)    // ← enable 为真才真的起监听
```

即 `enable: false` 时 mihomo **完全不创建 DNS 服务**（不监听任何端口，也不接管解析）；
`enable: true` 才按 `listen` 起监听。所以 SystemProxy 档写 `enable: true` 的净效果是
「多开一个 53 端口 + 多一份 resolver 常驻对象」，而流量根本不经过它 —— 纯负担。

```yaml
# SystemProxy / Manual 档（enable: false，但字段齐全，便于切换 Tun 时直接改一个布尔）
dns:
  enable: false
  ipv6: true
  enhanced-mode: fake-ip
  fake-ip-range: 198.18.0.1/16
  fake-ip-filter:
    - "*.lan"
    - "*.local"
    - "localhost.ptlogin2.qq.com"
    - "+.msftconnecttest.com"
    - "+.msftncsi.com"
  default-nameserver:
    - 223.5.5.5
    - 119.29.29.29
  nameserver:
    - https://223.5.5.5/dns-query
    - https://1.12.12.12/dns-query
  fallback:
    - tls://8.8.4.4:853
  fallback-filter:
    geoip: true
    geoip-code: CN
```

```yaml
# Tun 档（仅 enable 与监听差异）
dns:
  enable: true
  # 必需：dns-hijack 把流量劫持到本进程的 DNS 服务，该服务必须有个监听地址。
  # 这是**唯一**允许出现 0.0.0.0 的地方，理由与缓解见下表。
  listen: 0.0.0.0:53
  ipv6: true
  enhanced-mode: fake-ip
  fake-ip-range: 198.18.0.1/16
  # 其余同 SystemProxy 档
```

> **`dns.listen: 0.0.0.0:53` 是唯一豁免「禁止 0.0.0.0」的字段**，必须理解原因，不要顺手推广到其它端口：
>
> | 项 | 说明 |
> | --- | --- |
> | 为什么不能写 `127.0.0.1:53` | TUN 模式下流量从虚拟网卡进来，源/目的地址是 TUN 网段；只监听回环会导致 `dns-hijack` 劫持过来的包无人应答，表现为「TUN 开启后全部 DNS 解析失败」 |
> | LAN 暴露面 | 该 DNS 服务理论上可被同网段主机查询（因为绑定了全部接口） |
> | 缓解 1 | `allow-lan: false` 已关闭局域网代理入口；TUN 默认不开（只在 `proxy_mode = Tun` 时生成该段） |
> | 缓解 2 | Windows 防火墙对新程序的**入站**默认拦截；安装时不为 `mihomo.exe` 添加入站放行规则（08 章明确禁止） |
> | 缓解 3 | 该服务只在 TUN 运行时存在；退出/停止内核即消失 |
> | 验证要求 | 12 章验收必须实测「同网段另一台机器 `nslookup baidu.com <本机IP>` 失败或超时」；若可查询，视为安全问题必须修（方案：改用 `listen: <tun 网关IP>:53`） |
>
> 若将来 mihomo 支持 `dns.listen` 绑定到 TUN 设备名（而非地址），应优先采用 —— 目前只能绑地址。

> **为什么默认不用 fake-ip**：系统代理模式下应用自己解析域名再发 `CONNECT host:443`，
> 域名始终在规则里可见，fake-ip 没有收益却会干扰部分应用的 DNS 校验（如 `nslookup` 得到 198.18.x.x）。

### 5.3 `secret` 的生成与生命周期

| 项 | 规定 |
| --- | --- |
| 生成 | `getrandom` 填 16 字节 → 32 位小写 hex |
| 落盘 | `%LOCALAPPDATA%\ClashZhengl\runtime\secret`，ACL 仅当前用户（C.3、C.12.1） |
| 何时轮换 | **进程启动时**轮换（C.12.2「每次启动或每次 config 重建」） |
| 日志 | **禁止**打印 secret 全文；需要排错时只打 `secret_len=32` 或前 4 位 |
| 热更新限制 | **`PUT/PATCH /configs` 无法修改 `external-controller` 与 `secret`** —— 见 §8.1 |

```rust
// src-tauri/src/kernel/api.rs（生成侧，节选）
/// 32 位小写 hex 随机 secret。失败（熵源异常）直接返回错误，绝不退化成固定值。
pub fn generate_secret() -> Result<String, AppError> {
    let mut buf = [0u8; 16];
    getrandom::fill(&mut buf).map_err(|e| {
        AppError::new("E_UNSUPPORTED", "无法生成安全随机数", Some(e.to_string()), false)
    })?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}
```

### 5.4 完整 YAML 示例（2 个订阅、3 个节点、1 个重名冲突）

```yaml
# 由 Clash Zhengl 自动生成，请勿手工编辑（每次启动/订阅更新会重建）
# 生成时间：2026-11-06T16:40:12+08:00 · schema=2 · 订阅数=2 · 节点数=3
mixed-port: 7897
allow-lan: false
ipv6: true
mode: rule
log-level: info
unified-delay: true
tcp-concurrent: true
find-process-mode: strict
external-controller: 127.0.0.1:64512
secret: "<运行时生成的 32 位 hex，绝不写入文档/日志/示例>"
profile:
  store-selected: true
  store-fake-ip: false   # SystemProxy 档 dns.enable=false，fake-ip 不启用，此键无作用

dns:
  enable: false
  ipv6: true
  enhanced-mode: fake-ip
  fake-ip-range: 198.18.0.1/16
  fake-ip-filter:
    - "*.lan"
    - "*.local"
    - "+.msftconnecttest.com"
  default-nameserver:
    - 223.5.5.5
    - 119.29.29.29
  nameserver:
    - https://223.5.5.5/dns-query
    - https://1.12.12.12/dns-query

proxies:
  - name: HongKong-01
    type: ss
    server: hk01.example.com
    port: 8443
    cipher: aes-256-gcm
    password: "<机场下发，原样保留>"
    udp: true
  - name: Japan-02
    type: vmess
    server: jp02.example.com
    port: 443
    uuid: "<机场下发>"
    alterId: 0
    cipher: auto
    tls: true
    servername: jp02.example.com
    network: ws
    ws-opts:
      path: /ray
      headers:
        Host: jp02.example.com
  # 与 sub-a 的 HongKong-01 同名 → 加订阅前缀（N-2）
  - name: sub-b/HongKong-01
    type: trojan
    server: hk01.other-airport.net
    port: 443
    password: "<机场下发>"
    sni: hk01.other-airport.net
    skip-cert-verify: false
    udp: true

proxy-groups:
  # 只生成一个 select 主组（Lead 裁决：不生成每订阅子组）
  # 主组名统一为 Proxy，与附录 A.6.2 模板 / A.6.4 约束一致。
  # 禁用自动组的完整理由见附录 A.6.4（内核健康检查会与本应用测速形成两个决策者）。
  - name: Proxy
    type: select
    proxies:
      - HongKong-01
      - Japan-02
      - sub-b/HongKong-01
      - DIRECT

rules:
  # 来自 sub-a（目标 Proxy / DIRECT 均为已知组，原样保留）
  - DOMAIN-SUFFIX,example.com,Proxy
  - DOMAIN-SUFFIX,cn,DIRECT
  - IP-CIDR,10.0.0.0/8,DIRECT,no-resolve
  - IP-CIDR,192.168.0.0/16,DIRECT,no-resolve
  - GEOIP,CN,DIRECT
  # 兜底：必须是最后一条
  - MATCH,Proxy
```

> 注意示例里 **没有任何 `url-test` / `fallback` / `load-balance` 组** —— 见 §6.1 的硬约束与附录 A.6.4。
>
> **为什么只有一个组**：早期方案曾为每个订阅生成一个子组（便于「按机场整体切换」），
> 但 Lead 裁决简化为**只生成主组 `Proxy`**。理由：①子组会让 `switch_policy.scope = Global` 时
> `switch_apply_best.all` 长度随订阅数增长，切换一次要写 N 个组，失败面变大；
> ②用户按机场整体切换的需求可用「节点名前缀」在 UI 层分组展示满足，无需占用内核组；
> ③组越少，`PUT /proxies` 的状态同步越简单。该简化与附录 A.6.2 模板一致。

### 5.5 `find-process-mode` 为什么必须是 `strict`（不是 `off`）

实测 mihomo `tunnel/tunnel.go`：进程查找的开关直接决定「规则能不能匹配到进程」：

```go
switch FindProcessMode() {
case process.FindProcessAlways:
    helper.FindProcess()          // 立即查一次
    helper.FindProcess = nil
case process.FindProcessOff:
    helper.FindProcess = nil      // ← 彻底关掉，PROCESS-NAME/PROCESS-PATH 永远拿不到值
}
```

`rules/common/process.go` 的 `Match()` 依赖 `metadata.Process` / `metadata.ProcessPath`，
这两个字段只能由 `helper.FindProcess()` 填充。因此：

| 取值 | 语义 | 对我们的影响 |
| --- | --- | --- |
| `always` | 每条连接都强制查进程 | CPU 与内存开销最高（官方注释明确警告 "may increase many memory"） |
| `strict`（**选定**） | 仅当规则里真的出现进程类规则时才查 | 机场常用 `PROCESS-NAME,chrome.exe,...`，必须保留；开销按需 |
| `off` | 完全不查进程 | **机场下发的 `PROCESS-NAME` / `PROCESS-PATH` 规则静默失效** → 该走的代理不走，用户表现为「某软件连不上」 |

> 结论：**不要因为"看起来更省资源"就写 `off`**。`strict` 是 mihomo 的默认值，也是唯一兼顾正确性与开销的选项。
> 只有当我们确认合并后的 rules 里**不含**任何 `PROCESS-*` 规则时，才可以降为 `off`（可作为后续优化项：
> 在 `build_rules` 之后扫描一遍，若命中数为 0 则写 `off`）。当前版本**不做**该优化（少一个状态分支，减少 bug 面）。

### 5.6 `tun` 段：只在 `proxy_mode = Tun` 时才生成

| `proxy_mode` | 是否生成 `tun` 段 | 补充 |
| --- | --- | --- |
| `SystemProxy` / `Manual` | **不生成**（整个 `tun` 键不出现） | 避免内核尝试创建虚拟网卡（需要管理员，且会造成无谓的失败日志） |
| `Tun` | 生成，且 `dns.enable = true` + `dns.listen` | 需管理员权限，未提权时启动应返回 `E_PERM_ADMIN`（见 08 章） |

```yaml
# 仅 proxy_mode = Tun 时出现的段
tun:
  enable: true
  stack: mixed            # gvisor / mixed / system；mixed 兼容性最好
  auto-route: true
  auto-detect-interface: true
  dns-hijack:
    - 0.0.0.0:53          # 接管 DNS，配合 dns.enable = true
```

| 字段 | 取值 | 理由 |
| --- | --- | --- |
| `stack` | `mixed` | Windows 上 gvisor 纯用户态性能一般、system 依赖栈实现差异；`mixed` 是官方推荐折中 |
| `auto-route` | `true` | TUN 的意义就是接管全局路由；关掉等于没开 |
| `auto-detect-interface` | `true` | 避免硬编码网卡名（用户机器千差万别） |
| `dns-hijack` | `["0.0.0.0:53"]` | 默认值；配合 `dns.enable = true` 才能让 fake-ip 生效 |
| `mtu` | **不写** | 让内核用默认值；写错 MTU 会导致大包分片异常，极难排查 |

> TUN 相关的最小权限、`wintun.dll`、提权与回滚全部在 08 章。本章只负责「该不该写这段」与「写哪些键」。
> **不要**把订阅原文里的 `tun` 段搬进来：机场的 TUN 配置通常带 Linux 专用字段（`auto-redirect`、`routing-mark`），
> 直接搬会触发 `mihomo -t` 报错或行为异常。

---

## 6. 策略组生成规则

### 6.1 R-1/R-3：组类型硬约束（C.5 v1.2）

> **契约原文（C.5）**：`switch_policy.scope` 指向的策略组**必须**生成为 `type: select`。
> 禁止同时生成同名 `type: url-test` / `fallback` 组 —— 否则内核自带健康检查会与本应用的定时测速
> 争夺「当前节点」，表现为节点来回跳。自动切换由本应用**显式** `PUT /proxies/{group}` 完成。

**冲突机理（必须理解，否则会"好心"把组改成 url-test）**：

| 主体 | 行为 | 结果 |
| --- | --- | --- |
| 内核 `url-test` 组 | 按自己的 `interval`（如 300 s）跑自己的健康检查，**自动**改写组内选中项 | 内核写一次 |
| 本应用 06/07 章 | 每 `speedtest_interval_min`（默认 30 min）测速后 `PUT /proxies/{group}` | 我们写一次 |
| 两者叠加 | 两个写入者时间不同步、判据不同（内核不看 `tolerance_ms`/`min_dwell_sec`） | **节点来回跳**，用户看到"一直在切换" |

所以：**同一组只能是 `select`，且只有本应用有写权限**。

| 规则 | 内容 | 校验方式 |
| --- | --- | --- |
| **R-1** | 主组（`MAIN_GROUP`）必须 `type: select` | 单测断言 |
| **R-2** | **只生成主组这一个组**，不为每个订阅生成子组（Lead v1.4 裁决；越少组越少同步面） | 单测断言 `groups.len() == 1` |
| **R-3** | 生成结果中**不得存在任何** `type: url-test` / `fallback` / `load-balance` 组 | 单测断言（遍历 `proxy-groups`）+ 运行时 `assert_no_auto_groups` fail-fast |
| **R-4** | 组名唯一；且不得与任何节点 `name` 冲突（若冲突，节点侧改名，组名优先保留） | 单测断言 |
| **R-5** | 主组第一个成员 = 合并后第一个节点（保证空 `store-selected` 时有可用默认） | 单测断言 |
| **R-6** | 主组最后追加 `DIRECT`（便于排障时绕过代理） | 单测断言 |
| **R-7** | `switch_policy.scope = Global` 时，主组名固定为 `Proxy`；`Group(name)` 时必须等于 `name` | 单测断言 |

**反例 vs 正例**

```yaml
# ❌ 错误写法 1：同名组同时有 url-test
proxy-groups:
  - name: Proxy
    type: url-test          # ← 内核健康检查会抢着改选中项
    url: http://www.gstatic.com/generate_204
    interval: 300
    proxies: [HongKong-01, Japan-02]
  - name: Proxy
    type: select            # ← 同名重复，mihomo 直接配置报错
    proxies: [HongKong-01, Japan-02]
```

```yaml
# ❌ 错误写法 2：主组是 select，但里面套一个 url-test 组，主组指向它
proxy-groups:
  - name: Auto
    type: url-test          # ← 仍然是内核在改选中项
    url: http://www.gstatic.com/generate_204
    interval: 300
    proxies: [HongKong-01, Japan-02]
  - name: Proxy
    type: select
    proxies: [Auto, DIRECT]  # ← 用户/本应用切 Proxy 到 Auto 后，实际节点由内核决定
```

```yaml
# ✅ 正确写法：只有一个 select 组，扁平列出全部节点，自动切换由本应用 PUT 完成
#    主组名统一为 Proxy，与附录 A.6.2 模板一致；禁用自动组的完整理由见附录 A.6.4。
proxy-groups:
  - name: Proxy
    type: select
    proxies: [HongKong-01, Japan-02, sub-b/HongKong-01, DIRECT]
```

> **如果用户想要「内核原生 url-test」的省电模式怎么办？**
> 契约不允许（会与本应用测速打架）。若确有需求，走 `CONTRACT-CHANGE-REQUEST:`：
> 建议做法是「`switch_policy.enabled = false` 时允许生成一个独立的 `url-test` 组供用户手动选」，
> 但**当前不得实现**（附录 A.6.4 同此结论）。

### 6.2 组名与 scope 的映射

> **已定稿（v1.4 / Lead 裁决）**：只生成**一个**主组 `Proxy`，**不再为每个订阅生成子组**。
> 主组名与附录 A.6.2 模板一致。

| `switch_policy.scope` | 参与自动切换的组 | 主组名 | 说明 |
| --- | --- | --- | --- |
| `Global` | 主组 `Proxy` | `Proxy` | 组数恒为 1，`switch_apply_best.all` 长度恒为 1 |
| `Group(name)` | **仅** `name` 组（必须存在于 config 中，否则内核返回 404） | `name` | 同上，长度恒为 1 |

| 常量 | 值 | 说明 |
| --- | --- | --- |
| `MAIN_GROUP_DEFAULT` | `"Proxy"` | **已定稿**：附录 A.6.2 模板采用同名；`scope = Global` 或 `Group` 名为空时使用 |
| 订阅组 | **不生成** | 早期方案曾按订阅分组，已按 Lead 裁决移除 |

> **`scope = Group(name)` 的前置校验**：`name` 必须是本次合并实际生成的组名。
> 生成器在 `build_proxy_groups` 之后要断言 `groups` 里存在该名字，否则直接
> `Err(E_PARSE_YAML)`（带 detail `scope group not found`）—— 不要等内核 404 才发现，
> 那时配置已经写盘、`mihomo -t` 也已通过，排查成本高得多。

> **不再有「订阅名与组名冲突」问题**：由于不生成订阅组，组名空间里只有 `Proxy` 一个名字，
> 与订阅名（`Subscription.name`）不会碰撞。节点前缀的唯一化仍按 N-2/N-3 处理（见 §4.2）。

### 6.3 组生成实现

```rust
// src-tauri/src/subscription/manager.rs（节选）
use serde_yaml_ng::{Mapping, Value};

pub const MAIN_GROUP_DEFAULT: &str = "Proxy";

fn s(v: &str) -> Value {
    Value::String(v.to_string())
}

/// 构造一个 select 组。
fn select_group(name: &str, members: &[String]) -> Value {
    let mut m = Mapping::new();
    m.insert(s("name"), s(name));
    // R-1/R-2/R-3：硬编码 select，且**不生成** url/interval 字段
    m.insert(s("type"), s("select"));
    let seq: Vec<Value> = members.iter().map(|x| s(x)).collect();
    m.insert(s("proxies"), Value::Sequence(seq));
    Value::Mapping(m)
}

/// 生成 proxy-groups。入参均为确定性顺序，保证幂等。
///
/// 主组名统一为 `Proxy`，与附录 A.6.2 模板一致；禁用自动组的完整理由见附录 A.6.4。
/// 本函数**只生成一个主组**，不再为每个订阅生成子组（Lead v1.4 裁决）。
pub fn build_proxy_groups(
    main_group: &str,
    all_node_keys: &[String],
) -> Vec<Value> {
    let mut groups = Vec::with_capacity(1);

    // R-1 + R-5：主组 = 全部节点 + DIRECT。
    // 顺序即节点顺序（按订阅 sort_order + 订阅内原始顺序），保证幂等。
    let mut main_members: Vec<String> = all_node_keys.to_vec();
    main_members.push("DIRECT".to_string());
    groups.push(select_group(main_group, &main_members));

    groups
}

/// R-3 的运行时自检：任何 url-test/fallback/load-balance 都视为 bug，直接 fail-fast。
pub fn assert_no_auto_groups(groups: &[Value]) -> Result<(), AppError> {
    for g in groups {
        let Some(m) = g.as_mapping() else { continue };
        let ty = m
            .get(Value::String("type".into()))
            .and_then(Value::as_str)
            .unwrap_or("");
        if matches!(ty, "url-test" | "fallback" | "load-balance" | "relay") {
            return Err(AppError::new(
                "E_PARSE_YAML",
                "内部配置错误：不允许生成自动测速策略组",
                Some(format!("group type `{ty}` is forbidden by contract C.5")),
                false,
            ));
        }
    }
    Ok(())
}
```

### 6.4 `rules` 合并与目标重映射

| 步骤 | 规则 |
| --- | --- |
| 收集顺序 | 按订阅 `sort_order`，同订阅内保持文件顺序 |
| 去重 | 规则**整行字符串**完全相同则丢弃后来的（保留首次出现） |
| 目标重映射 | 拆出最后一或第二个字段（策略目标）后按下表处理 |
| 数量上限 | 合计 ≤ 20 000 条；超出截断并 WARN（法规/内存保护） |
| 兜底 | 最后**必须**是 `MATCH,<主组>`；若原始最后一条已是 `MATCH`，替换其目标为主组 |

**目标重映射表**

| 原目标 | 处理 |
| --- | --- |
| `DIRECT` / `REJECT` / `REJECT-DROP` / `PASS` / `COMPATIBLE` | 原样保留（内核内置） |
| 命中「组名映射表」（如原名 `Proxy`、订阅名） | 换成合并后的实际组名 |
| 命中「节点名映射表」（订阅内旧名 → 新 `node_key`） | 换成新 `node_key` |
| 其它（订阅引用了不存在的组/节点） | **丢弃该条并计数**，WARN（不失败） |

> 丢弃是**有意为之**：留着指向不存在目标的规则会让 `mihomo -t` 直接报错（规则目标必须是已知 proxy/group），
> 那会导致整个 config 不可用 —— 为几条规则牺牲整个订阅不值得。

```rust
// src-tauri/src/subscription/manager.rs（节选）
/// 重映射单条规则；返回 None 表示应丢弃。
pub fn remap_rule(
    rule: &str,
    group_map: &HashMap<String, String>,
    node_map: &HashMap<String, String>,
) -> Option<String> {
    let parts: Vec<&str> = rule.split(',').map(str::trim).collect();
    if parts.len() < 2 {
        return None; // 形如 "MATCH" 的残条
    }
    let kind = parts[0].to_ascii_uppercase();

    // MATCH 只有两个字段：MATCH,<target>
    let (target_idx, target) = if kind == "MATCH" {
        (1, parts[1])
    } else {
        // 常规规则：...,<target>[,no-resolve|src|...]
        // 目标位置是「最后一个非附加参数」；附加参数只有这些白名单值
        const MODIFIERS: [&str; 6] = ["no-resolve", "src", "dst", "no-drop", "force-remote-dns", "tcp"];
        let mut idx = parts.len() - 1;
        while idx > 1 && MODIFIERS.contains(&parts[idx].to_ascii_lowercase().as_str()) {
            idx -= 1;
        }
        (idx, parts[idx])
    };
    if target.is_empty() {
        return None;
    }

    let resolved = match target {
        "DIRECT" | "REJECT" | "REJECT-DROP" | "PASS" | "COMPATIBLE" => target.to_string(),
        t => group_map
            .get(t)
            .or_else(|| node_map.get(t))
            .cloned()
            .unwrap_or_default(),
    };
    if resolved.is_empty() {
        return None; // 未知目标 → 丢弃
    }

    let mut out: Vec<String> = parts.iter().map(|x| x.to_string()).collect();
    out[target_idx] = resolved;
    Some(out.join(","))
}

/// 组装最终 rules：合并 → 去重 → 重映射 → 兜底 MATCH。
pub fn build_rules(
    per_sub_rules: &[Vec<String>],
    group_map: &HashMap<String, String>,
    node_map: &HashMap<String, String>,
    main_group: &str,
) -> (Vec<String>, u32) {
    const MAX_RULES: usize = 20_000;
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<String> = Vec::new();
    let mut dropped = 0u32;

    for rules in per_sub_rules {
        for raw in rules {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // 丢弃订阅里自带的 MATCH（我们的兜底统一追加）
            if line.to_ascii_uppercase().starts_with("MATCH") {
                continue;
            }
            match remap_rule(line, group_map, node_map) {
                Some(mapped) => {
                    if seen.insert(mapped.clone()) {
                        out.push(mapped);
                    }
                }
                None => dropped += 1,
            }
            if out.len() >= MAX_RULES {
                tracing::warn!(limit = MAX_RULES, "rules truncated");
                break;
            }
        }
    }

    out.push(format!("MATCH,{main_group}"));
    (out, dropped)
}
```

> `remap_rule` 的白名单 `MODIFIERS` 必须覆盖 mihomo 常用后缀。若未来新增参数（如 `dns-failed`），
> 漏掉会被当成 target 而误丢弃规则 —— 因此单测要覆盖每种后缀组合。

---

## 7. 生成、序列化与预校验

### 7.1 序列化的确定性

| 要求 | 做法 |
| --- | --- |
| 顶层键顺序固定 | 手工用 `Mapping::insert` 按 §5.4 顺序插入（`serde_yaml_ng::Mapping` 保序） |
| 不做 AST 全量重排 | 只对我们构造的 `Mapping` 序列化；订阅原文用 `Value` 片段直接搬过来 |
| 不输出 `null` | 空值时**不插入该键**，而不是插入 `Null`（`Null` 会让 `mihomo -t` 报类型错） |
| 行宽 | 默认即可；不要用 `serde_yaml_ng` 的 flow 风格输出（难以人工排查） |

```rust
// src-tauri/src/subscription/manager.rs（节选）
/// 把合并结果序列化为最终 YAML 字节。
pub fn serialize_runtime(root: &Mapping) -> Result<Vec<u8>, AppError> {
    let mut out = String::with_capacity(64 * 1024);
    out.push_str("# 由 Clash Zhengl 自动生成，请勿手工编辑（每次启动/订阅更新会重建）\n");
    out.push_str(&format!(
        "# 生成时间：{} · schema={} · 仅供本机内核使用\n",
        crate::util::time::rfc3339_local(crate::util::time::now_unix()),
        crate::domain::model::CURRENT_SCHEMA_VERSION
    ));
    let body = serde_yaml_ng::to_string(&Value::Mapping(root.clone())).map_err(|e| {
        AppError::new(
            "E_PARSE_YAML",
            "无法生成运行配置",
            Some(format!("serialize: {e}")),
            false,
        )
    })?;
    out.push_str(&body);
    Ok(out.into_bytes())
}
```

### 7.2 `mihomo -t` 预校验（**强制步骤**）

在把 config 交给内核之前，用内核自己校验一遍：`-t` 会完整解析配置并退出。

```powershell
# 手工验证命令（与代码里完全一致）
& "$env:LOCALAPPDATA\ClashZhengl\runtime\core\v1.19.33\mihomo.exe" `
  -d "$env:LOCALAPPDATA\ClashZhengl\runtime" `
  -f "$env:LOCALAPPDATA\ClashZhengl\runtime\config.yaml" `
  -t
# 成功：打印 "configuration file ... test is successful"，退出码 0
# 失败：打印 "configuration file ... test failed" + 错误详情，退出码 1
```

| 项 | 规定 |
| --- | --- |
| 时机 | 每次写完 `runtime/config.yaml` 之后、热更新/重启之前 |
| 超时 | 8 s（含 geo 数据库加载；超时视为失败） |
| 退出码 0 | 通过 |
| 退出码 1 | 失败 → 保留旧 config 文件（`runtime/config.rejected.yaml` 留档）、不改内核、返回 `E_CORE_START_FAILED`，detail 为 **stderr+stdout** 末尾 500 字符（需脱敏） |
| 退出码其它 / 无法执行 | `E_CORE_NOT_FOUND` 或 `E_CORE_START_FAILED` |
| 成本 | 约 200–600 ms；只在「重建 config」时跑，不是热路径 |
| 为什么要它 | 我们的合并是字符串级改写，`serde_yaml_ng` 只能保证 YAML 合法，**不能**保证 mihomo 语义合法（未知字段、规则目标不存在、组内成员不存在） |

```rust
// src-tauri/src/kernel/supervisor.rs（节选，被 manager 调用）
/// 用内核自身校验 config。返回 Ok(()) 表示可安全应用。
pub async fn validate_config(
    core_exe: &std::path::Path,
    runtime_dir: &std::path::Path,
    config_path: &std::path::Path,
) -> Result<(), AppError> {
    use tokio::process::Command;

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(8),
        Command::new(core_exe)
            .arg("-d")
            .arg(runtime_dir)
            .arg("-f")
            .arg(config_path)
            .arg("-t")
            .stdin(std::process::Stdio::null())
            .output(),
    )
    .await
    .map_err(|_| {
        AppError::new("E_CORE_START_FAILED", "内核配置校验超时", Some("mihomo -t timeout 8s".into()), true)
    })?
    .map_err(|e| {
        AppError::new("E_CORE_NOT_FOUND", "未安装内核，请先下载", Some(e.to_string()), true)
    })?;

    if output.status.success() {
        return Ok(());
    }

    // 实测 mihomo main.go：错误详情走 `log.Errorln`（stderr），而
    // "configuration file ... test failed" 走 `fmt.Printf`（stdout）。
    // 因此必须**两路都收**，只读 stderr 会丢掉关键字样。
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stderr}\n{stdout}");
    let tail: String = combined
        .trim()
        .chars()
        .rev()
        .take(500)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    Err(AppError::new(
        "E_CORE_START_FAILED",
        "合并配置未通过内核校验，请查看日志",
        Some(crate::util::log::redact::scrub(&tail)),
        false,
    ))
}
```

---

## 8. 原子写与热更新决策

### 8.1 三种应用方式的能力边界（**实测依据**）

| 方式 | 能改什么 | 不能改什么 | 依据 |
| --- | --- | --- | --- |
| `PATCH /configs` | 仅 ±22 个「通用字段」（全部来自 `hub/route/configs.go::configSchema`，逐一核对过）：`port`、`socks-port`、`redir-port`、`tproxy-port`、`mixed-port`、`tun`、`tuic-server`、`ss-config`、`vmess-config`、`tcptun-config`、`udptun-config`、`allow-lan`、`skip-auth-prefixes`、`lan-allowed-ips`、`lan-disallowed-ips`、`bind-address`、`mode`、`log-level`、`ipv6`、`sniffing`、`tcp-concurrent`、`find-process-mode`、`interface-name` | **`proxies` / `proxy-groups` / `rules` / `dns` / `hosts` / `sniffer` / `geodata` 一律改不了**（`configSchema` 里根本没有这些字段，传了会被 `DecodeJSON` 静默忽略） | mihomo `hub/route/configs.go::patchConfigs` + `configSchema` |
| `PUT /configs?force=false` `{"path":"<abs>"}` | 完整重解析配置文件：`proxies`、`groups`、`rules`、`dns`、`hosts`、`sniffer`… | **不重建监听器**（`updateListeners` 在 `force=false` 时直接 return），因此 `mixed-port` 变更不生效；`external-controller` / `secret` 永不生效 | `hub/executor/executor.go::updateListeners` |
| `PUT /configs?force=true` | 同上 + **重建全部监听器**（HTTP/SOCKS/mixed/redir/tproxy/tun 全部 close 再 open） | `external-controller` / `secret` 仍不生效（`ApplyConfig` 明确「without ExternalController」） | 同上 |
| 重启内核进程 | 一切（含端口、secret、geodata 加载参数） | — | 进程级重建 |

> 这条边界是本项目最重要的实现事实：**改节点/组/规则不需要重启内核；改端口/secret 必须重启。**
> 早期把「订阅更新」实现成「重启内核」是轻量化的最大杀手——每次更新都会断流 1–2 s。

### 8.2 决策表（必须实现为纯函数）

```rust
// src-tauri/src/kernel/supervisor.rs（节选）
/// 由「新 config 相对旧 config 的变化」决定应用方式。纯函数，可单测。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyMode {
    /// 只改了节点/组/规则/DNS：PUT /configs?force=false，不断流
    HotReload,
    /// 改了监听端口或 TUN：PUT /configs?force=true（监听器重建，短暂断流）
    HotReloadWithListeners,
    /// 改了 external-controller / secret / 内核版本：必须重启进程
    RestartProcess,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConfigFingerprint {
    pub mixed_port: u16,
    pub controller_port: u16,
    /// secret 只存哈希，避免在内存快照里出现明文
    pub secret_hash: [u8; 8],
    pub tun_enabled: bool,
    /// 除上面几项外的其余内容哈希（proxies/groups/rules/dns…）
    pub body_hash: [u8; 8],
}

pub fn decide_apply_mode(old: ConfigFingerprint, new: ConfigFingerprint, core_running: bool) -> ApplyMode {
    if !core_running {
        return ApplyMode::RestartProcess; // 没在跑，直接起
    }
    if new.controller_port != old.controller_port || new.secret_hash != old.secret_hash {
        return ApplyMode::RestartProcess;
    }
    if new.mixed_port != old.mixed_port || new.tun_enabled != old.tun_enabled {
        return ApplyMode::HotReloadWithListeners;
    }
    if new.body_hash != old.body_hash {
        return ApplyMode::HotReload;
    }
    ApplyMode::HotReload // 无变化：仍走一次 PUT 也无害，幂等
}
```

| 变化内容 | 应用方式 | 断流 | 用户可见 |
| --- | --- | --- | --- |
| 只有 `proxies` / `proxy-groups` / `rules` / `dns` | `HotReload`（`force=false`） | 无（已建立连接继续走旧路径，新连接走新规则） | 无感 |
| `mixed-port` 变化（端口冲突后换端口） | `HotReloadWithListeners`（`force=true`） | 约 100–500 ms | 需同步更新系统代理端口（08 章） |
| `tun.enable` 变化 | `HotReloadWithListeners` | 约 0.5–2 s（虚拟网卡重建） | 可能瞬时断网 |
| `external-controller` 端口 / `secret` / 内核版本 | `RestartProcess` | 1–2 s | 提示「正在重启内核」 |

### 8.3 热更新实现

```rust
// src-tauri/src/kernel/api.rs（节选）
/// 热重载：把配置文件路径交给内核，让它自己读。
/// 必须传**绝对路径**，且路径必须在 mihomo 的 home dir（`-d` 目录）下，
/// 否则触发 `IsSafePath` 检查 → 400 `path is not subpath of home directory`。
pub async fn reload_config(&self, abs_path: &std::path::Path, force: bool) -> Result<(), AppError> {
    let url = format!(
        "{}/configs?force={}",
        self.base_url,           // http://127.0.0.1:<port>
        if force { "true" } else { "false" }
    );
    let body = serde_json::json!({ "path": abs_path.to_string_lossy() });

    let resp = self
        .client
        .put(&url)
        .bearer_auth(&self.secret)
        .json(&body)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(map_reqwest_err)?;

    match resp.status() {
        reqwest::StatusCode::NO_CONTENT => Ok(()),
        reqwest::StatusCode::UNAUTHORIZED => Err(AppError::new(
            "E_CORE_API_UNAUTH",
            "内核 API 鉴权失败（secret 不一致）",
            None,
            false,
        )),
        reqwest::StatusCode::BAD_REQUEST => {
            let text = resp.text().await.unwrap_or_default();
            // 典型：SAFE_PATHS 不通过、配置解析失败
            Err(AppError::new(
                "E_CORE_START_FAILED",
                "内核拒绝了新配置",
                Some(crate::util::log::redact::scrub(&text)),
                false,
            ))
        }
        s => Err(AppError::new(
            "E_CORE_START_FAILED",
            format!("内核返回 HTTP {}", s.as_u16()),
            None,
            true,
        )),
    }
}
```

**`SAFE_PATHS` 的坑（必须记住）**：mihomo 的 `IsSafePath` 只允许「`-d` 指定的 home dir 子路径」或
`SAFE_PATHS` 环境变量里的路径。我们把配置写到 `runtime/config.yaml` 且 `-d runtime`，
所以路径天然安全。**若把 config 放到 `%TEMP%` 再 PUT，会直接 400。** 这也是
「runtime 目录必须由 `-d` 指向」的原因，不要为了"整洁"把 config 挪走。

### 8.4 失败与回滚

| 失败点 | 处置 |
| --- | --- |
| 序列化失败 | 不写文件；返回 `E_PARSE_YAML`；旧 config 不动 |
| 原子写失败 | 返回 `E_STORE_IO`；旧 config 不动（rename 失败时 `.tmp` 保留用于排查） |
| `mihomo -t` 失败 | 新文件改名 `runtime/config.rejected.yaml`（保留 1 份，供诊断导出）；旧 config 不动；返回 `E_CORE_START_FAILED` |
| `PUT /configs` 4xx | 视为内核拒绝；**不要**自动重启（可能是我们生成了非法配置），返回错误 + 保留 `config.rejected.yaml` |
| `PUT /configs` 5xx / 超时 | 内核可能已进入不确定状态 → 检查 `/version`；失败则 `RestartProcess`；再失败 `CoreState::Failed` |
| `RestartProcess` 连续失败 3 次 | 退避 1/2/4 s；仍失败 → `core:state = Failed(reason)` + 中文通知；**保持 UI 可用**（铁律 4） |

> **热更新导致内核退出的风险（必须说明）**：`PUT /configs?force=true` 会 close 再 open 监听器。
> 若新端口已被别的进程占用（例如另一个代理软件），内核会在 `ReCreateMixed` 处失败。
> 实测表现可能是**内核进程直接退出**（下一轮健康检查发现 `/version` 连不上）。
> 因此：① `force=true` 前必须由 `ports::alloc` 保证端口空闲；② 调用后 500 ms 内做一次 `/version` 探活；
> ③ 探活失败即进入重启流程，并对用户提示一次（不要静默）。

---

## 9. 写盘约束与确定性校验

### 9.1 写盘流程

```rust
// src-tauri/src/subscription/manager.rs（节选）
/// 生成并落盘 runtime/config.yaml。返回指纹供 decide_apply_mode 使用。
pub fn write_runtime_config(
    bytes: &[u8],
    runtime_dir: &std::path::Path,
) -> Result<std::path::PathBuf, AppError> {
    let path = runtime_dir.join("config.yaml");
    // 复用 03 章 §6.2 的 atomic_write（tmp → flush → sync_all → rename）
    crate::store::json_store::atomic_write(&path, bytes)?;
    Ok(path)
}
```

| 约束 | 说明 |
| --- | --- |
| 目录 | `%LOCALAPPDATA%\ClashZhengl\runtime\`（C.3，Local 不漫游） |
| 文件名 | 固定 `config.yaml`（内核 `-d` 下的默认名，也便于 `-f` 显式指定） |
| 权限 | 目录 ACL 仅当前用户（含 secret 文件，同目录） |
| 保留 | 最多保留 `config.rejected.yaml` 一份；不保留历史版本（重建成本低，省磁盘） |
| fsync | 必须。否则断电后内核可能读到空文件（本次启动失败，且 UI 报错难解释） |

### 9.2 确定性单测（必须实现）

| 测试名 | 断言 |
| --- | --- |
| `merge_is_idempotent` | 同一输入连续生成两次，`bytes` 完全相同（逐字节） |
| `dedup_same_node_across_subs` | 两个订阅含同一 ss 节点 → 结果只有 1 个，`dropped_duplicates == 1` |
| `rename_on_conflict` | 两订阅各有 `HongKong-01`（不同类型/服务器）→ 后者变 `sub-b/HongKong-01` |
| `rename_suffix_when_still_dup` | 同一订阅内两个 `HK` → 第二个 `HK #2` |
| `no_auto_groups` | 生成的 `proxy-groups` 中不含 `url-test`/`fallback`/`load-balance`（R-3） |
| `main_group_is_select` | 主组 `type == "select"`（R-1） |
| `main_group_first_member_is_node` | 主组第 0 个成员是节点，不是组（R-5） |
| `rules_end_with_match` | 最后一条规则严格等于 `MATCH,<主组>` |
| `rules_unknown_target_dropped` | 目标为不存在组的规则被丢弃，`dropped_rules` 正确 |
| `rules_modifier_preserved` | `IP-CIDR,10.0.0.0/8,DIRECT,no-resolve` 重映射后 `no-resolve` 保留 |
| `disabled_sub_excluded` | `enabled=false` 的订阅不产生任何节点（§2） |
| `all_disabled_yields_empty` | 全部停用 → `E_PARSE_EMPTY` |
| `node_key_url_encoded` | 含 `/` 的 `node_key` 在 API 路径中被编码为 `%2F`（07 章依赖） |
| `fingerprint_no_secret_leak` | `node_fingerprint` 的返回值与日志字段均不含凭据原文 |
| `secret_is_32_hex` | `generate_secret()` 长度 32 且全为 `[0-9a-f]` |
| `decide_apply_mode_table` | §8.2 决策表每一行都有一条用例 |

---

## 10. 本章自检清单（Agent 必须逐条确认）

> 每条都必须能用手上的测试或命令证明，不接受「应该没问题」。

- [ ] 1. `Subscription.enabled == false` 的订阅是否**完全**不参与合并（无节点、无组、无规则）？
- [ ] 2. 合并后 `proxies` 为空时是否返回 `E_PARSE_EMPTY`（而不是生成一份没有节点的 config 交给内核）？
- [ ] 3. 去重指纹是否 = `SHA-256(type\0server\0port\0credential)` 的前 16 hex？**是否不含 `name`**？
- [ ] 4. 指纹/凭据是否**绝不**出现在日志里（日志只允许 `fp=` 前 8 位）？抓一份日志搜凭据原文验证过吗？
- [ ] 5. 跨订阅同名节点是否按 C.5 加订阅前缀 `<subName>/<nodeName>`？加前缀后的 `node_key` 与写入 `proxies[].name` 是否**完全一致**（没有第二套映射）？
- [ ] 6. `kernel/api.rs` 是否对节点名做了 `percent_encode`（否则含 `/`、空格、`+` 的名字在 `/proxies/{name}/delay` 会 404）？
- [ ] 7. **R-1/R-2**：是否**只生成一个** `type: select` 主组 `Proxy`（没有为每个订阅生成子组）？组名是否与附录 A.6.2 模板一致？
- [ ] 8. **R-3**：生成的 `proxy-groups` 里是否**没有任何** `url-test` / `fallback` / `load-balance` / `relay`？有 `assert_no_auto_groups` 单测吗？
- [ ] 9. 是否有「同名组同时是 select 和 url-test」的错误写法出现在任何示例/代码里？（必须为 0）
- [ ] 10. config 是否包含 C.5 要求的全部必填字段：`mixed-port`、`external-controller: 127.0.0.1:<port>`、`secret`、`mode: rule`、`log-level`、`allow-lan: false`、`ipv6`、`profile.store-selected: true`、`unified-delay: true`、`tcp-concurrent: true`、`dns`、`rules` 兜底 `MATCH`？
- [ ] 11. 是否**没有**生成 `bind-address`（`allow-lan: false` 下它无效，写了会误导）？
- [ ] 12. `external-controller` 是否恒为 `127.0.0.1:<port>`？有无 `0.0.0.0`？`secret` 是否 32 hex 且非空？是否只出现在 `runtime/secret`（Local）而非日志/文档示例？
- [ ] 13. 文档/示例里的 `secret` 是否用占位符（`"<运行时生成的 32 位 hex…>"`）而**不是**真实值？
- [ ] 14. `rules` 合并是否①按 `sort_order` ②整行去重 ③目标三级重映射 ④未知目标丢弃并计数 ⑤末条强制 `MATCH,<主组>`？
- [ ] 15. 连续两次生成是否**逐字节相同**（`merge_is_idempotent` 通过）？成员/组/规则顺序是否都不依赖 `HashMap` 迭代序？
- [ ] 16. `runtime/config.yaml` 是否走原子写（tmp → flush → fsync → rename），且失败时旧 config 不被破坏？
- [ ] 17. 是否在应用前执行了 `mihomo -d <runtime> -f <runtime>/config.yaml -t`，并按退出码处理（失败保留 `config.rejected.yaml` + 不动内核）？
- [ ] 18. 应用方式是否用 `decide_apply_mode`（纯函数）判定，而不是「一律重启内核」或「一律 force=true」？
- [ ] 19. 是否明确知道 `PATCH /configs` **改不了** proxies/groups/rules/dns，且 `PUT /configs` **无论 force 与否都改不了** `external-controller`/`secret`？
- [ ] 20. `PUT /configs` 传的路径是否**绝对路径**且位于 `-d` 的 home dir 下（否则 `IsSafePath` 返回 400）？
- [ ] 21. `force=true` 前是否确认目标端口空闲？调用后是否做了 `/version` 探活？探活失败是否有明确的重启与用户提示（不静默）？
- [ ] 22. 重启内核失败是否退避 1/2/4 s、最多 3 次、最终 `CoreState::Failed` 且**保持 UI 可用**（中文提示）？
- [ ] 23. `dns` 段是否按 `proxy_mode` 分档（SystemProxy/Manual → `enable: false`；Tun → `enable: true`）？
- [ ] 24. 是否**未**把订阅原文的 `tun` / `sniffer` / `proxy-providers` / `external-ui-url` 搬进 runtime config？
- [ ] 25. 本章是否**未**修改 `01`/`C`/其它章节文件？契约缺口是否以 `CONTRACT-CHANGE-REQUEST:` 提出？
- [ ] 26. `find-process-mode` 是否写成 `strict`（**不是** `off`）？是否理解 `off` 会让机场 `PROCESS-NAME`/`PROCESS-PATH` 规则静默失效？
- [ ] 27. 是否**没有**用 `hex` crate（不在 02 章 §5.1 白名单）？指纹 hex 是否用 `format!("{b:02x}")` 手写？
- [ ] 28. `dns` 段在 `proxy_mode = Tun` 时是否带 `listen` 且 `enable: true`，且 config 有 `tun` 段（`stack`/`auto-route`/`dns-hijack`）？在 SystemProxy/Manual 档下是否**没有**生成 `tun` 段？
- [ ] 29. `secret` 是否在进程启动时轮换、且写盘到 `runtime/secret`（Local，ACL 仅当前用户）？是否**没有**出现在 `config.rejected.yaml` 或诊断包明文中？
- [ ] 30. `dns.listen: 0.0.0.0:53` 是否**只在** `proxy_mode = Tun` 时出现，且 §5.2 的豁免说明与「同网段实测不可查」验收项都写进了 12 章？其它任何字段是否都没有绑定 `0.0.0.0`？

---

## 11. 契约对齐记录（v1.3）

> 本章原按 v1.2 落笔；C 章升到 **v1.3**（§C.6.1 内部接口冻结面）后复核，补充第 13–16 行。
> 本章是**产出方**（生成组/节点），不是 selector API 的消费方，因此 v1.3 不改变本章代码，只增加两条硬约束。

| # | 本章做法 | 对应契约 |
| --- | --- | --- |
| 1 | 生成 `proxy-groups` 时**只生成 `type: select`**，且**只生成一个主组 `Proxy`**（不生成每订阅子组）；**禁止**生成 `url-test`/`fallback`/`load-balance`/`relay`，并加运行时断言 `assert_no_auto_groups`；给出「同名 select + url-test」与「select 套 url-test」两种反例 | C.5「合并配置的组类型约束（对 04 章的要求）」、附录 A.6.4 |
| 2 | 自动切换只由本应用 `PUT /proxies/{group}` 完成，内核侧不设自动组 | C.5 同上、C.14「selector」 |
| 3 | 节点冲突加前缀 `sub-a/HongKong-01`，且 `node_key` ≡ 写入 config 的 `name` | C.5 `ProbeResult.node_key` 注释 |
| 4 | 去重指纹只把 hash 前 8 位写日志，凭据绝不入日志 | C.9、C.12.1、01 §8 |
| 5 | 必填字段：`mixed-port`、`external-controller: 127.0.0.1:<port>`、`secret`(32 hex)、`mode: rule`、`log-level`、`allow-lan: false`、`ipv6`、`profile.store-selected: true`、`unified-delay: true`、`tcp-concurrent: true`、`dns`、`rules` 末条 `MATCH` | C.12.2 + 附录 A 字段表 |
| 6 | **不写 `bind-address`**（`allow-lan: false` 下无效） | C.12.2（仅约束监听地址） |
| 7 | `runtime/config.yaml` 原子写（tmp→fsync→rename），位于 Local | C.3 |
| 8 | 停用订阅不参与合并；合并后空 `proxies` → `E_PARSE_EMPTY` | C.6 `subscription_set_enabled`、C.8 |
| 9 | 热更新用 `PUT /configs?force=false|true` + `{"path":"<abs>"}`；并明确其**改不了** `external-controller`/`secret` | 附录 A（`PUT /configs` 语义） |
| 10 | 配置校验失败映射到 `E_CORE_START_FAILED`（未新增错误码） | C.8 |
| 11 | `scope = Global` 时主组名固定 `Proxy`；`Group(name)` 时等于 `name` | C.5 `SwitchScope`、C.6 `switch_apply_best`（`all` 长度） |
| 12 | 本章不涉及 `SwitchPolicy.manual_override_min` / `SwitchRecord.to_delay: Option<u32>` 的**写入**逻辑（在 07 章），但主组结构保证 07 章 `PUT` 能直接命中节点名 | C.5 v1.2、C.6 v1.2 |
| 13 | **（v1.3）** 本章生成的组 `type:` 一律写 YAML 侧小写 `select`；而 07 章经 API 读到的 `kind` 是 `"Selector"`。两套取值不可混用，本章只在 YAML 侧出现 `select`，**不**产出 `"Selector"` 字样 | C.6.1「`kind` 取值陷阱」 |
| 14 | **（v1.3）** `node_key` 同时是 `/proxies/{name}` 的路径参数；`kernel/api.rs` 必须对名字做 `percent_encode`（本章 §4.1 已规定 `name` ≡ `node_key`，故编码责任在 API 层而非合并层） | C.6.1 `SelectorApi::set_group_selected(group, name)` |
| 15 | **（本轮自查修正，已被 v1.4 采纳）** `find-process-mode` 由 `off` 改为 `strict`：`off` 会让机场 `PROCESS-NAME`/`PROCESS-PATH` 规则静默失效（实测 `tunnel.go` 把 `helper.FindProcess` 置 nil）。Lead 已按此修订附录 A.6.1 | C.5 组类型约束的同类原则：正确性优先于省资源；附录 A.6.1（v1.4 已改） |
| 16 | **（本轮自查补充）** §5.6 明确 `tun` 段只在 `proxy_mode = Tun` 生成，并给出 `dns.listen: 0.0.0.0:53` 的**唯一豁免**说明与「同网段不可查」验收要求 | C.12.2「禁止 0.0.0.0」（豁免理由已写明）、附录 A.6.1 `tun` 行 |
| 17 | **（v1.4 对齐）** 主组名 `MAIN_GROUP_DEFAULT = "Proxy"` **已定稿**：附录 A.6.2 模板采用同名，`switch_policy.scope` 两种取值下组数恒为 1，`switch_apply_best.all` 长度恒为 1 | 附录 A.6.2（v1.4 已改）、C.6 `switch_apply_best` |
| 18 | **（v1.4 对齐）** 取消「每订阅一个子组」：`build_proxy_groups` 签名简化为 `(main_group, all_node_keys)`，无副作用地减少了 `PUT /proxies` 的同步面 | Lead v1.4 裁决、附录 A.6.4「只有 select 组」 |

### 已裁决事项存档（v1.4 Lead 确认，均无需再改）

| # | 原未决项 | 裁决 | 本章落地 |
| --- | --- | --- | --- |
| 1 | 主组默认名 `MAIN_GROUP_DEFAULT = "Proxy"` | **已定稿**，附录 A.6.2 已采用同名 | §6.2 标注「已定稿」；常量保留 |
| 2 | 是否每订阅生成子组 | **简化**：只生成主组 `Proxy` | §5.4 示例、§6.1 R-2、§6.2、§6.3 `build_proxy_groups` 全部已改 |
| 3 | 规则条数上限 20 000 | **保留**，标注为「自定保护值，非契约」 | §6.4 保留原值 + 注明性质 |
| 4 | `find-process-mode` / `ipv6` / `dns.enable` 三处取值冲突 | **全部按本章取值**，附录 A.6.1 已修订 | §5.1/§5.2/§5.5 无需改动（本就是本章取值） |
| 5 | 附录 A.6.2 模板含 `url-test` 组 | **已删除**，新增 A.6.4「禁止生成自动策略组」 | §6.1 已加交叉引用；文中多处指向附录 A.6.4 |
| 6 | `scope = Group(name)` 的组不存在时怎么办 | 契约未变；本章增加生成期前置断言 | §6.2「`scope = Group(name)` 的前置校验」 |

> **本轮无新增未决项。** 04 章与 C 章 v1.4、附录 A（v1.4）在组类型、组数量、三处字段取值上已完全一致。
