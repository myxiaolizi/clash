# 附录 B · `subscription-userinfo` 规范与解析实现

> **用途**：机场（订阅提供方）在订阅响应头里下发流量与到期信息的非官方约定。
> 本附录是该字段的**唯一解析依据**，对应代码位置 `src-tauri/src/subscription/userinfo.rs`（C.2）。
> **数据落点**：`UserInfo`（C.5），持久化到 `profiles/<uid>.meta.json`（C.3）。
> **错误码**：缺头且兜底失败 → `E_USERINFO_MISSING`（C.8）。

---

## B.1 头格式规范

```http
HTTP/1.1 200 OK
Content-Type: text/yaml; charset=utf-8
subscription-userinfo: upload=455727941; download=6174315081; total=1073741824000; expire=1798732800
profile-web-page-url: https://example-airport.com/user
```

| 组成 | 规则 |
| --- | --- |
| 键值分隔 | `= ` 两侧可有空格：`upload = 455727941` 合法 |
| 项分隔 | `;`，**尾部可能带 `;`**，中间可能有空项 |
| 值类型 | 十进制整数字符串，**不带单位** |
| 字段名 | 小写；大小写不敏感视为宽容实现，但**只认小写标准名** |
| 未知字段 | 忽略（不得报错，机场会加自定义键） |
| 缺失字段 | 视为 `None`，不得填 `0`（语义完全不同，见 B.3） |
| 编码 | 头值仅 ASCII 数字与符号；出现非 ASCII 说明是畸形头，走「不可解析」分支 |

### B.1.1 字段语义

| 字段 | 类型 | 语义 | 缺失时 |
| --- | --- | --- | --- |
| `upload` | 字节 | 已上传累计 | `UserInfo.upload = None` |
| `download` | 字节 | 已下载累计 | `UserInfo.download = None` |
| `total` | 字节 | 套餐总流量；**`0` 表示不限量/未知** | `None` |
| `expire` | **Unix 秒**（UTC） | 到期时刻；`0` 表示不限时 | `None` |

### B.1.2 相关响应头（可选，增强展示）

| 头 | 用途 | 落点 |
| --- | --- | --- |
| `profile-web-page-url` | 机场用户中心地址，UI 提供「打开管理页」 | `UserInfo.web_page_url` |
| `profile-update-interval` | 机场建议的更新间隔（**小时**） | 只做参考提示，**不覆盖**用户设置的 `update_interval_min` |
| `content-disposition` | 部分机场用它给文件名 | 不使用 |
| `subscription-userinfo` | 本附录主体 | `UserInfo.source = Header` |

---

## B.2 解析算法（可直接照抄实现）

```text
parse_userinfo_header(raw: &str) -> UserInfo
  raw = raw.trim()
  if raw.is_empty() → return UserInfo::unknown()      // source = Unknown

  for part in raw.split(';'):
      part = part.trim()
      if part.is_empty(): continue
      let (k, v) = match part.split_once('='):
          None => continue                            // 无 '=' 的畸形项：跳过，不整体失败
          Some(x) => x
      k = k.trim().to_ascii_lowercase()
      v = v.trim()
      match k.as_str():
          "upload"   => upload   = parse_bytes(v)
          "download" => download = parse_bytes(v)
          "total"    => total    = parse_bytes(v)
          "expire"   => expire   = parse_expire(v)
          _          => continue                       // 未知键：忽略

  // 至少要有一个有效字段，否则视为「头存在但内容不可用」
  if upload.is_none() && download.is_none() && total.is_none() && expire.is_none():
      return UserInfo::unknown()

  source = Header; raw = Some(原始字符串，截断到 512 字节)
```

四条不可动摇的实现要求：

