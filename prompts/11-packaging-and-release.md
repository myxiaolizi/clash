# 11 · 打包、分发与发布（Packaging & Release）

> **适用阶段**：P7（可在 P1 就搭好骨架，避免后期返工）
> **前置阅读**：`01-master-prompt.md`、`appendices/C-conventions.md`（下称 C 章）、
> `08-kernel-and-system-proxy.md`、`10-lightweight-and-performance.md`
> **本章职责**：Tauri 2 NSIS 安装包配置、图标与版本、WebView2 分发、代码签名取舍、
> 内核分发策略与 GPL-3.0 合规、About 页、卸载清理、GitHub Actions CI、发布检查清单与回滚。
> **本章产出可验收物**：`ClashZhengl_<version>_x64-setup.exe`（≤12 MB，不含内核与 WebView2）。

---

## 11.0 交付物一览

| 产物 | 路径 | 体积目标 | 说明 |
| --- | --- | --- | --- |
| 主程序 exe | `src-tauri/target/release/clash-zhengl.exe` | ≤ 8 MB | C.1 可执行文件名 |
| NSIS 安装包 | `ClashZhengl_<ver>_x64-setup.exe` | **≤ 12 MB** | C.1 命名，C.11 预算 |
| 安装包签名文件 | `.exe` 的 Authenticode 签名（可选） | — | 见 11.6 |
| 许可证清单 | `LICENSE`、`THIRD-PARTY-NOTICES.md` | — | GPL 合规，见 11.7 |
| 内核（可选独立分发） | 运行时下载，不进安装包 | — | 推荐策略 A |
| SHA256 校验文件 | `SHA256SUMS.txt` | — | Release 附件 |

> **C.11 约束复核**（v1.1 口径）：12 MB 预算**不含**内核与 WebView2，且计量对象是
> **签名后的** `ClashZhengl_<ver>_x64-setup.exe`（`Get-Item .Length`）。捆绑内核会使安装包
> 达到 25~30 MB，直接违反预算 —— 这是选策略 A（首启下载）的第二个理由（第一个是 GPL）。
>
> **过渡仲裁**（C.11 v1.1）：实测落在 12 MB 的 100%–120%（即 12–14.4 MB）时允许保留，
> 但必须记入 `docs/perf-log.md` 并在下一阶段给出改进项；**超过 14.4 MB 视为不达标，
> 必须立即处置**（通常是某个依赖的 `default-features` 没关）。CI 只在 >12 MB 时告警、
> >14.4 MB 时 fail，见 11.9。

---

## 11.1 `tauri.conf.json` 完整片段

```jsonc
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "Clash Zhengl",
  "identifier": "com.clashzhengl.app",
  "version": "0.1.0",
  "build": {
    "frontendDist": "../dist",
    "devUrl": "http://localhost:5173",
    "beforeDevCommand": "pnpm dev",
    "beforeBuildCommand": "pnpm build"
  },
  "app": {
    "windows": [
      {
        "label": "main",
        "title": "Clash Zhengl",
        "width": 880,
        "height": 620,
        "minWidth": 720,
        "minHeight": 520,
        "resizable": true,
        "fullscreen": false,
        "center": true,
        "visible": false,
        "decorations": false,
        "transparent": false,
        "shadow": true,
        "skipTaskbar": false,
        "dragDropEnabled": false
      }
    ],
    "trayIcon": {
      "id": "main",
      "iconPath": "icons/tray-32.png",
      "iconAsTemplate": false
    },
    "security": {
      "csp": "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self' ipc: http://ipc.localhost"
    }
  },
  "bundle": {
    "active": true,
    "targets": ["nsis"],
    "icon": [
      "icons/32x32.png",
      "icons/128x128.png",
      "icons/128x128@2x.png",
      "icons/icon.ico"
    ],
    "publisher": "Clash Zhengl",
    "copyright": "Copyright © 2026 Clash Zhengl contributors",
    "category": "Utility",
    "shortDescription": "轻量 Clash 客户端",
    "longDescription": "Clash Zhengl 是一个 Windows x64 轻量代理客户端：管理多个订阅、展示流量与到期日、定时自动更新、自动测速并切换到最低延迟节点。本软件不提供任何节点或代理服务。",
    "licenseFile": "../LICENSE",
    "resources": [
      "../THIRD-PARTY-NOTICES.md",
      "../licenses/mihomo-LICENSE.txt",
      "../licenses/README-KERNEL.txt"
    ],
    "externalBin": [],
    "windows": {
      "webviewInstallMode": {
        "type": "downloadBootstrapper",
        "silent": true
      },
      "nsis": {
        "installMode": "currentUser",
        "installerIcon": "icons/icon.ico",
        "languages": ["SimpChinese", "English"],
        "displayLanguageSelector": false,
        "compression": "lzma",
        "startMenuFolder": "Clash Zhengl",
        "minimumWebview2Version": "109.0.1518.0"
      },
      "allowDowngrades": false
    }
  }
}
```

### 11.1.1 字段速查与取舍

| 字段 | 取值 | 理由 |
| --- | --- | --- |
| `identifier` | `com.clashzhengl.app` | **C.1 硬契约**，改动会让已装用户成为"另一个应用" |
| `version` | `0.1.0` | C.1 SemVer，首版 0.1.0；与 `Cargo.toml` `package.version` 必须一致 |
| `bundle.targets` | `["nsis"]` | 只出 NSIS；**不出 MSI**（无需企业部署，且 MSI 需额外签名配置） |
| `windows.nsis.installMode` | `currentUser` | 免 UAC，符合 C.12.4「默认非管理员运行」；内核装到 `%LOCALAPPDATA%` 也不需要管理员 |
| `windows.nsis.compression` | `lzma` | 最省体积；解压慢 1~2s 可接受（一次性） |
| `webviewInstallMode.type` | `downloadBootstrapper` | Win10 1809 可能无 WebView2 → 引导下载（见 11.5） |
| `windows.nsis.minimumWebview2Version` | `109.0.1518.0` | Win7/8 最后支持版本；低于此版本前端可能白屏 |
| `bundle.externalBin` | `[]` | **不捆绑内核 sidecar**（策略 A）；若走策略 B 见 11.7.3 |
| `bundle.resources` | 4 个文件 | 许可证与内核说明必须随包分发（GPL 合规） |
| `app.security.csp` | 见上 | 严格 CSP；`connect-src 'self' ipc:` 是 Tauri IPC 必需 |
| `visible: false` | `false` | 先建窗再显示，避免白屏闪烁（前端 ready 后 `show()`） |
| `dragDropEnabled: false` | `false` | 不需要文件拖入，关掉可减少 WebView2 事件开销 |
| `decorations: false` | `false` | 09 章自绘标题栏需要 |

