# 白洞 (White Hole) - 现代化桌面浏览器

白洞 是一个基于 Tauri 和 Dioxus 构建的现代化桌面浏览器，提供简洁、高效的网页浏览体验。采用 Rust 后端和现代前端技术栈，具备优异的性能和用户体验。

## 🚀 特性

### 核心功能

- **现代化界面**: 基于 Dioxus 构建的响应式用户界面
- **多标签页支持**: 支持多个标签页同时浏览
- **无痕浏览模式**: 保护隐私的无痕浏览功能
- **智能搜索**: 支持关键词搜索和 URL 直接访问
- **历史记录管理**: 自动保存和搜索浏览历史，支持导航日志分页查询与实时同步
- **书签功能**: 支持网页收藏和星标管理
- **内置 HTTP 客户端**: 支持方法、请求头、请求体配置，自动识别 JSON 并补充 Content-Type，方便调试接口
- **深度链接**: 支持 http/https 协议关联，可从命令行或链接直接打开
- **自动更新**: 支持应用自动更新功能
- **深色模式**: 支持系统级深色模式切换
- **窗口状态保存**: 自动保存和恢复窗口位置及大小
- **单实例运行**: 防止重复启动，支持命令行参数传递
- **链接预览**: 支持链接悬停预览功能
- **浮动标签页**: Ctrl+点击链接在当前窗口内以浮动视图打开预览，自带标题栏和控制按钮，支持最大化为常规标签页
- **文件下载**: 支持文件下载管理，自动发送系统通知（开始/完成/失败）
- **分屏浏览**: 支持画中画模式

### 技术特色

- **跨平台支持**: 基于 Tauri 框架，支持 Windows、Linux、macOS
- **高性能**: Rust 后端提供优异的性能表现
- **轻量级**: 使用系统原生 WebView，应用体积小
- **极简**: 点击标题或图标进入唯一主界面，Tab、历史、收藏夹三合一
- **Tailwind CSS**: 使用现代化 CSS 框架和 DaisyUI 组件库
- **SQLite 数据库**: 本地数据存储 (SQLx)，支持内存数据库的无痕模式
- **热键管理**: 支持全局热键和自定义快捷键

## 📦 安装

### 系统要求

- Windows 10/11, Linux (glibc 2.28+), 或 macOS 10.15+
- 系统 WebView 组件 (Edge WebView2 on Windows, WebKit on Linux/macOS)
- 至少 100MB 可用磁盘空间

### 下载安装