1. **`split_once('=')` 而不是 `split('=')`**：值里即便出现 `=`（畸形）也只取第一段，不 panic、不丢数据。
2. **每个字段独立失败**：`total=abc` 只让 `total` 变 `None`，不影响已经解析出的 `upload`。
3. **绝不 `unwrap()`**：C.13 铁律；全程 `Option`，最终由调用方判定是否 `E_USERINFO_MISSING`。
4. **`raw` 落盘前截断 512 字节并脱敏**：`raw` 只含数字与符号，本身不敏感，但仍要防止畸形超长头把日志/JSON 撑爆。

### B.2.1 `parse_bytes` 伪代码

```rust
fn parse_bytes(v: &str) -> Option<i64> {
    let s = v.trim();
    if s.is_empty() { return None; }
    let n: i64 = s.parse().ok()?;      // 非数字 → None
    if n < 0 { return None; }          // 负数视为畸形
    Some(n)                             // 注意：0 是合法值（total=0 语义见 B.3）
}
```

### B.2.2 `parse_expire` 伪代码（含秒/毫秒判别与越界保护）

```rust
/// 合法区间：2020-01-01 ..= 2100-01-01（Unix 秒）
const EXPIRE_MIN: i64 = 1_577_836_800;
const EXPIRE_MAX: i64 = 4_102_444_800;
const MS_THRESHOLD: i64 = 10_000_000_000; // 1e10：秒制最大约 2286 年，超过必是毫秒

fn parse_expire(v: &str) -> Option<i64> {
    let s = v.trim();
    if s.is_empty() { return None; }
    let mut n: i64 = s.parse().ok()?;                  // 非数字（含 "2026-11-06"）→ None
    if n < 0 { return None; }                          // 负数 → None

    // 1) 13 位（≈1.7e12）判为毫秒 → 降级为秒
    if n >= MS_THRESHOLD { n /= 1000; }

    // 2) expire=0 表示「不限时」，按 None 处理（UI 显示「无限期」）
    if n == 0 { return None; }

    // 3) 越界：明显不是本世纪的到期时间，按不可解析处理
    if n < EXPIRE_MIN || n > EXPIRE_MAX { return None; }

    Some(n)
}
```

> **为什么 `expire` 的 `0` 返回 `None` 而不是 `Some(0)`**：
> `Some(0)` 会被 C.5 的派生规则算成「1970 年到期 / 已过期」，UI 会显示一个荒谬的红色过期卡。
> `total` 的 `0` 则必须保留成 `Some(0)`，因为那是「不限量」的**有效语义**。两者处理不同，别写成对称逻辑。

---

## B.3 `total=0` 与「未知」的区分（最容易做错的一处）

| 场景 | 头内容 | `UserInfo.total` | UI 文案 | 进度条 |
| --- | --- | --- | --- | --- |
| 不限量套餐 | `total=0` | `Some(0)` | 「不限量」 | 不渲染进度条 |
| 机场未下发该键 | 无 `total` | `None` | 「—」 | 不渲染 |
| 解析失败 | `total=abc` | `None` | 「—」 | 不渲染 |
| 有值 | `total=1073741824000` | `Some(1.07e12)` | 「已用 6.18 GB / 1000 GB」 | 正常百分比 |

派生计算（与 C.5 「派生字段规则」一致，前端不得重算）：

| 派生量 | 公式 | 边界 |
| --- | --- | --- |
| 已用 | `upload + download` | 任一为 `None` → 显示 `—`（不要按 0 计） |
| 剩余 | `total - 已用` | `total == Some(0)` → 「不限量」；`total` 为 `None` → `—`；结果为负 → 夹到 `0` 并标记「已超量」 |
| 使用率 | `已用 / total` | `total == Some(0)` 或 `None` → 不计算（避免除零） |
| 剩余天数 | `(expire - now) / 86400` | `expire` 为 `None` → 「无限期」；负数 → 「已过期 N 天」 |

---

## B.4 异常值处理总表