> `$schema` 指向 `schema.tauri.app/config/2`；实际构建以 `Cargo.lock` 中 `tauri` 版本为准
> （Tauri 2.x 的 bundle 字段有 `windows.nsis` / `windows.webviewInstallMode` 两级，
> 混用 1.x 的 `nsis` 顶层字段会报 schema 错误）。

### 11.1.2 `Cargo.toml` release profile（与 10 章对齐）

```toml
[profile.release]
opt-level = "z"       # 体积优先；代理客户端非计算密集
lto = true            # 全量 LTO
codegen-units = 1
strip = true          # 去掉符号，省 2~4 MB
panic = "abort"       # 不展开栈；08.8 的退出逻辑不依赖 unwind
incremental = false
```

> `panic = "abort"` 的代价：`catch_unwind` 失效。08.10 的"panic hook 恢复系统代理"仍可用
> （hook 在 abort 之前执行），但**禁止**在业务代码里依赖 `catch_unwind`。

### 11.1.3 版本号三处同步（防漂移）

| 位置 | 字段 | 检查方式 |
| --- | --- | --- |
| `src-tauri/Cargo.toml` | `[package] version` | 真源 |
| `src-tauri/tauri.conf.json` | `version` | CI 脚本比对 |
| `package.json` | `version` | CI 脚本比对（仅记录用途） |

```powershell
# scripts/check-version.ps1 —— CI 与本地 preflight 共用
$cargo = (Select-String -Path src-tauri/Cargo.toml -Pattern '^version\s*=\s*"(.+)"').Matches[0].Groups[1].Value
$conf  = (Get-Content src-tauri/tauri.conf.json -Raw | ConvertFrom-Json).version
$pkg   = (Get-Content package.json -Raw | ConvertFrom-Json).version
if ($cargo -ne $conf -or $cargo -ne $pkg) {
  throw "版本号不一致: cargo=$cargo conf=$conf pkg=$pkg"
}
Write-Host "version ok: $cargo"
```

---

## 11.2 图标清单

`src-tauri/icons/` 必须包含（Tauri 2 构建会校验存在性）：

| 文件 | 尺寸 | 用途 | 必需 |
| --- | --- | --- | --- |
| `icon.ico` | **多尺寸 16/24/32/48/64/128/256** | 安装包、exe 图标、任务栏 | ✅ |
| `32x32.png` | 32×32 | 窗口图标 | ✅ |
| `128x128.png` | 128×128 | 关于页、商店 | ✅ |
| `128x128@2x.png` | 256×256 | 高 DPI | ✅ |
| `tray-32.png` | 32×32（**纯白/纯黑单色 + 透明**） | 托盘（深色任务栏自动反色由系统处理，我们提供浅色版） | ✅ |
| `tray-err.png` | 32×32（带红色角标） | 内核 Failed 状态 | ✅ |
| `icon.png` | 512×512 | 源图（不参与打包） | 建议 |

**生成命令**（一次性，产物入库）：

```powershell
# 需要一张 1024x1024 的 icon.png 源图
pnpm tauri icon .\assets\icon.png
# 产物：src-tauri/icons/ 下自动生成全套 png + ico
```

| 图标设计要求 | 说明 |
| --- | --- |
| 小尺寸可辨识 | 16×16 下必须能看出轮廓；避免细线 |
| 无文字 | 中文字形在 16px 下必糊 |
| 对比度 | 深色任务栏与浅色任务栏都能看见（外描边 1px） |
| 托盘单色 | 不依赖彩色，避免在 Win11 圆角任务栏里显得脏 |
| 免版权争议 | 原创或 MIT/Apache 授权素材，记录来源于 `THIRD-PARTY-NOTICES.md` |

---

## 11.3 NSIS 安装包：安装目录、注册表与卸载清理

### 11.3.1 安装后的目录布局

```
%LOCALAPPDATA%\Programs\Clash Zhengl\        ← currentUser 模式安装目录
├─ clash-zhengl.exe
├─ LICENSE
├─ THIRD-PARTY-NOTICES.md
├─ licenses\
│  ├─ mihomo-LICENSE.txt
│  └─ README-KERNEL.txt
└─ （WebView2 由系统级安装，不在此处）

%APPDATA%\ClashZhengl\                       ← 用户配置（C.3，Roaming）
├─ settings.json
├─ profiles\
└─ ...

%LOCALAPPDATA%\ClashZhengl\                  ← 运行时（C.3，Local）
├─ runtime\  (config.yaml / secret / core\<ver>\ / sysproxy.journal.json)
├─ logs\
└─ diagnostics\
```

### 11.3.2 安装 / 升级 / 卸载钩子

```nsi
; src-tauri/installer-hooks.nsh（Tauri NSIS 支持 custom installer hooks）
; ---- 安装前：结束正在运行的实例，避免文件占用 ----
!macro NSIS_HOOK_PREINSTALL
  nsExec::Exec 'taskkill /IM clash-zhengl.exe /F'
  Sleep 800
!macroend

; ---- 安装后：不自动启动（用户主动） ----
!macro NSIS_HOOK_POSTINSTALL
  ; 清理旧版本可能残留的 v1 命名（若曾用别的 productName 发过版）
  Delete "$INSTDIR\clash-zhengl.exe.old"
!macroend

; ---- 卸载前：结束进程 ----
!macro NSIS_HOOK_PREUNINSTALL
  nsExec::Exec 'taskkill /IM clash-zhengl.exe /F'
  Sleep 800
!macroend

; ---- 卸载后：询问是否保留配置（C.12.7：默认保留） ----
!macro NSIS_HOOK_POSTUNINSTALL
  IfFileExists "$LOCALAPPDATA\ClashZhengl\*.*" 0 done_data
    MessageBox MB_YESNO|MB_ICONQUESTION \
      "是否删除本地缓存、日志与内核文件？（订阅与设置会保留）$\r$\n$\r$\n选择「否」可保留全部数据，下次安装自动沿用。" \
      IDYES del_local IDNO done_data
  del_local:
    RMDir /r "$LOCALAPPDATA\ClashZhengl"
    Goto done_data
  done_data:

  IfFileExists "$APPDATA\ClashZhengl\*.*" 0 done_all
    MessageBox MB_YESNO|MB_ICONWARNING \
      "是否同时删除所有订阅与设置？（含加密的订阅地址，删除后不可恢复）" \
      IDYES del_roam IDNO done_all
  del_roam:
    RMDir /r "$APPDATA\ClashZhengl"
    Goto done_all
  done_all:

  ; 自启项兜底清理（正常情况下应用退出时会自己删）
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "ClashZhengl"
!macroend
```

