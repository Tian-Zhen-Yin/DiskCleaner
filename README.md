# DiskClearTool - Windows C盘清理工具

基于 **Tauri 2 + React + TypeScript + Rust** 的 Windows C 盘清理工具，支持定时执行、系统计划任务集成，以管理员身份运行。

## 功能

- 🧹 **可选清理项**
  - Windows Temp 临时文件（`%TEMP%`、`C:\Windows\Temp`、CrashDumps）
  - 浏览器缓存（Chrome / Edge / Firefox 的 Cache / Code Cache / GPUCache 等）
  - Windows Update 缓存与日志（`SoftwareDistribution\Download`、`Logs\CBS`、`Logs\WindowsUpdate`）
- 📊 扫描预估可释放空间、显示 C 盘使用率
- ⏰ **定时执行**：
  - 应用内 tokio 定时器（前台运行时生效）
  - 通过 `schtasks` 注册系统级 Windows 任务计划，开机自动执行
- 🛡 启动时通过 manifest 请求 **管理员权限**，可清理受保护的系统目录
- 📝 实时日志面板

## 项目结构

```
DiskClearTool/
├── package.json              # 前端依赖与脚本
├── tsconfig.json
├── vite.config.ts
├── index.html
├── src/                      # React 前端
│   ├── main.tsx
│   ├── App.tsx
│   └── styles.css
└── src-tauri/                # Rust 后端
    ├── Cargo.toml
    ├── build.rs
    ├── app.manifest          # UAC requireAdministrator
    ├── tauri.conf.json
    ├── capabilities/default.json
    └── src/
        ├── main.rs
        ├── lib.rs            # Tauri 命令入口
        ├── models.rs         # 类型定义
        ├── paths.rs          # 各清理类别对应目录
        ├── cleaner.rs        # 扫描 / 清理逻辑 + 磁盘信息
        ├── config.rs         # 配置持久化
        ├── scheduler.rs      # 应用内 tokio 定时器
        └── sys_task.rs       # Windows 任务计划注册
```

## 环境要求

1. **Node.js** ≥ 18（已检测 v24.12 ✓）
2. **Rust** 工具链：访问 <https://rustup.rs/> 安装 `rustup`（当前机器未检测到，需要先安装）
3. **Microsoft C++ Build Tools**（Tauri 必需）
4. **WebView2** 运行时（Windows 10/11 一般已预装）

## 打包成 exe

### 方式一：本地打包（推荐，需要"干净"的 Windows 环境）

```powershell
# 1. 安装 Rust（如未安装）
winget install Rustlang.Rustup
# 重新打开 PowerShell 以刷新 PATH

# 2. 安装 MSVC Build Tools（如未安装）
winget install Microsoft.VisualStudio.2022.BuildTools

# 3. 安装 Node 依赖
npm install

# 4. 打包
npm run tauri build
```

打包产物位置：

| 类型 | 路径 |
|------|------|
| 可执行 exe（绿色版） | `src-tauri/target/release/disk-clear-tool.exe` |
| MSI 安装包 | `src-tauri/target/release/bundle/msi/*.msi` |
| NSIS 安装包 | `src-tauri/target/release/bundle/nsis/*-setup.exe` |

> 注意：若 cargo 在 build script 阶段稳定出现 `Os { code: 0, message: "操作成功完成" }` panic，
> 通常是本机安装的安全软件（360 / 火绒 / Defender 实时防护等）对 `CreateProcess`/`WaitForSingleObject` 做了挂钩。
> 可临时退出安全软件或将 `%USERPROFILE%\.cargo` 与项目目录加入白名单后重试。

### 方式二：用 GitHub Actions 云端打包（本地环境受限时的兜底方案）

仓库根目录已自带 [`.github/workflows/build.yml`](./.github/workflows/build.yml)。

1. 将代码推送到 GitHub：
   ```bash
   git init
   git remote add origin https://github.com/<你的用户名>/<你的仓库名>.git
   git add .
   git commit -m "init"
   git push -u origin main
   ```
2. 进入 GitHub 仓库 → Actions → 选择 "Build Tauri App" → 点击 **Run workflow**，或直接推送一个 tag：
   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```
3. 等待绿色 ✓ 完成后，从该次 Run 的 **Artifacts** 区下载：
   - `DiskClearTool-portable-exe`：绿色版 exe
   - `DiskClearTool-msi`：MSI 安装包
   - `DiskClearTool-nsis`：NSIS 安装包
4. 如果是用 tag 触发，还会自动创建一个 GitHub Release 把上面三种产物作为附件。


## 使用说明

1. 启动后点击 **「扫描」** 查看各项可释放空间。
2. 勾选要清理的类别，点击 **「立即清理」**。
3. 在 **「定时清理」** 区设置频率（每日 / 每周）和时间，然后：
   - 点击 **「保存(应用内定时)」**：当应用打开时自动按计划执行。
   - 点击 **「注册到系统计划任务」**：写入 Windows 任务计划程序，开机后由 SYSTEM 后台静默执行（调用 `your.exe --headless-clean WindowsTemp BrowserCache WindowsUpdate`）。
   - **「移除系统计划任务」**：从系统计划中删除。

## 隐式命令行模式

当注册到系统计划任务后，可执行文件会被以这种方式调用：

```
DiskClearTool.exe --headless-clean WindowsTemp BrowserCache WindowsUpdate
```

此模式不显示 UI，仅执行清理并退出。

## 安全说明

- 清理仅作用于上述明确的临时/缓存目录，不会触及用户文档、桌面或程序文件。
- 浏览器缓存清理建议在浏览器关闭时执行；正在被占用的文件会被跳过并记入错误列表。
- 一旦清理无法撤销，请确认勾选项后再执行。