| # | 输入示例 | 判定 | 结果字段 | 后续动作 |
| --- | --- | --- | --- | --- |
| 1 | `expire=1798732800000`（13 位） | 毫秒 | `Some(1798732800)` | 记 Debug 日志 `expire_unit=ms` |
| 2 | `expire=-100` | 负数 | `None` | Warn，不阻断 |
| 3 | `expire=abc` / `expire=2026-11-06` | 非数字 | `None` | Warn |
| 4 | `expire=100` / `expire=99999999999` | 超出 `[2020, 2100]` | `None` | Warn，防止 UI 显示 1970/5138 年 |
| 5 | `expire=0` | 不限时 | `None` | UI「无限期」 |
| 6 | `total=-1` | 负数 | `None` | Warn |
| 7 | `upload= 12 3`（内含空格） | 非数字 | `None` | Warn |
| 8 | `upload=` （空值） | 空串 | `None` | 不记日志（常见） |
| 9 | `;;upload=1;` | 空项 | `upload=Some(1)` | 正常 |
| 10 | `upload=1;foo=2;download=3` | 未知键 | 正常取 upload/download | 忽略 foo |
| 11 | `UPLOAD=1;Download=2` | 大小写混杂 | 正常（实现做 lowercase） | 记 Debug |
| 12 | 整头为空字符串 | 空 | 全 `None`，`source=Unknown` | 走 `E_USERINFO_MISSING` 或网页兜底 |
| 13 | 头存在但四项全畸形 | 不可用 | 全 `None`，`source=Unknown` | 同上 |
| 14 | 值超过 `i64::MAX`（22 位数字） | 溢出 | `None` | 用 `i64::from_str` 的 `Err` 分支，不 panic |
| 15 | 头重复出现两次 | 只用第一个 | 以 `HeaderMap::get` 首值为准 | 记 Debug |

---

## B.5 机场差异样本（6 条真实形态）

> 以下均为**已脱敏**样本（数字为量级示意），用于单测固定输入。真实样本仅允许存在于 `tests/fixtures/`，
> 且必须把域名、用户标识替换为假值。

| # | 机场形态 | 头内容 | 预期解析 |
| --- | --- | --- | --- |
| 1 | 标准 Azure 风格 | `upload=455727941; download=6174315081; total=1073741824000; expire=1798732800` | 四项全 `Some`，`expire` 为 2027-01 前后 |
| 2 | 带尾分号 + 空格 | `upload=0; download=1024; total=0; expire=1798732800;` | `upload=Some(0)`、`total=Some(0)`→「不限量」 |
| 3 | 只给流量不给到期 | `upload=100; download=200; total=1000000` | `expire=None` → 「无限期」 |
| 4 | 毫秒时间戳 | `upload=1024; download=2048; total=102400; expire=1798732800000` | `expire=Some(1798732800)` |
| 5 | 键顺序乱 + 未知键 | `expire=1798732800; plan=pro; total=1099511627776; download=5; upload=7` | 正确取四项，忽略 `plan` |
| 6 | 畸形（网页/中转层改写） | `upload=NA; download=NA; total=NA; expire=NA` | 全 `None` → 走兜底 / `E_USERINFO_MISSING` |

另两条边界样本（放进测试但不算「机场差异」）：