```jsonc
// tauri.conf.json 的 bundle.windows.nsis 追加：
"nsis": {
  "installerHooks": "./installer-hooks.nsh",
  // ... 其余同上
}
```

| 卸载行为 | 默认 | 说明 |
| --- | --- | --- |
| 删除程序目录 | ✅ 自动 | NSIS 负责 |
| 删除 `%LOCALAPPDATA%\ClashZhengl` | ❓ 询问（默认"否"） | 含内核（可重新下载）与日志 |
| 删除 `%APPDATA%\ClashZhengl` | ❓ 询问（默认"否"） | 含加密订阅，误删代价大 |
| 清理 HKCU Run 自启项 | ✅ 自动 | 兜底，即使应用崩溃也清 |
| 恢复系统代理 | ✅ 卸载前由应用自身退出流程处理（08.8） | 若进程已被 taskkill，下一次启动新版本时 journal 守卫兜底；若彻底卸载，**必须先运行一次应用或手工检查** |

> ⚠️ **卸载残留代理的风险**：如果用户在"系统代理开启"状态下被 taskkill 然后直接卸载，
> 代理会残留在注册表里指向死端口。**缓解**：`NSIS_HOOK_PREUNINSTALL` 中先尝试优雅退出
> （`nsExec::Exec '"$INSTDIR\clash-zhengl.exe" --shutdown-proxy'`），该模式只做"恢复代理并退出"，
> 不启动界面、不加载内核，耗时 <300ms。

### 11.3.3 安装向导页面

| 页面 | 内容 | 是否保留 |
| --- | --- | --- |
| 欢迎 | 常规 | ✅ |
| 许可协议 | `LICENSE` + GPL 声明摘要 | ✅ 必须 |
| 安装位置 | 默认 `%LOCALAPPDATA%\Programs\Clash Zhengl` | ✅ |
| 开始菜单 | 创建快捷方式 | ✅ |
| 完成 | 「运行 Clash Zhengl」勾选框 | ✅ |
| **语言选择** | `displayLanguageSelector: false` | ❌ 关掉（只做中文优先，减少一个页面） |

---

## 11.4 首次启动合规提示（C.12.6）

首次启动（`settings.json` 不存在）显示**一次**不可跳过的模态：

```
┌─ 使用前请阅读 ─────────────────────────────────┐
│ 1. 本软件不提供任何节点、订阅或代理服务。          │
│ 2. 你需要自备来自合法服务商的订阅地址。            │
│ 3. 订阅地址含你的专属 token，仅以 Windows DPAPI   │
│    加密保存在本机，不会上传到任何服务器。          │
│ 4. 请遵守你所在地区的法律法规与服务商条款。        │
│                                                 │
│ ☐ 我已阅读并理解                                │
│                                    [继续]       │
└─────────────────────────────────────────────────┘
```

| 要求 | 说明 |
| --- | --- |
| 触发条件 | `settings.compliance_accepted_at == null`（C.5），即从未接受过 |
| 记录 | `settings_save` 写入 `complianceAcceptedAt = now()`（Unix 秒，C.10） |
| 不可绕过 | 「继续」按钮在勾选前 disabled |
| 不重复 | 该字段非 `null` 即不再显示 |
| schema 兼容 | 该字段随 C.5 引入于 `schema_version = 2`；v1 → v2 迁移时旧用户该字段为 `null`，会弹一次 |
| 不可回滚 | 用户清空 `%APPDATA%\ClashZhengl` 后视为未接受，下次启动再弹（符合 C.12.6） |

---

## 11.5 WebView2 分发策略

### 11.5.1 各 Windows 版本的 WebView2 现状

| 系统版本 | WebView2 预装情况 | 处置 |
| --- | --- | --- |
| Windows 11（全部） | ✅ 随系统 | 无需处理 |
| Windows 10 21H2+ | 多数已由 Edge 更新带入 | 通常无需处理 |
| **Windows 10 1809~2004** | ⚠️ 可能没有 | 需 bootstrapper |
| Windows 10 LTSC 2019 | ⚠️ 通常没有（无 Edge） | 需 bootstrapper |
| Windows 7/8.1 | ❌ 不支持本产品（C.1 目标 1809+） | 安装器直接拒绝 |

### 11.5.2 三种模式对比

| 模式 | `webviewInstallMode.type` | 安装包增量 | 需要联网 | 适用 |
| --- | --- | --- | --- | --- |
| **下载引导**（推荐） | `downloadBootstrapper` | ≈ 0 | ✅ 首次安装时 | 绝大多数用户 |
| 离线安装包 | `offlineInstaller` | **+120 MB** | ❌ | 内网/离线分发 |
| 固定版本运行时 | `fixedRuntime` | +150 MB | ❌ | 需要锁定 WebView2 版本 |
| 跳过 | `skip` | 0 | — | 仅当确认目标机器已有 |

**决策：`downloadBootstrapper` + `minimumWebview2Version: "109.0.1518.0"`。**

| 理由 | 说明 |
| --- | --- |
| 体积 | offline 会把 12 MB 预算炸到 130 MB，直接违约 |
| 覆盖率 | 1809+ 用户绝大多数已有 WebView2（Edge 自带） |
| 失败兜底 | bootstrapper 失败时 NSIS 会显示明确错误；我们在安装后首次启动检测 `webview2` 可用性并给出中文指引 |

### 11.5.3 兜底提示（应用内检测）

```rust
// main.rs 启动最早期：仅在 WebView 创建失败时走到
fn webview_error_hint() -> &'static str {
    "无法启动界面组件（WebView2）。\n\
     请安装 Microsoft Edge WebView2 Runtime 后重试：\n\
     https://developer.microsoft.com/microsoft-edge/webview2/\n\
     若已安装仍失败，请尝试修复 Microsoft Edge。"
}
```

