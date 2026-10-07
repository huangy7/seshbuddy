# SeshBuddy 开发指南

这份文档描述 **SeshBuddy** 的开发方式、目录结构和调试入口，帮助你快速进入日常开发与排障流程。

## 项目概览

SeshBuddy 是一个基于 Tauri 2 的桌面应用，前端使用 Vue 3 + TypeScript，后端使用 Rust。

- **前端**：负责界面、状态管理、Monaco Editor 集成、Markdown 渲染、设置页与 API 调试视图
- **后端**：负责本地会话扫描、CLI 配置读写、代理管理、文件系统操作与监听、数据持久化和系统集成
- **当前支持平台**：macOS、Windows、Linux
- **当前支持的 CLI**：`Claude Code`、`Codex`、`Gemini`、`Antigravity`、`WorkBuddy`、`DSH`

## 环境要求

| 工具 | 建议版本 | 说明 |
|------|----------|------|
| Node.js | 20+ | 前端构建与脚本运行（Vitest 4 要求 20+） |
| npm | 8+ | 包管理 |
| Rust | 1.70+ | Tauri 后端编译 |
| rustup | 1.25+ | Rust 工具链管理 |

### macOS 依赖

```bash
xcode-select --install
```

### Windows 依赖

- [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)（勾选“C++ 桌面开发”）
- [WebView2 Runtime](https://developer.microsoft.com/en-us/microsoft-edge/webview2/)

### Linux 依赖 (Ubuntu / Debian)

Tauri 2 构建依赖 WebKitGTK 4.1 与应用指示器等系统库：

```bash
sudo apt-get install -y libgtk-3-dev libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf
```

## 快速开始

```bash
npm install
npm run tauri dev
```

这是最常用的本地开发入口。它会依次：
- 构建 `seshbuddy-proxy`（`beforeDevCommand` 的第一段）
- 启动前端开发服务器（`http://localhost:1420`）
- 编译并启动 Tauri 桌面应用

`seshbuddy-proxy` 是 workspace 的独立成员，**不在 `src-tauri` 的依赖里**，所以
`tauri dev` 内部的 `cargo build` 不会带上它 —— 这就是它必须显式写进
`beforeDevCommand` 的原因。不构建它的话，`proxy.rs` 会因为找不到二进制而报
「找不到 seshbuddy-proxy 二进制文件」，代理与流量检视功能随之不可用。

`npm run build:proxy` 除了构建，还会把产物按 sidecar 命名约定拷进
`src-tauri/binaries/seshbuddy-proxy-<target-triple>`。这一步不能省：
`tauri.conf.json` 的 `bundle.externalBin` 声明为 `binaries/seshbuddy-proxy`，
`tauri-build` 会在**编译期**（`src-tauri/build.rs`）把它展开成带 triple 后缀的
路径并校验文件存在，缺失时整个 crate 的构建脚本直接失败，报
``resource path `binaries/seshbuddy-proxy-<triple>` doesn't exist``。

命名与摆放的规则统一在 `scripts/stage-sidecar.mjs` 里。它按 `TAURI_ENV_TARGET_TRIPLE`
（tauri CLI 注入给 hook 的环境变量）推导目标，分两种情形：

- **普通目标**：构建一次，摆一份 `seshbuddy-proxy-<triple>`
- **`universal-apple-darwin`**：对 `aarch64` 与 `x86_64` 各构建一次，
  再 `lipo -create` 合成，最终摆**三份** —— 两个 per-arch 的供 tauri CLI 的两次
  构建校验，一份 universal 的供 bundler。sidecar 不参与主程序的 lipo，必须自己合成

注意这与上一条是两回事：前者是编译期对资源文件的校验，后者是运行期 `proxy.rs`
对二进制的查找。因此**任何** `cargo check` / `cargo build` 都需要先备好 sidecar，
CI 里对应的就是 `stage proxy sidecar` 那一步。

改动 `src-tauri-proxy/` 下的代码后需要**重启** `tauri dev`（`beforeDevCommand`
只在启动时跑一次）；也可以单独跑 `npm run build:proxy` 而不重启前端。

## 目录结构

```text
SeshBuddy/
├── src/                  # Vue 3 前端源码
│   ├── components/       # 视图组件（会话树、对话窗、Monaco 编辑器等）
│   ├── composables/      # 状态管理与业务逻辑
│   ├── types/            # TypeScript 类型定义
│   └── utils/            # 通用工具函数
├── src-tauri/            # Tauri 桌面应用后端 (Rust)
│   ├── src/
│   │   ├── commands/     # 前端调用的 Tauri commands
│   │   ├── db/           # SQLite 数据库访问与会话归档
│   │   ├── parser/       # 各 CLI 会话日志解析器
│   │   ├── assistant/    # 助手后端逻辑与提示词
│   │   ├── lib.rs        # 应用初始化与指令注册
│   │   └── main.rs       # 桌面端入口
│   └── tauri.conf.json   # Tauri 应用配置
├── src-tauri-proxy/      # 独立 API 反向代理服务 (Rust)
├── transcript-store/     # 会话正文抽取与缓存的共享 crate（后端与代理共用）
└── scripts/              # 跨平台构建与发布辅助脚本
```

## 调试与日志

### 前端调试

在开发模式下，可以在应用窗口里通过右键或快捷键打开 WebView DevTools。

### Rust 调试

```bash
cd src-tauri
cargo check
cargo test
```

### 应用日志

SeshBuddy 启动时会初始化应用日志，默认写入：

- macOS：`~/Library/Application Support/com.seshbuddy.app/logs/seshbuddy.log`
- Windows：`%APPDATA%\com.seshbuddy.app\logs\seshbuddy.log`

代理服务日志默认写入：
- `logs/proxy.log`

## 应用数据目录

SeshBuddy 的本地数据根目录位于：

- macOS：`~/Library/Application Support/com.seshbuddy.app/`
- Windows：`%APPDATA%\com.seshbuddy.app\`

关键数据文件：
- `data/app.db`：主 SQLite 数据库（存储会话索引、书签、设置、API profile）
- `data/traffic.db`：API 代理流量记录数据库
- `logs/`：运行时日志

## 构建与测试

```bash
# 前端测试
npm test

# 编译检查前端生产构建（含 vue-tsc 类型检查）
npm run build:web

# 后端测试与编译检查
# 干净检出后必须先跑这一步：seshbuddy 的构建脚本要求 sidecar 已就位（见「快速开始」）
npm run build:proxy
cargo test
cargo check -p seshbuddy
```

CI 会在 PR 与 push 到 `main` 时自动跑上述前端与后端测试。

## 发布

发布产物**全部由 CI 构建**，本地不再有打包脚本。流程是：

1. 改 `package.json` 的 `version`
2. 打 tag 并推送：`git tag v0.1.0 && git push origin v0.1.0`
3. `.github/workflows/release.yml` 跑三条 matrix 分支：`macos-latest` 分别以
   `--target aarch64-apple-darwin` 与 `--target x86_64-apple-darwin` 各出一条腿，
   `windows-latest` 出一条 x64 腿，由 `tauri-action` 创建一个 **draft** Release
   并上传安装包、`.sig` 签名与 `latest.json`。**不发 universal 包**，原因见该文件注释
4. **手动去 Releases 页面点 Publish**

第 4 步不能省：应用的更新端点是 `https://github.com/huangy7/seshbuddy/releases/latest/download/latest.json`，
而 GitHub 的 `/releases/latest` 按定义排除 draft 与 prerelease。未 Publish 之前，
这个端点会一直返回上一个已发布版本的清单（首个版本尚未发布时则为 404），用户端表现为「没有更新」。

注意 `tauri-action@v1` 在 `releaseDraft: true` 下遇到「该 tag 已存在非 draft release」会直接失败。
也就是说**手动 Publish 之后不能再 re-run 这个 workflow**；要重发得先删除该 Release 再重跑。

### 产物命名与下载链接

Release 资产名由 `release.yml` 的 `releaseAssetNamePattern` 固定为 `[name]_[arch][setup][ext]`，
**不含版本号**，因此每个文件名跨版本稳定：

| 平台 | 文件 |
|------|------|
| macOS（Apple Silicon） | `SeshBuddy_aarch64.dmg` |
| macOS（Intel） | `SeshBuddy_x64.dmg` |
| Windows 安装器 | `SeshBuddy_x64-setup.exe` |
| Windows MSI | `SeshBuddy_x64.msi` |
| 更新包（自动更新用） | `SeshBuddy_<arch>.app.tar.gz` 及其 `.sig` |

官网可直接写死 `https://github.com/huangy7/seshbuddy/releases/latest/download/<文件名>`。
链接指向的 release 必须已 Publish —— 与更新端点同理，`/releases/latest` 排除 draft。

自动更新不受文件名影响：更新器读 `latest.json`，其中每个平台的 `url` 指向 asset ID。

### 代码签名

当前发布的是未签名版本，CI 里没有任何签名步骤。日后要加时，接入点是 `release.yml` 中
`build and release Tauri app` **之前**的一个 step：Windows 用 `signtool`，macOS 用
Developer ID 证书 + notarization（`tauri-action` 支持 `APPLE_CERTIFICATE` 等 secret）。

## 发布签名密钥

更新包必须用 rsign 私钥签名，`tauri.conf.json` 的 `plugins.updater.pubkey` 固定了对应的公钥。
私钥**只存两处**，任何情况下都不要进仓库（`.gitignore` 已忽略 `updater-signing.key`）：

| 位置 | 用途 |
|------|------|
| `~/.tauri/seshbuddy.key` | 本机生成与备份 |
| GitHub repository secret | CI 发布 |

### 生成与配置

```bash
npx tauri signer generate -w ~/.tauri/seshbuddy.key   # 会要求设置口令
```

把输出的公钥填进 `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`。本机不再打包，
导出下面两个变量是为了在本地复现 CI 的验签自检 —— 让 `tauri signer sign` 与
`verify-updater-key.mjs`（见文末）能读到这把私钥，提前确认它与 pin 的公钥配对：

```bash
# 本机（macOS / Linux）
export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/seshbuddy.key)"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=<生成时设置的口令>
```

Windows 用 `set /p TAURI_SIGNING_PRIVATE_KEY=<%USERPROFILE%\.tauri\seshbuddy.key`。

CI 侧在 GitHub → Settings → Secrets → Actions 新增同名的两个 secret，`release.yml` 已经在读。

### 三个必须注意的点

1. **`TAURI_SIGNING_PRIVATE_KEY` 必须是密钥内容本身，不能是文件路径。** 给路径时
   `tauri signer` 会把路径字符串当 base64 去解码，报 `Invalid symbol 46`（46 即 `.`）。
   所以用 `$(cat ...)` 或 `set /p` 把内容读进变量。
2. **口令要和私钥一起备份。** 私钥丢了可以重新生成一对，但**口令丢了这把私钥就永久报废** ——
   它被 Argon2id 加密，没有任何恢复途径。建议把私钥和口令分开存放。
3. **密钥必须与 `tauri.conf.json` 里的公钥配对。** 配错不会报任何错，只会安静地产出
   一个签名无效的更新包。`release.yml` 会在构建前跑
   `scripts/verify-updater-key.mjs` 做一次探针签名 + 验签，不配对立刻中止。
   该脚本也可单独运行：

   ```bash
   npx tauri signer sign /tmp/probe.txt
   node scripts/verify-updater-key.mjs /tmp/probe.txt /tmp/probe.txt.sig src-tauri/tauri.conf.json
   ```