```text
# 7 完全无该头 —— 部分机场只在网页显示
(响应头不含 subscription-userinfo)

# 8 中转 CDN 加了引号 —— 畸形但偶见
subscription-userinfo: "upload=1; download=2; total=3; expire=1798732800"
# 处理：首尾引号在 trim 阶段不会自动去掉，值 `"upload` 无法匹配键 → 全 None。
# 若要宽容处理，去掉成对的 '"' 后再解析（推荐做，成本 3 行）。
```

---

## B.6 Clash Verge Rev 参考实现说明

Clash Verge Rev（`clash-verge-rev`）在其 profile 元信息处理里对 `subscription-userinfo` 的做法，可作参照：

| 维度 | Clash Verge Rev 做法 | 本项目取舍 |
| --- | --- | --- |
| 解析位置 | 拉取 profile 时解析响应头，写入 profile 的 `extra` 元数据 | 同：写入 `profiles/<uid>.meta.json` |
| 字段映射 | `upload` / `download` / `total` / `expire` → 统一放入 `extra` 字典 | 同，但强类型化为 C.5 的 `UserInfo` |
| 缺失处理 | 允许缺字段，UI 侧判空展示 | 同，且用 `Option` 在类型层强制「缺失 ≠ 0」 |
| `total=0` | 视为无限/未知，UI 不显示进度 | 同，保留 `Some(0)` 语义 |
| 到期判定 | 用 `expire` 与当前时间比较算剩余天数，过期高亮 | 同，另加 `notify_on_expire` 通知（C.5） |
| 网页兜底 | 部分版本支持从 `profile-web-page-url` 抓页面解析 | **本项目默认不做网络兜底**：抓网页属额外外连，与「只访问用户配置的地址」边界冲突；仅在用户显式点击「尝试获取流量」时允许，且必须对同一 host 单次请求 |
| 时间单位 | 按秒处理，遇到毫秒值会显示异常年份 | **我们额外做 13 位毫秒判别**（B.2.2），避免同类 bug |

> **参考实现只用来对齐语义，不复制代码**：`clash-verge-rev` 为开源项目，复制其源码需遵循其许可证；
> 本项目按上述伪代码独立实现。这里仅描述行为层面的对应关系。

### B.6.1 网页兜底（可选，默认关闭）

```
触发条件：头缺失 且 用户显式点击「尝试获取流量信息」
前置条件：meta.json 里有 web_page_url，且 host 与订阅 host 相关（同名或同 TLD 校验）
限制：只 GET 一次 / 5s 超时 / 不携带任何 cookie 与凭据 / 响应上限 512 KB
解析：正则 upload|download|total|expire 附近的「数字 + 单位」→ 单位换算成字节
失败：E_USERINFO_MISSING，source 保持 Unknown
成功：source = WebPage
风险：页面结构千变万化、部分是登录后可见、可能触发机场风控 → 因此**不做自动兜底**
```

---

## B.7 单测用例表（15 行，直接照此写 `#[test]`）

> 文件位置：`src-tauri/src/subscription/userinfo.rs` 内 `#[cfg(test)] mod tests`。
> 断言只针对 `UserInfo` 的四个数值字段 + `source`，不依赖当前时间。

| # | 用例名 | 输入 `raw` | upload | download | total | expire | source |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | `std_full` | `upload=455727941; download=6174315081; total=1073741824000; expire=1798732800` | 455727941 | 6174315081 | 1073741824000 | 1798732800 | Header |
| 2 | `trailing_semicolon` | `upload=0; download=1024; total=0; expire=1798732800;` | 0 | 1024 | 0 | 1798732800 | Header |
| 3 | `no_expire` | `upload=100; download=200; total=1000000` | 100 | 200 | 1000000 | None | Header |
| 4 | `expire_millis` | `upload=1; download=2; total=3; expire=1798732800000` | 1 | 2 | 3 | 1798732800 | Header |
| 5 | `expire_zero_means_unlimited` | `upload=1; download=2; total=3; expire=0` | 1 | 2 | 3 | None | Header |
| 6 | `expire_negative` | `upload=1; download=2; total=3; expire=-1` | 1 | 2 | 3 | None | Header |
| 7 | `expire_non_numeric` | `expire=abc` | None | None | None | None | Unknown |
| 8 | `expire_date_string` | `expire=2026-11-06` | None | None | None | None | Unknown |
| 9 | `unknown_keys_ignored` | `plan=pro; upload=7; download=5; total=1099511627776; expire=1798732800` | 7 | 5 | 1099511627776 | 1798732800 | Header |
| 10 | `empty_items` | `;;upload=1;;download=2;;` | 1 | 2 | None | None | Header |
| 11 | `malformed_no_equals` | `upload1; download2; total3` | None | None | None | None | Unknown |
| 12 | `empty_string` | `` （空串） | None | None | None | None | Unknown |
| 13 | `negative_total` | `total=-1; upload=5` | 5 | None | None | None | Header |
| 14 | `uppercase_keys` | `UPLOAD=1; Download=2; TOTAL=3; EXPIRE=1798732800` | 1 | 2 | 3 | 1798732800 | Header |
| 15 | `overflow_value` | `upload=99999999999999999999999; download=1` | None | 1 | None | None | Header |