| 场景 | 表现 | 处理 |
| --- | --- | --- |
| WebView2 缺失 | 窗口空白/创建失败 | 弹原生 `MessageBoxW` 显示上述文案（**不能用 WebView 显示**） |
| WebView2 过旧 | 前端 JS 语法错误 | `minimumWebview2Version` 已挡在安装阶段；运行时再检测一次 |
| 企业策略禁用 | 创建失败 | 同上，加一句「请联系管理员」 |

> **禁止**：为兼容旧 WebView2 而降低 TS 目标到 ES5（会让产物膨胀 30%+）。
> 用 `target: "es2020"`，并在安装阶段挡住旧版本。

---

## 11.6 代码签名取舍

### 11.6.1 三种状态对比

| 方案 | 成本 | SmartScreen 表现 | 用户信任 | 建议 |
| --- | --- | --- | --- | --- |
| **不签名** | 0 | ⚠️ 首次下载运行弹「Windows 已保护你的电脑」，需点「更多信息 → 仍要运行」 | 低 | 0.1.x 草创期可用 |
| **OV 代码签名证书** | ~$200–400/年 | 需累积声誉，初期仍可能提示；几周后转好 | 中 | 有预算后首选 |
| **EV 代码签名证书** | ~$400–700/年 + 硬件 token | 立即获得 SmartScreen 信誉 | 高 | 正式商业化阶段 |
| 自签名证书 | 0 | ❌ 更糟（未知发布者），且无法通过时间戳 | 极低 | **仅内测** |

### 11.6.2 不签名期间的缓解措施

| 措施 | 实现 |
| --- | --- |
| 文档明示 | README 与 Release Notes 写清「未签名，首次运行需点『更多信息』」并附截图 |
| 提供 SHA256 | 每个 Release 附带 `SHA256SUMS.txt`，README 教用户核对 |
| 发布渠道单一 | 只在 GitHub Releases 发布，不接受第三方转载 |
| 提供自签脚本 | `scripts/sign-local.ps1` 供企业内网自建信任链（见下） |

### 11.6.3 自签方案（企业内测）

```powershell
# scripts/sign-local.ps1 —— 生成自签证书并签名（仅内测，勿公开发布用此证书产物）
param([string]$Exe = "src-tauri\target\release\clash-zhengl.exe")

$cert = New-SelfSignedCertificate `
  -Type CodeSigningCert `
  -Subject "CN=Clash Zhengl Internal" `
  -KeyUsage DigitalSignature `
  -FriendlyName "Clash Zhengl Internal Code Signing" `
  -CertStoreLocation "Cert:\CurrentUser\My" `
  -NotAfter (Get-Date).AddYears(3)

# 导出 .cer 供测试机导入「受信任的根证书颁发机构」
Export-Certificate -Cert $cert -FilePath ".\clash-zhengl-internal.cer" | Out-Null

$signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" |
            Sort-Object FullName -Descending | Select-Object -First 1
& $signtool.FullName sign /fd SHA256 /a /f $null /sha1 $cert.Thumbprint $Exe
```

### 11.6.4 正式签名的 CI 集成（有证书后）

```yaml
# 关键片段：签名在 tauri build 之后、上传之前
- name: Import certificate
  if: env.SIGNING_CERT_B64 != ''
  shell: pwsh
  run: |
    $bytes = [Convert]::FromBase64String($env:SIGNING_CERT_B64)
    [IO.File]::WriteAllBytes("$env:RUNNER_TEMP\cert.pfx", $bytes)
    Import-PfxCertificate -FilePath "$env:RUNNER_TEMP\cert.pfx" `
      -CertStoreLocation Cert:\CurrentUser\My -Password (ConvertTo-SecureString $env:SIGNING_CERT_PASS -AsPlainText -Force)
```

> Tauri 2 支持在 `bundle.windows` 下配置 `certificateThumbprint` / `digestAlgorithm` /
> `timestampUrl`；配置后 `tauri build` 会自动调 `signtool` 签名**所有**产物。
> 未配置时跳过签名，构建不失败。

---

## 11.7 内核分发策略与 GPL-3.0 合规

### 11.7.1 两策略对比（决策表）

| 维度 | **策略 A：首启下载**（推荐） | **策略 B：捆绑进安装包** |
| --- | --- | --- |
| 安装包体积 | 11 MB 左右（符合 C.11 ≤12 MB） | 25~30 MB（**违反 C.11**） |
| 首次可用耗时 | 需下载 10~16 MB，10~40s（视网络） | 立即 |
| 离线可用 | ❌ 需要网络一次 | ✅ |
| GPL-3.0 传染风险 | **低**（不在同一发行物中分发） | **高**（需完整履行 GPL 义务） |
| 升级内核 | 无需重发客户端 | 需重发整个安装包 |
| 上游安全补丁 | 用户可自选新版本，响应快 | 需重新构建发布 |
| 实现复杂度 | 中（08.2 全部逻辑） | 低（`externalBin`） |
| CI 是否需外网 | ❌ 不需要（运行时下载） | ✅ 需要（构建期拉内核，违反 00-README §4.5） |

**决策：策略 A。** 三条硬理由：C.11 包体预算、C.12.5 推荐项、00-README §4.5「构建阶段不得依赖外网」。

### 11.7.2 GPL-3.0 的边界（必须准确表述，不许含糊）

| 问题 | 结论 | 依据 |
| --- | --- | --- |
| mihomo 的许可证 | **GPL-3.0** | mihomo 仓库 LICENSE |
| 通过子进程 + REST + 命令行参数调用，是否构成"衍生作品" | 一般**不**构成（进程边界 + 标准 IPC）。这是业内普遍做法（Clash Verge Rev 同理） | FSF 关于"独立进程通过标准机制通信"的说明 |
| 但**分发** mihomo 二进制时 | **必须**遵守 GPL-3.0：随附 LICENSE、提供对应源码（或书面提供源码获取方式） | GPL-3.0 §4、§6 |
| 策略 A 下 | 内核由**用户自己从官方 Release 下载**，我们只是自动化了这个动作 → 不是我们在分发 | — |
| 策略 A 仍需做 | 在 UI 与 About 页声明内核为 mihomo、GPL-3.0、来源链接（诚实披露） | 合规最佳实践 |
| 绝对不能做 | 修改 mihomo 源码（01 §8）、静态链接进主程序、声称内核是自己写的 | — |

### 11.7.3 若将来必须走策略 B（`externalBin` 具体写法）

```jsonc
// tauri.conf.json
"bundle": {
  "externalBin": ["binaries/mihomo"],   // 实际文件名 mihomo-x86_64-pc-windows-msvc.exe
  "resources": [
    "licenses/mihomo-LICENSE.txt",
    "licenses/mihomo-SOURCE-OFFER.txt",  // 源码获取方式（版本号 + 仓库链接 + 归档 sha256）
    "THIRD-PARTY-NOTICES.md"
  ]
}
```

```rust
// 运行时定位 sidecar（Tauri 2）
let sidecar = tauri::process::current_binary(&app.env())?  // 或 app.path().resource_dir()
    .parent().unwrap().join("mihomo.exe");
