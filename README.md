# 白洞 (White Hole) - 现代化桌面浏览器

白洞 是一个基于 Tauri 和 Dioxus 构建的现代化桌面浏览器，提供简洁、高效的网页浏览体验。采用 Rust 后端和现代前端技术栈，具备优异的性能和用户体验。

## 🚀 特性

### 核心功能
- **现代化界面**: 基于 Dioxus 构建的响应式用户界面
- **多标签页支持**: 支持多个标签页同时浏览
- **无痕浏览模式**: 保护隐私的无痕浏览功能
- **智能搜索**: 支持关键词搜索和 URL 直接访问
- **历史记录管理**: 自动保存和搜索浏览历史
- **书签功能**: 支持网页收藏和星标管理
- **自动更新**: 支持应用自动更新功能

### 技术特色
- **跨平台支持**: 基于 Tauri 框架，支持 Windows、Linux、macOS
- **高性能**: Rust 后端提供优异的性能表现
- **轻量级**: 使用系统原生 WebView，应用体积小
- **极简**: 点击标题或图标进入唯一主界面，Tab、历史、收藏夹三合一
- **Tailwind CSS**: 使用现代化 CSS 框架和 DaisyUI 组件库
- **SQLite 数据库**: 本地数据存储，支持内存数据库的无痕模式

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
- Rust 1.70+
- Node.js 18+
- pnpm 8+
- Tauri CLI: `cargo install tauri-cli`
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
├── src/                 # 前端代码 (Dioxus)
│   ├── main.rs         # 应用入口
│   ├── app.rs          # 主应用组件
│   ├── search_page.rs  # 搜索页面组件
│   ├── api.rs          # Tauri 命令接口
│   ├── settings.rs     # 设置页面组件
│   └── ...
├── src-tauri/          # 后端代码 (Rust)
│   ├── src/
│   │   ├── lib.rs      # Tauri 应用入口
│   │   ├── browser.rs  # 浏览器核心逻辑
│   │   ├── tab.rs      # 标签页管理
│   │   ├── database.rs # 数据库操作
│   │   └── ...
│   ├── capabilities/   # Tauri 权限配置
│   ├── migrations/     # 数据库迁移文件
│   └── tauri.conf.json # 应用配置文件
├── assets/             # 静态资源 (CSS, 图标等)
├── .github/workflows/  # GitHub Actions 自动化部署
└── tailwind.css        # Tailwind CSS 配置
```

## ⌨️ 快捷键

| 功能 | 快捷键 |
|------|--------|
| 关闭标签页 | Ctrl+W |
| 刷新页面 | F5 或 Ctrl+R |
| 前进 | Alt+→ |
| 后退 | Alt+← |
| 打开搜索 | Ctrl+L 或 Ctrl+T |

## 🔧 配置

### 数据库
应用使用 SQLite 数据库存储浏览历史、书签等数据：
- **正常模式**: 本地文件存储 (`~/.local/share/white-hole/white-hole.db`)
- **无痕模式**: 内存数据库 (退出后清除所有数据)
- **数据迁移**: 支持数据库版本升级和迁移

### 自动更新
应用支持自动更新功能：
- 基于 Tauri Updater 插件
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

## 📞 联系方式

- 作者: thlstsul
- 项目主页: https://github.com/thlstsul/white-hole

---

**白洞** - 探索网络的新维度 🌌