补充两个纯函数级用例（不是头解析，但必须一起写）：

| # | 用例名 | 输入 | 期望 |
| --- | --- | --- | --- |
| 16 | `derived_used_when_partial` | `upload=None, download=100` | 已用显示 `—`（不得算成 100） |
| 17 | `derived_remaining_clamped` | `total=100, upload=80, download=50` | 剩余夹到 `0`，标记超量 |

---

## B.8 脱敏与不落明文要求

| 项 | 要求 |
| --- | --- |
| `raw` 字段 | 只含数字与 `;= `，本身不含凭据；仍截断到 512 字节后落盘（防畸形超长头） |
| 日志 | 解析结果落日志时**只输出计数级信息**：`INFO [subscription] userinfo uid=a1b2c3d4 source=header has_total=true has_expire=true`，禁止整行打印 `raw` |
| Debug 级别 | 允许打印各项数值（字节数、时间戳），**绝不打印订阅 URL** |
| 网页兜底 | 抓回页面后**不落盘原文**；只落解析后的数值与 `source=WebPage`；URL 记 host 级即可 |
| 诊断包 | `diagnostics_export` 只含 `UserInfo` 数值字段与 `source`，不含 `raw`、不含 `web_page_url`（含 token 的可能） |
| UI | 流量数字可展示；「打开管理页」只显示 host + path 前缀，token 段打码 |
| 单测 fixture | `tests/fixtures/userinfo/*.txt` 中的 `expire` 用固定值，域名统一 `example.invalid` |
| 禁止 | 把 `subscription-userinfo` 原值、`profile-web-page-url` 完整值写进 `last_error`（C.8：`message/detail` 不得含 URL 与 token） |

---

## B.9 与 IPC / 事件的接口位置（不新增契约）

| 场景 | 走的现成契约 | 说明 |
| --- | --- | --- |
| 前端首屏拿流量 | `app_bootstrap` → `subscriptions[].mgmt` | C.6，无需新命令 |
| 单订阅刷新生效 | 事件 `subscription:updated` → 负载含 `mgmt` | C.7 |
| 多订阅聚合 | `traffic_summary` → `TrafficSummary` | C.6；聚合规则：各订阅分别展示 + 总计卡按「有 total 的订阅之和」 |
| 诊断 | `diagnostics_export` | 只含数值字段 |
| 错误 | `E_USERINFO_MISSING` | 文案：「该订阅未提供流量信息」 |

> **不要新增命令**：本附录能力全部能由 C.6 现有命令承载。若确实需要「单独重新获取流量」按钮，
> 复用 `subscription_refresh`（`{uid}`）+ 事件 `subscription:updated`，不要发明 `userinfo_refresh`。

---

## 附录 B 自检清单

> 本清单用于验收 `subscription/userinfo.rs` 的实现与本文一致性。逐条打勾，缺一不可。

### 解析正确性