```

**捆绑时必须同时做到**（缺一即违约）：

1. 安装目录含 `licenses/mihomo-LICENSE.txt`（GPL-3.0 全文）；
2. `THIRD-PARTY-NOTICES.md` 写明：内核名、版本号、许可证、源码仓库 URL、索取源码的方式；
3. About 页可见同样信息；
4. 若曾对内核做任何修改（**本产品不允许**），必须公开修改后的源码；
5. 不使用"GPL 传染"以外的误导表述（例如不能写"本软件为 MIT 且包含 GPL 组件"而不解释）。

### 11.7.4 `licenses/README-KERNEL.txt` 内容（策略 A 实际随包分发）

```text
关于 mihomo 内核
================

Clash Zhengl（下称"本软件"）本身不包含、不修改、不静态链接 mihomo 内核。

本软件通过以下方式使用 mihomo：
  1. 在用户明确同意后，从 MetaCubeX/mihomo 的 GitHub Releases 下载官方发布的
     Windows amd64 二进制压缩包；
  2. 以独立的子进程方式启动该二进制，通过命令行参数与本地回环 HTTP API 通信；
  3. 不修改其源码，不重新分发其二进制。

mihomo 项目：
  仓库   : https://github.com/MetaCubeX/mihomo
  许可证 : GNU General Public License v3.0
  许可证全文见同目录 mihomo-LICENSE.txt（或上述仓库的 LICENSE 文件）

内核的许可证义务由 mihomo 项目与其上游承担；本软件仅作为调用方。
如需获取 mihomo 对应版本的完整源码，请访问上述仓库的 Releases 页面，
下载对应 tag 的 Source code (zip/tar.gz)。

本软件不提供任何节点、订阅或代理服务。
```

---

## 11.8 About 页内容（UI 必含字段）

| 字段 | 来源 | 示例值 | 说明 |
| --- | --- | --- | --- |
| 应用名 | 常量 | Clash Zhengl | C.1 显示名 |
| 版本 | `tauri.conf.json` | 0.1.0 | 与安装包一致 |
| 构建时间 | `build.rs` 注入 `BUILD_TIME` | 2026-11-06T15:54+08:00 | 便于排障 |
| 提交哈希 | `build.rs` 注入 `GIT_SHA` | `a1b2c3d`（dirty 加 `-dirty`） | |
| 内核版本 | `core_status` | mihomo v1.19.33（compatible） | 未安装时显示"未安装" |
| 内核许可证 | 常量 | GPL-3.0 | 必含（C.12.5） |
| 内核来源 | 常量 | GitHub MetaCubeX/mihomo | 可点击（`open` 插件）但需二次确认 |
| 本软件许可证 | 常量 | 见 `LICENSE`（建议 MIT 或专有） | |
| 第三方组件 | `THIRD-PARTY-NOTICES.md` | Rust crates / Tauri / mihomo | 列出名称+许可证 |
| 数据目录 | `paths::roaming_dir()` | `%APPDATA%\ClashZhengl` | 按钮「打开」 |
| 日志目录 | `paths::log_dir()` | `%LOCALAPPDATA%\ClashZhengl\logs` | 按钮「打开」 |
| 免责声明 | 常量 | 见下 | |
| 检查更新 | 手动按钮 | 见 11.11 | |

```text
免责声明
本软件为免费开源（或按 LICENSE 授权）的代理客户端管理工具，不提供任何
节点、订阅或网络代理服务。用户须自行准备合法的订阅来源，并对使用行为
承担全部责任。请遵守所在地区法律法规及服务商条款。
```

> About 页的"打开外部链接"必须走 `tauri-plugin-opener` 且**先弹确认**：避免误点跳转。
> 禁止在 About 页放赞助、推广、返利链接（01 §8）。

---

## 11.9 GitHub Actions CI（完整 workflow）

`.github/workflows/release.yml`

```yaml
name: release

on:
  push:
    tags: ["v*.*.*"]
  workflow_dispatch:
    inputs:
      version:
        description: "版本号（如 0.1.0），留空则从 tag 取"
        required: false

permissions:
  contents: write

concurrency:
  group: release-${{ github.ref }}
  cancel-in-progress: false

env:
  CARGO_TERM_COLOR: always
  RUST_BACKTRACE: 1