1. 从 [Releases 页面](https://github.com/thlstsul/white-hole/releases) 下载最新版本
2. 运行安装程序完成安装

### 从源码构建

```bash
# 克隆项目
git clone https://github.com/thlstsul/white-hole.git
cd white-hole

# 构建应用
cargo tauri build
```

## 🛠️ 开发

### 环境要求

- Rust 1.85+ (Edition 2024)
- Node.js 18+
- pnpm 8+
- Tauri CLI: `cargo install tauri-cli`
- Dioxus CLI: `cargo install dioxus-cli` (或 `cargo binstall dioxus-cli`)
- SQLx CLI: `cargo install sqlx-cli` (用于数据库迁移)
- 系统构建工具 (Visual Studio Build Tools on Windows, build-essential on Linux, Xcode on macOS)

### 开发模式运行

```bash
# 启动开发服务器 (自动打开调试工具)
cargo tauri dev

# 仅构建前端
dx serve
```

### 项目结构

```
white-hole/
├── Cargo.toml              # 工作区配置和依赖
├── Dioxus.toml             # Dioxus 配置
├── package.json            # 前端依赖配置
├── build.rs                # 构建脚本
├── src/                    # 前端代码 (Dioxus)
│   ├── main.rs             # 应用入口
│   ├── app.rs              # 主应用组件
│   ├── search_page.rs      # 搜索页面组件
│   ├── search_input.rs     # 搜索输入框组件
│   ├── navigation.rs       # 导航栏组件
│   ├── title_bar.rs        # 标题栏组件
│   ├── window_decoration.rs # 窗口装饰组件
│   ├── url.rs              # URL 处理工具
│   ├── api.rs              # Tauri 命令接口
│   ├── incognito.rs        # 无痕浏览模式
│   ├── extension.rs        # 扩展功能
│   ├── darkreader.rs       # 深色模式实现
│   ├── http_client.rs      # HTTP 客户端入口
│   ├── http_client/        # HTTP 客户端组件 (method/uri/header/body/response/send)
│   └── ...
├── src-tauri/              # 后端代码 (Rust)
│   ├── Cargo.toml          # 后端依赖配置
│   ├── src/
│   │   ├── lib.rs          # Tauri 应用入口
│   │   ├── main.rs         # 主函数入口
│   │   ├── browser.rs      # 浏览器核心逻辑 (窗口编排、浮动 Tab)
│   │   ├── tab.rs          # 标签页管理 (Tab/TabMap 数据结构)
│   │   ├── tab_service.rs  # 标签领域服务 (生命周期/历史同步/事件队列)
│   │   ├── database.rs     # 数据库操作
│   │   ├── command.rs      # 命令处理 (Tauri IPC 命令)
│   │   ├── state.rs        # 应用状态管理
│   │   ├── hotkey.rs       # 热键管理
│   │   ├── download.rs     # 文件下载管理与系统通知
│   │   ├── update.rs       # 更新检查
│   │   ├── url.rs          # URL 处理
│   │   ├── user_agent.rs   # 用户代理设置
│   │   ├── page.rs         # 页面管理
│   │   ├── icon.rs         # 图标处理
│   │   ├── task.rs         # 任务管理
│   │   ├── log.rs          # 日志系统
│   │   ├── error.rs        # 错误处理
│   │   ├── history.rs      # 历史事件定义
│   │   ├── darkreader.rs   # 深色模式实现
│   │   ├── public_suffix.rs # 公共后缀处理
│   │   ├── prevent_default.rs # 默认行为阻止 (Windows)
│   │   ├── clipboard.rs    # 剪贴板历史兼容 (Windows)
│   │   ├── request.rs      # fetch 命令 (HTTP 请求)
│   │   ├── macros.rs       # 自定义宏
│   │   └── ...
│   ├── capabilities/       # Tauri 权限配置
│   ├── permissions/        # 权限定义
│   ├── js/                 # 注入的 WebView 脚本 (darkreader/floating_tab 等)
│   ├── windows/            # Windows 平台特定配置
│   └── tauri.conf.json     # 应用配置文件
├── hotkey/                 # 热键功能模块
├── hotkey-macros/          # 热键宏定义 (proc-macro)
├── migrations/             # 数据库迁移文件
├── assets/                 # 静态资源 (CSS, 图标等)
├── dist/                   # 构建输出目录
├── .github/workflows/      # GitHub Actions 自动化部署
└── tailwind.css            # Tailwind CSS 配置
```

## ⌨️ 快捷键

| 功能             | 快捷键                        |
| ---------------- | ----------------------------- |
| 关闭标签页       | Ctrl+W                        |
| 刷新页面         | F5 或 Ctrl+R                  |
| 前进             | Alt+→                         |
| 后退             | Alt+←                         |
| 打开搜索视图     | Ctrl+L                        |
| 焦点离开搜索视图 | Esc                           |
| 下一标签页       | Ctrl+Tab                      |
| 上一标签页       | Ctrl+Shift+Tab                |
| 全屏切换         | F11                           |
| 开发者工具       | Ctrl+D 或 F12 或 Ctrl+Shift+I |
| 无痕浏览         | Ctrl+I                        |
| 打印页面         | Ctrl+P                        |

## 🔧 配置

### 数据库

应用使用 SQLite 数据库存储浏览历史、书签等数据：

- **正常模式**: 本地文件存储 (应用本地数据目录下的 `white-hole.db`，如 Windows `%LOCALAPPDATA%\com.thlstsul.white-hole\white-hole.db`、Linux `~/.local/share/com.thlstsul.white-hole/white-hole.db`)
- **无痕模式**: 内存数据库 (退出后清除所有数据)
- **数据迁移**: 使用 SQLx 迁移 (`migrations/` 目录，执行 `sqlx migrate run`)

### 自动更新

应用支持自动更新功能：

- 基于 Tauri Updater 插件
- 更新地址: `https://thlstsul.github.io/white-hole-latest.json`
- 发布新版本时自动检测和下载更新

### 构建配置

- **开发服务器**: 运行在 `http://localhost:1420`
- **发布构建**: 使用 `dx bundle --release` 构建前端
- **代码签名**: 支持应用代码签名 (需要配置私钥)

## 🤝 贡献

欢迎提交 Issue 和 Pull Request！请确保：

1. 遵循项目的代码风格
2. 添加适当的测试
3. 更新相关文档

## 📄 许可证

本项目采用 MIT 许可证 - 查看 [LICENSE](LICENSE) 文件了解详情。

## 🙏 致谢

- [Tauri](https://tauri.app/) - 跨平台应用框架
- [Dioxus](https://dioxuslabs.com/) - Rust 前端框架
- [Tailwind CSS](https://tailwindcss.com/) - CSS 框架
- [DaisyUI](https://daisyui.com/) - Tailwind 组件库
- [SQLx](https://github.com/launchbadge/sqlx) - 异步 SQL 数据库工具包
- [Reqwest](https://github.com/seanmonstar/reqwest) - Rust HTTP 客户端
- [time](https://github.com/time-rs/time) - 时间处理库
- [tauri-plugin-log](https://github.com/tauri-apps/plugins-workspace) - 日志插件
- [tauri-plugin-window-state](https://github.com/tauri-apps/plugins-workspace) - 窗口状态管理插件
- [tauri-plugin-deep-link](https://github.com/tauri-apps/plugins-workspace) - 深度链接插件
- [tauri-plugin-updater](https://github.com/tauri-apps/plugins-workspace) - 自动更新插件
- [tauri-plugin-notification](https://github.com/tauri-apps/plugins-workspace) - 通知插件
- [tauri-plugin-single-instance](https://github.com/tauri-apps/plugins-workspace) - 单实例运行插件
- [tokio](https://github.com/tokio-rs/tokio) - 异步运行时
- [scc](https://github.com/justjavac/scc) - 高性能并发容器
- [publicsuffix](https://github.com/rushmorem/publicsuffix) - 公共后缀列表
- [url](https://github.com/servo/rust-url) - URL 解析库
- [colored](https://github.com/mackwic/colored) - 控制台颜色库
- [fern](https://github.com/davidbarsky/fern) - 日志系统库
- [error_set](https://github.com/oxc-project/error-set) - 错误类型宏

## 📞 联系方式

- 作者: thlstsul
- 项目主页: https://github.com/thlstsul/white-hole

---

**白洞** - 探索网络的新维度 🌌