- [ ] 用 `split_once('=')` 而非 `split('=')`；畸形项跳过，不整体失败
- [ ] 四个字段**各自独立** `Option`，一个坏了不影响其它三个
- [ ] `upload`/`download`/`total`：负数 → `None`；`0` → `Some(0)`（`total=0` 是「不限量」有效语义）
- [ ] `expire`：`0` → `None`（不是 `Some(0)`）；负数 → `None`；13 位 → ÷1000；越界 `[1577836800, 4102444800]` → `None`
- [ ] 未知键（如 `plan=pro`）被忽略，不报错也不影响已解析字段
- [ ] 尾部分号、空项（`;;`）、键大小写混杂均正确容忍
- [ ] 值溢出 `i64`（22 位数字）走 `Err` 分支返回 `None`，**不 panic**
- [ ] 全程无 `unwrap()` / `expect()`（C.13）
- [ ] `raw` 落盘前截断到 512 字节
- [ ] 四项全缺失或全畸形 → `source = Unknown`，由调用方决定是否 `E_USERINFO_MISSING`

### 单测覆盖（附录 B.7 的 15 行 + 2 行派生 = 17）

- [ ] 15 个头解析用例全部写成 `#[test]`，名称与 B.7 表格一致
- [ ] 2 个派生用例（`derived_used_when_partial`、`derived_remaining_clamped`）已写
- [ ] fixture 域名统一 `example.invalid`，`expire` 用固定值（不依赖当前时间）
- [ ] `cargo test --lib subscription::userinfo` 全绿

### 展示与聚合

- [ ] UI 区分「不限量」（`total=Some(0)`）与「未知」（`total=None`）两种文案
- [ ] 已用 = `upload + download`，任一缺失显示 `—`（不得按 0 计）
- [ ] 剩余为负时夹到 0 并标「已超量」
- [ ] `expire=None` 显示「无限期」，负数显示「已过期 N 天」
- [ ] `total=Some(0)` 或 `None` 时不计算使用率（不除零）
- [ ] 单位 1024 进制、2 位小数
- [ ] `traffic_summary` 的 `TrafficSummary` 聚合：`total=0` 的订阅被忽略、`expire_at` 取最早、`has_missing` 正确置位

### 安全与脱敏

- [ ] 日志只输出 `uid / source / has_total / has_expire`，**不打印 `raw` 整行**
- [ ] Debug 级别也不打印订阅 URL
- [ ] `diagnostics_export` 不含 `raw` 与 `web_page_url`
- [ ] `last_error` 不含 URL/token（C.8）
- [ ] 网页兜底默认关闭；开启时只 GET 一次、5s 超时、不携带 cookie、响应上限 512 KB
- [ ] 兜底抓回的页面**不落盘原文**

### 契约一致性

- [ ] 只使用 C.6 现有命令（`subscription_refresh` 等），无 `userinfo_refresh` 之类自创命令
- [ ] 字段名与 C.5 `UserInfo` 完全一致（`expire_at` 而非 `expire`、`source` 为 `UserInfoSource`）
- [ ] 错误码只用 `E_USERINFO_MISSING`；缺失头**不是**错误（除非兜底也失败）

---

## 契约对齐记录（v1.3）

| 契约条目 | 本附录落点 | 状态 |
| --- | --- | --- |
| v1.1 · `TrafficSummary` / `SubscriptionTraffic` / `AggregateTraffic` | B.3 派生表 + B.9 接口位置；聚合规则与 C.5 注释一致 | ✅ 已落地 |
| v1.1 · `AppSettings.compliance_accepted_at`（schema v2） | 不在本附录范围（13 章 P5 / 12 章 P5 验收覆盖） | — |
| v1.2 · `SwitchRecord.to_delay: Option<u32>` | 不在本附录范围（用户流量无关，12 章异常清单 #28 覆盖） | — |
| v1.3 · §C.6.1 内部接口冻结面（5 类型） | 不在本附录范围（`kernel/api.rs` / `selector.rs`，附录 A 覆盖） | — |
| `subscription-userinfo` 解析算法 | B.2 / B.2.1 / B.2.2 伪代码 | ✅ 权威定义 |
| 单测用例表 15 行 | B.7 | ✅ 已落地 |

**未决项**：无。本附录不依赖 v1.2 / v1.3 的任何新增类型。