jobs:
  # ---------- 阶段 1：代码质量门禁（C.13，全绿才继续） ----------
  gate:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4

      - name: Setup Node
        uses: actions/setup-node@v4
        with:
          node-version: "20"
          cache: "pnpm"

      - name: Setup pnpm
        uses: pnpm/action-setup@v4
        with:
          version: 9

      - name: Setup Rust
        uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy

      - name: Cache Rust
        uses: Swatinem/rust-cache@v2
        with:
          workspaces: "src-tauri -> target"

      - name: Install frontend deps
        run: pnpm install --frozen-lockfile

      - name: Version consistency
        shell: pwsh
        run: pwsh -File scripts/check-version.ps1

      - name: cargo fmt
        working-directory: src-tauri
        run: cargo fmt --check

      - name: cargo clippy
        working-directory: src-tauri
        run: cargo clippy --all-targets -- -D warnings

      - name: cargo test
        working-directory: src-tauri
        run: cargo test --all-features

      - name: tsc
        run: pnpm tsc --noEmit

      - name: biome
        run: pnpm biome check

      - name: build frontend
        run: pnpm build

      - name: dist size budget (<= 60 KB)
        shell: pwsh
        run: |
          $size = (Get-ChildItem dist -Recurse -File | Measure-Object Length -Sum).Sum
          Write-Host "dist size: $([math]::Round($size/1KB,1)) KB"
          if ($size -gt 60KB) { throw "前端产物超出预算: $size bytes" }

  # ---------- 阶段 2：构建 NSIS 安装包 ----------
  build:
    needs: gate
    runs-on: windows-latest
    outputs:
      setup: ${{ steps.paths.outputs.setup }}
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0        # 需要 tag 与 git sha

      - uses: actions/setup-node@v4
        with: { node-version: "20", cache: "pnpm" }
      - uses: pnpm/action-setup@v4
        with: { version: 9 }
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
        with: { workspaces: "src-tauri -> target" }

      - name: Install deps
        run: pnpm install --frozen-lockfile

      # GPL 合规文件必须存在于仓库（见 11.7.4）
      - name: Verify license bundle
        shell: pwsh
        run: |
          $need = @(
            "LICENSE",
            "THIRD-PARTY-NOTICES.md",
            "licenses/mihomo-LICENSE.txt",
            "licenses/README-KERNEL.txt"
          )
          foreach ($f in $need) {
            if (-not (Test-Path $f)) { throw "缺少许可证文件: $f（GPL 合规要求）" }
          }

      - name: Build Tauri (NSIS)
        id: tauri
        uses: tauri-apps/tauri-action@v0
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          # 有证书时启用（见 11.6.4）
          TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
        with:
          tagName: ${{ github.ref_name }}
          releaseName: "Clash Zhengl ${{ github.ref_name }}"
          releaseBody: |
            见 CHANGELOG.md。
            ⚠️ 安装包未包含内核：首次启动会引导你从官方 Release 下载 mihomo（GPL-3.0）。
            ⚠️ 本安装包未签名，首次运行 Windows 可能提示，请选择「更多信息 → 仍要运行」。
          releaseDraft: true
          prerelease: false
          args: --target x86_64-pc-windows-msvc

      - name: Collect artifacts
        id: paths
        shell: pwsh
        run: |
          $exe = Get-ChildItem "src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/*.exe" |
                 Select-Object -First 1
          if (-not $exe) { throw "未找到 NSIS 安装包" }
          Write-Host "setup: $($exe.FullName)"
          "setup=$($exe.FullName)" >> $env:GITHUB_OUTPUT

          # 体积预算（C.11 v1.1：口径 = 签名后的 setup.exe；100%-120% 可保留但记 perf-log）
          $mb = $exe.Length / 1MB
          Write-Host "安装包体积(签名后): $([math]::Round($mb,2)) MB"
          if ($mb -gt 14.4) {
            throw "安装包超出硬上限(12MB×120%=14.4MB): $([math]::Round($mb,2)) MB"
          } elseif ($mb -gt 12) {
            Write-Host "::warning::安装包 $([math]::Round($mb,2)) MB 处于 100%-120% 过渡带，需记入 docs/perf-log.md 并给出改进项"
          }

      - name: Generate SHA256SUMS
        shell: pwsh
        run: |
          $exe = Get-ChildItem "src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/*.exe" |
                 Select-Object -First 1
          $hash = (Get-FileHash $exe.FullName -Algorithm SHA256).Hash.ToLower()
          "$hash  $($exe.Name)" | Out-File -Encoding ascii SHA256SUMS.txt
          Get-Content SHA256SUMS.txt

      - name: Upload to release
        if: startsWith(github.ref, 'refs/tags/')
        shell: pwsh
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
          $tag = "${{ github.ref_name }}"
          $exe = Get-ChildItem "src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/*.exe" |
                 Select-Object -First 1
          gh release upload $tag $exe.FullName SHA256SUMS.txt --clobber

      - name: Upload workflow artifacts
        uses: actions/upload-artifact@v4
        with:
          name: nsis-x64
          path: |
            src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/*.exe
            SHA256SUMS.txt
          retention-days: 30
```

### 11.9.1 CI 设计要点

| 决策 | 理由 |
| --- | --- |
| gate 与 build **分两个 job** | 门禁失败不浪费 15 分钟构建时间 |
| `releaseDraft: true` | 先出草稿，人工核对体积/SHA/文案后再点发布 |
| 只在 tag 触发上传 | `workflow_dispatch` 只产 artifact，不发 Release |
| **不缓存 `target/`**？ | `Swatinem/rust-cache` 缓存的是 registry 与增量，不缓存最终产物，安全 |
| 不下载内核 | 00-README §4.5：构建期不得依赖外网 |
| 体积硬门槛 | 超 12 MB 直接 fail，防止悄悄变大 |
| `fetch-depth: 0` | `build.rs` 需要 git sha 与 tag |

### 11.9.2 `build.rs` 注入构建信息

```rust
// src-tauri/build.rs
fn main() {
    tauri_build::build();

    let sha = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output().ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into());

    let dirty = std::process::Command::new("git")
        .args(["status", "--porcelain"]).output().ok()
        .map(|o| !o.stdout.is_empty()).unwrap_or(false);

    println!("cargo:rustc-env=GIT_SHA={}{}", sha, if dirty { "-dirty" } else { "" });
    println!("cargo:rustc-env=BUILD_TIME={}", chrono::Local::now().to_rfc3339());
}
```

---

## 11.10 发布检查清单（每次发版逐条打勾）

### 11.10.1 发版前（T-1 天）

| # | 检查项 | 命令/方法 | 责任人 |
| --- | --- | --- | --- |
| 1 | 三处版本号一致 | `pwsh scripts/check-version.ps1` | dev |
| 2 | `CHANGELOG.md` 已更新（含 breaking changes） | 人工 | dev |
| 3 | 全量门禁绿 | 11.9 的 gate job 或本地等价命令 | dev |
| 4 | 已跑过 12 章的异常场景清单（15 项） | 手工 | QA |
| 5 | `perf-probe.ps1` 数字达标（RSS/CPU/启动，C.11） | 脚本 | dev |
| 6 | 脱敏检查：`logs/` 无 token/secret | `grep -riE "token=|secret=[0-9a-f]{8}" logs/` | dev |
| 7 | 许可证文件四件套存在 | CI 的 `Verify license bundle` | dev |
| 8 | 图标全套存在且 16px 可辨识 | 人工看图 | design |
| 9 | 干净机器安装测试（无 WebView2 的 Win10 1809 虚拟机） | 手工 | QA |
| 10 | 卸载测试：安装 → 用 → 卸载 → 检查残留 | 手工 + `reg query` | QA |

### 11.10.2 发版时

| # | 步骤 | 说明 |
| --- | --- | --- |
| 1 | 打 tag `v0.1.0` 并 push | 触发 workflow |
| 2 | 等 gate 全绿 | 失败即修，不绕过 |
| 3 | 检查 draft Release 的安装包体积 | ≤12 MB |
| 4 | 本地下载安装包，`Get-FileHash` 与 `SHA256SUMS.txt` 比对 | 验证上传完整性 |
| 5 | 在干净虚拟机上实装一次 | 最后一道关 |
| 6 | 编辑 Release Notes（含未签名提示、内核首启下载说明、已知问题） | 见 11.10.3 模板 |
| 7 | 发布（取消 draft） | |

### 11.10.3 Release Notes 模板

```markdown
## Clash Zhengl v0.1.0

### 安装
1. 下载 `ClashZhengl_0.1.0_x64-setup.exe`
2. 校验 SHA256（见 `SHA256SUMS.txt`）
3. 运行安装包。**本版本未代码签名**，Windows 可能提示"已保护你的电脑"，
   请点击「更多信息」→「仍要运行」。

### 首次启动
- 安装包**不含内核**。首次启动会询问是否从 GitHub 下载 mihomo（GPL-3.0），
  需联网一次（约 12 MB）。若下载失败，可在设置中配置下载代理后重试。
- 首次启动会显示合规提示，确认后方可使用。

### 变更
- （逐条列出）

### 已知问题
- （逐条列出，未验证的功能必须写"未验证"）

### 许可证
- Clash Zhengl 本身：见 LICENSE
- mihomo 内核：GPL-3.0，独立进程调用，未修改，未随包分发
```

---

## 11.11 更新检查（可选能力，**不做自动下载**）

| 项 | 决定 | 理由 |
| --- | --- | --- |
| 自动检查更新 | **默认关闭**，设置项可开 | 01 §3.1「除必要地址外不得发起网络请求」 |
| 检查地址 | GitHub API `releases/latest`（唯一外部请求） | 唯一可接受来源 |
| 频率 | 手动点击 + 开启后每 24 小时最多 1 次 | 避免唤醒与流量 |
| 自动下载 | ❌ **禁止** | 01 §2.2 未列入；静默下载可执行文件是重大安全风险 |
| 自动安装 | ❌ 禁止 | 同上 |
| 展示 | About 页显示「有新版本 0.2.0」+ 打开 Release 页按钮 | 用户自行决定 |

```rust
// commands/settings.rs —— 可选实现
#[tauri::command]
pub async fn update_check() -> Result<Option<UpdateInfo>, AppError> {
    // 仅在用户显式触发时调用；24h 节流由调用方判断
    let resp = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent(concat!("clash-zhengl/", env!("CARGO_PKG_VERSION")))
        .build()?
        .get("https://api.github.com/repos/<owner>/clash-zhengl/releases/latest")
        .send().await.map_err(AppError::from_reqwest)?;
    if !resp.status().is_success() { return Ok(None); }
    let v: serde_json::Value = resp.json().await.map_err(AppError::from_reqwest)?;
    let latest = v.get("tag_name").and_then(|t| t.as_str()).unwrap_or("").trim_start_matches('v');
    Ok(semver_gt(latest, env!("CARGO_PKG_VERSION"))
        .then(|| UpdateInfo { version: latest.into(), url: v.get("html_url").and_then(|u| u.as_str()).unwrap_or("").into() }))
}
```

> **决策（C v1.1 已确认）**：`update_check` **MVP 不做**，C.6 命令表中没有它。
> 因此上段代码仅作为「将来若要做」的参考实现，**0.1.x 不落地**。
> 届时的正确路径是提 `CONTRACT-CHANGE-REQUEST:` 把命令加进 C.6（返回 `Option<{version, url}>`），
> 而不是在 `commands/` 里先写一个表外命令——表外命令会让前端契约失序。
> MVP 的更新提示改用**纯静态方式**：About 页放一个「打开发布页」按钮（走 `open` 插件），
> 由用户自行查看，零新增 IPC。

---

## 11.12 回滚方案

### 11.12.1 发布事故分级

| 级别 | 定义 | 响应时限 | 动作 |
| --- | --- | --- | --- |
| P0 | 安装后无法启动 / 系统代理无法恢复 / 泄露凭据 | 立即 | **删除 Release + 撤回 tag**，公告 |
| P1 | 核心功能不可用（订阅全失败、内核起不来） | ≤4h | 发补丁版 `0.1.1` |
| P2 | 局部功能异常（某页面错乱、某错误码文案错） | ≤3 天 | 下个常规版本 |
| P3 | 体验问题 | 排期 | — |

### 11.12.2 P0 的具体操作

```powershell
# 1) 把 Release 转为 draft（用户下载不到，但已下载者不受影响）
gh release edit v0.1.0 --draft

# 2) 删除 tag（本地 + 远端）
git tag -d v0.1.0
git push origin :refs/tags/v0.1.0

# 3) 打补丁 tag 重新构建
git tag v0.1.1
git push origin v0.1.1
```

| 事项 | 说明 |
| --- | --- |
| **已安装用户怎么办** | 若涉及系统代理残留（P0 最坏情况），在 Release Notes 与 README 顶部给出**手工恢复步骤**（见 11.12.3） |
| 版本降级 | `allowDowngrades: false` 表示 Tauri 不允许安装更低版本。回滚时**必须发新版本号**（`0.1.1`），不能重发 `0.1.0` |
| 数据兼容 | `schema_version` 只增不减；降级安装会因版本号更高而通过，但旧代码读新 schema 会走 `migrate.rs` 的向下兼容路径 —— **不在支持范围**，明确拒绝降级 |
| 内核回滚 | 用户可在设置里切回旧版本内核（`core_version` + `core_start`），无需回滚客户端 |

### 11.12.3 手工恢复系统代理（必须写进 README）

```powershell
# 系统代理被残留在死端口时，在 PowerShell 里执行：
Set-ItemProperty -Path "HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings" `
  -Name ProxyEnable -Value 0
# 或在 Windows 设置 → 网络和 Internet → 代理 → 关闭"使用代理服务器"
# 然后重启浏览器
```

### 11.12.4 回滚演练要求

| 项 | 要求 |
| --- | --- |
| 演练频率 | 每个 MINOR 版本至少一次 |
| 演练内容 | 在测试机装 `v0.1.0` → 模拟 P0 → 执行 11.12.2 全部步骤 → 装 `v0.1.1` |
| 验收 | 测试机能正常安装新版本，旧数据完好，代理恢复正常 |

---

## 11.13 验收要点（P7 门禁）

1. `cargo tauri build` 产出 `ClashZhengl_0.1.0_x64-setup.exe`，体积 ≤ 12 MB；
2. 干净 Win10 1809（无 WebView2）虚拟机：安装包能引导安装 WebView2 并启动成功；
3. 干净 Win11：安装 → 首次启动显示合规提示 → 引导下载内核 → 代理可用；
4. 安装目录无内核二进制（策略 A 验证）；`licenses/` 下四个文件齐全；
5. About 页显示 version / kernel version / GPL 声明 / 数据目录按钮均可用；
6. 卸载：勾"否"保留配置 → 重装后订阅仍在；勾"是"→ 目录彻底清空、自启项已删；
7. 开启系统代理状态下卸载 → 代理已恢复（`ProxyEnable=0`）；
8. CI 在 tag push 后自动产出 draft Release，含 exe + `SHA256SUMS.txt`；
9. 故意把版本改成 13 MB 体积然后跑 CI → build job 必须 fail（预算门槛有效）；
10. 故意删 `licenses/mihomo-LICENSE.txt` → `Verify license bundle` 必须 fail。

---

## 11.14 契约对齐记录（v1.1 / v1.2）

> C 章 v1.1 已裁决本章提出的全部 6 条缺口。原「冲突点」表作废（保留于 git 历史）。

| # | 裁决结果（C 章） | 本章修改 | 对应节 |
| --- | --- | --- | --- |
| 1 | `AppSettings.compliance_accepted_at: Option<i64>`，schema_version → 2 | 11.4 触发条件改为「该字段为 `null`」；补 schema v1→v2 迁移说明与「清空 `%APPDATA%` 会再弹」 | C.5 `AppSettings` |
| 2 | C.1 未规定打包清单文件名 | 本章定为 `LICENSE`、`THIRD-PARTY-NOTICES.md`、`licenses/mihomo-LICENSE.txt`、`licenses/README-KERNEL.txt` 四件套，CI 逐项校验存在性 | C.1（约定细化，不阻塞） |
| 3 | `update_check` **确认 MVP 不做**（不新增命令；若将来要做须提 `CONTRACT-CHANGE-REQUEST:`） | 11.11 保留「仅提示不自动下载」的结论，删去「无命令可用」的疑问语气，改为明确决策 | C.6（无此命令） |
| 4 | C.3 新增 `runtime/download/` | 11.3 安装目录树与卸载保留项引用契约路径 | C.3 |
| 5 | C.11 体积口径明确为**签名后的** setup.exe；新增 100%–120% 仲裁 | 11.1 补口径说明与 12–14.4 MB 过渡带；11.9 CI 门槛改为 `>14.4` fail、`>12` warning 并记 `docs/perf-log.md` | C.11 |
| 6 | 内核版本基线钉 `v1.19.33` | 本章默认内核版本取该值（与 08 章 `DEFAULT_CORE_VERSION` 一致） | 08 章常量（非 C 章条目） |

> v1.2 的 3 条裁决（`switch_apply_best` 返回结构、`to_delay` 为 `Option`、`manual_override_min`）
> **不涉及打包与发布**，本章无改动；相关 UI 落地见 09.18。

---

## 11.15 本章 TODO(待确认) 清单

| # | 待确认项 | 当前默认 | 确认方式 |
| --- | --- | --- | --- |
| 1 | 本软件自身采用什么许可证 | MIT（待定） | 需产品决策；影响 `LICENSE` 与 About 文案 |
| 2 | 是否有代码签名预算 | 无（0.1.x 不签名） | 产品决策 |
| 3 | GitHub 仓库归属（`<owner>/clash-zhengl`） | 占位 | 需创建仓库后回填 CI 与 About |
| 4 | Tauri updater 插件是否启用 | 不启用（仅提示，不自动下载） | 产品决策 |
| 5 | 是否提供便携版（zip，免安装） | MVP 不做 | 便携版需自处理开自启与卸载清理 |
| 6 | Win10 1809 是否仍作为最低支持版本 | 是（C.1） | 若放弃可去掉 WebView2 引导复杂度 |

---

## 11.16 本章自检清单（Agent 必须逐条确认）

- [ ] 1. `tauri.conf.json` 的 `identifier` 是否为 `com.clashzhengl.app`、产物名是否符合 `ClashZhengl_<ver>_x64-setup.exe`（C.1）？
- [ ] 2. `bundle.targets` 是否**只有** `["nsis"]`？`installMode` 是否 `currentUser`（免 UAC）？
- [ ] 3. `webviewInstallMode` 是否 `downloadBootstrapper` 且 `minimumWebview2Version` 已设？是否**没有**误选 `offlineInstaller`（会把包体炸到 130 MB）？
- [ ] 4. `bundle.externalBin` 是否为空数组（策略 A 不捆绑内核）？若改为捆绑，是否补齐 GPL 四件套（11.7.3 五条义务）？
- [ ] 5. 三处版本号（`Cargo.toml` / `tauri.conf.json` / `package.json`）是否一致？`check-version.ps1` 是否进 CI？
- [ ] 6. 安装包体积是否按**签名后**口径验收，且落在 ≤12 MB（或 ≤14.4 MB 过渡带并记 `docs/perf-log.md`）（C.11）？
- [ ] 7. `resources` 四件套（`LICENSE` / `THIRD-PARTY-NOTICES.md` / `licenses/mihomo-LICENSE.txt` / `licenses/README-KERNEL.txt`）是否都打进包内并有 CI 存在性校验？
- [ ] 8. About 页是否含内核名/版本/许可证/来源/免责声明等 15 个字段（C.12.5）？
- [ ] 9. 卸载钩子是否：PREUNINSTALL 尝试 `--shutdown-proxy` 恢复系统代理、POSTUNINSTALL 分两问且**默认保留** `%APPDATA%`（C.12.7）？
- [ ] 10. 首次启动合规提示是否由 `settings.compliance_accepted_at == null` 触发、不可绕过、接受后写回（C.5/C.12.6）？
- [ ] 11. `release.yml` 是否为 `releaseDraft: true`、产出 `SHA256SUMS.txt`、体积门槛生效？
- [ ] 12. 未签名发布时，README 与 Release Notes 是否明示「首次运行需点『更多信息 → 仍要运行』」并配截图？
- [ ] 13. 回滚方案是否含 P0–P3 分级、`gh release edit --draft`、以及**手工恢复系统代理**的命令（防止用户断网）？
- [ ] 14. 是否**没有**实现 `update_check` 自动下载？是否保持「仅提示」（C.6 无该命令，将来做须提 CHANGE-REQUEST）？
- [ ] 15. 本章是否**没有**修改 `01-master-prompt.md`、`appendices/C-conventions.md` 或其它章节？
