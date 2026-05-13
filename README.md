# 鸣潮 (Wuthering Waves) 下载管理器

一个基于 [Tauri 2](https://v2.tauri.app/) + Vue 3 的跨平台桌面应用，用于管理鸣潮游戏客户端的下载、更新和服务器切换。

支持 **国服 (CN)**、**国际服 (Global)**、**B站服 (Bilibili)** 三个服务器。

## 平台支持

- **Windows** — 完整支持
- **Linux** — 完整支持

## 功能

| 功能 | 说明 |
|------|------|
| ⬇ 下载游戏 | 从官方 CDN 下载完整游戏客户端，下载完成后自动校验完整性 |
| 🔄 更新游戏 | 检查服务器最新版本，若可更新则自动清理旧文件并重新下载 |
| 🔍 检验游戏完整性 | 对所有游戏文件进行 MD5 校验，补下缺失或损坏的文件 |
| ✅ 应用服务器更改 | 在国服/国际服/B站服之间切换 |
| 📦 预下载 | 提前将更新包下载到本地，执行「更新游戏」时自动应用 |
| 🎮 DX11/12 切换 | 支持在启动游戏时选择 DX11 或 DX12 渲染模式 |
| 📂 选择路径 | 支持手动输入或系统文件夹选择器 |

## 技术栈

- **前端**: Vue 3 + Vite
- **后端**: Rust + Tauri 2
- **HTTP 客户端**: reqwest (支持断点续传)
- **并发**: tokio (8 路并发下载，每下载支持 8 线程分块)
- **校验**: MD5 完整性校验 + 本地缓存（64 路并发校验）

## 开发环境要求

- [Node.js](https://nodejs.org/) >= 18
- [Rust](https://www.rust-lang.org/) >= 1.70
- Linux: 需要安装 `libwebkit2gtk-4.1-dev` 等系统依赖
- Windows: 需要安装 Visual Studio Build Tools 或 Windows SDK

### Linux 系统依赖

```bash
# Debian / Ubuntu
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev

# Arch
sudo pacman -S webkit2gtk-4.1 base-devel curl wget file \
  libxdo openssl libayatana-appindicator librsvg
```

## 快速开始

```bash
# 克隆项目
git clone <repo-url>
cd wutheringwaves-download-manager-gui

# 安装前端依赖
npm install

# 开发模式运行
npm run tauri dev

# 构建发布版本
npm run tauri build
```

构建产物位于 `src-tauri/target/release/bundle/`。

## 项目结构

```
├── src/                      # Vue 前端源码
│   └── App.vue               # 主界面 (单文件组件)
├── src-tauri/                # Rust 后端源码
│   ├── src/
│   │   ├── main.rs           # 入口
│   │   ├── lib.rs            # Tauri 插件和命令注册
│   │   ├── commands.rs       # Tauri 命令 (前后端桥接)
│   │   ├── manager.rs        # 核心逻辑 (下载/同步/更新/预下载)
│   │   ├── config.rs         # 服务器配置和应用配置
│   │   ├── error.rs          # 错误类型定义
│   │   └── md5_cache.rs      # MD5 缓存 (避免重复计算)
│   ├── Cargo.toml
│   └── tauri.conf.json
├── package.json
└── vite.config.js
```

## 配置文件

应用配置保存在：
- **Windows**: `%APPDATA%/ww_manager/config.json`
- **Linux**: `~/.config/ww_manager/config.json`

游戏客户端配置保存在游戏目录下的 `launcherDownloadConfig.json`。

## 使用说明

1. 启动应用后，设置游戏安装路径（可手动输入或点击 📂 选择）
2. 选择目标服务器（国服/国际服/B站服）
3. 点击对应操作按钮

### 操作说明

- **下载游戏**: 全新安装，会从 CDN 下载完整客户端并自动校验
- **更新游戏**: 检查服务器版本，若已是最新版则提示无需更新；若有新版本则自动清理旧文件并重新下载
- **检验完整性**: 对所有已下载文件做 MD5 校验，补下损坏或缺失的文件
- **应用服务器更改**: 切换服务器时自动对差异文件进行处理并做全量同步
- **预下载**: 在版本更新前提前下载更新包到 `.predownload/` 目录，不影响当前游戏。执行「更新游戏」时会自动检测并应用已预下载的文件

## 运行环境要求

| 平台 | 运行依赖 |
|------|----------|
| **Windows** | [WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)（Windows 10/11 通常已内置） |
| **Linux** | [WebKitGTK](https://webkitgtk.org/) (libwebkit2gtk-4.1) |

Linux 用户需安装以下依赖：

通过deb和rpm包安装会自动处理依赖

```bash
# Arch
sudo pacman -S webkit2gtk-4.1 gtk3
```

## 在 Linux 上游玩鸣潮

本工具仅负责游戏文件管理。Linux 用户需通过 **Steam + Proton** 运行游戏。

点击「启动游戏」按钮会弹出详细指引，以下是完整步骤：

### 1. 添加游戏到 Steam

1. 打开 **Steam → 左下角「添加游戏」→「添加非 Steam 游戏」**
2. 点击「浏览」，选择游戏可执行文件：
   `Client/Binaries/Win64/Client-Win64-Shipping.exe`

### 2. 启用 Proton 兼容层

1. 右键游戏 → **属性**
2. 勾选 **「强制使用特定 Steam Play 兼容性工具」**
3. 选择以下任一兼容层：

| 兼容层 | 链接 |
|--------|------|
| [GE-Proton](https://github.com/GloriousEggroll/proton-ge-custom) | https://github.com/GloriousEggroll/proton-ge-custom |
| [dwproton](https://dawn.wine/dawn-winery/dwproton) | https://dawn.wine/dawn-winery/dwproton |

### 3. 设置启动参数

在 Steam 启动选项中添加（默认不添加即为 DX12 模式）：

```
steamdeck=1
```

如需切换为 DX11 模式：

```
steamdeck=1 -dx11
```

> 如果遇到 ACE 反作弊环境警告，可尝试在 **GE-Proton** 和 **dwproton** 之间切换。

Windows 用户可直接点击「启动游戏」按钮，无需额外配置。

## 致谢

本项目受 [wutheringwaves-cli-manager](https://github.com/timetetng/wutheringwaves-cli-manager) 启发，感谢该项目提供的服务端 API 分析参考。

## License

MIT
