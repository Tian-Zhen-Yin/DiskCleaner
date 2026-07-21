# C 盘占用分析器 v1 — 设计文档

## 摘要

在现有清理工具上新增"C 盘占用分析"子系统,用监控目录 + 大文件清单的双层结构解决"不知道谁吃了 C 盘"。扫描引擎用原生 FindFirstFileExW(不依赖 walkdir 逐项 stat),快照是扁平 Vec 而非目录树,单份 < 20 KB。v1 做扫描、快照、可视化、手动 diff;"清理完又红"需要的后台定时快照与告警通知留 v2。

## 核心决策

评审前版方案暴露两个阻断问题:前 3 层截断遗漏深路径元凶(Docker vhdx / 微信 FileStorage / WSL distro 全在 6-7 层);walkdir + JSON 树导致性能和体积双双失控。本版改用:

- 双层快照:监控目录(扁平列表,含深路径)+ Top-N 大文件。两者都是扁平列表,体积 KB 级。
- 原生 walker:基于 FindFirstFileExW + FIND_FIRST_EX_LARGE_FETCH,从 WIN32_FIND_DATAW.nFileSize 直接读大小,不做二次 stat。比 walkdir+metadata 快 2-3 倍。
- 手动触发为主:全盘扫用于首份快照和手动刷新;日常可只重扫监控目录(秒级)。后台定时是 v2。

## 数据结构

    pub struct MonitorSnapshot {
        pub timestamp: String,
        pub drive_total: u64,
        pub drive_used: u64,
        pub monitor_dirs: Vec<MonitorEntry>,
        pub large_files: Vec<LargeFileEntry>,
    }

    pub struct MonitorEntry {
        pub path: String,
        pub size_bytes: u64,
        pub file_count: u64,
        pub exists: bool,
    }

    pub struct LargeFileEntry {
        pub path: String,
        pub size_bytes: u64,
    }

快照不存 children,下钻由 analyze_drilldown 命令实时扫描单层。

## 关键改动

### 1. analyzer.rs — 扫描引擎与快照

原生 walker walk_dir_fast(root, collector):
- FindFirstFileExW 带 FIND_FIRST_EX_LARGE_FETCH 枚举,从 WIN32_FIND_DATAW 取 nFileSize 和 dwFileAttributes。
- 跳过 reparse point(FILE_ATTRIBUTE_REPARSE_POINT,避免 junction 循环)。
- 跳过 System Volume Information(无 backup privilege 不可访问)。
- 递归子目录,access-denied 静默跳过。

全盘扫描 scan_full() -> MonitorSnapshot:
- 用 walker 遍历 C 盘根目录,累计两个输出:监控目录列表各目录大小 + 全局 Top-N 大文件。
- Top-N 用大小为 N 的最小堆维护,避免全量排序 200 万文件。
- 自动追加:加载上一份快照,把其中 large_files 的父目录并进本次监控目录列表(去重,上限 200 条)。

监控目录重扫 scan_monitor_dirs(dirs) -> Vec<MonitorEntry>:
- 只对预设/追加的几十个目录各跑一次 walker,秒级完成。
- 不枚举全盘大文件,沿用上次全扫结果。

监控目录默认集(环境变量展开,UAC 下指向当前用户):
C 盘根下 Users / ProgramData / Program Files / Program Files (x86) / Windows / Windows Installer / Windows SoftwareDistribution / Windows Temp,
加 %LOCALAPPDATA% / %LOCALAPPDATA%\Docker / %LOCALAPPDATA%\Packages / %LOCALAPPDATA%\Programs / %APPDATA% / %USERPROFILE%\Downloads / %USERPROFILE%\Documents。

下钻 drilldown(path) -> Vec<MonitorEntry>:
- 扫描 path 的直接子项,目录大小用 walker 累计,文件大小直接读。只一层不递归。

快照存储 SnapshotStore:
- 路径 %APPDATA%\DiskClearTool\snapshots 下按 unix 时间戳命名(每份 < 20 KB,JSON 可接受)。
- index.json 存摘要列表,list 时不解析全量快照。
- save / list / load(ts) / prune(keep_days)。
- 写入用 tokio Mutex 串行化,防并发冲突。

Diff diff(prev, curr) -> Vec<DirDelta>:
- 对 monitor_dirs 和 large_files 分别按 path 键合并,算 delta_bytes。
- 移动抑制:prev 消失的大文件与 curr 新增的大文件,大小在正负 5% 且扩展名相同,标记 kind 为 moved,不报暴涨。
- 降序输出。

### 2. lib.rs — Tauri 命令

- analyze_full_scan() — 全盘扫,存快照,返回 MonitorSnapshot。
- analyze_rescan_monitors() — 只重扫监控目录,返回 Vec<MonitorEntry>。
- analyze_get_latest() — 最近一份快照。
- analyze_list_snapshots() — 快照历史摘要。
- analyze_drilldown(path) — 单层下钻。
- analyze_diff(prev_ts, curr_ts) — 两份快照 diff。
- analyze_get_config() / analyze_set_config(cfg) — 监控目录、阈值、Top-N、保留天数。

全盘扫通过 spawn_blocking 跑,前端事件 analyze-scan-progress 推进度。

### 3. 前端 — 占用分析页签

顶部 tab 切换:清理(现有全部)/ 占用分析(新增)。现有内容包进 cleanup tab 不动。

占用分析页:
- 顶部操作条:全盘扫描、重扫监控目录、对比上一份快照(下拉选时间点)。
- 快照摘要:C 盘已用/总量、扫描时间、扫描类型。
- 监控目录列表:降序,每行 路径/大小/占比条/增量(有 diff 时)。点行展开一层子目录。
- 大文件清单:Top 50,每行 路径/大小。系统大文件(pagefile/hiberfil/swapfile)标灰色。
- 监控目录列表和大文件列表用虚拟滚动。
- diff 视图:选中两份快照后,监控目录列表右侧显示增量列(红涨绿降),大文件清单切到新增/消失/移动分组。

配置面板:监控目录增删、大文件阈值(默认 500 MB)、Top-N(默认 50)、保留天数(默认 14)。

### 4. 配置模型 models.rs

    pub struct AnalyzerConfig {
        pub monitor_dirs: Vec<String>,
        pub large_file_min_bytes: u64,
        pub large_file_top_n: u32,
        pub snapshot_keep_days: u32,
    }

存 %APPDATA%\DiskClearTool\analyzer.json,与 config.json 同级。

## 不做(v2/v3)

- 后台定时快照:v1 纯手动/前台。v2 加 IntervalRunner,每 N 小时自动重扫监控目录、存快照、跑 diff。
- 告警通知:需要 tauri-plugin-notification + capability 改动,v2 随定时器一起做。
- DISM / Windows.old / 休眠文件清理:属深度清理范畴,独立 spec。

## 测试计划

analyzer.rs 单元测试(用 tempdir):
- 构造嵌套目录 + 已知大小文件,验证 walker 累加正确。
- 验证 reparse point 被跳过(用 junction 构造循环,确认不死递归)。
- 验证 Top-N 最小堆在大于 N 个候选时保留最大的 N 个。
- 验证 diff 的新增、删除、增减、移动四种情况 delta 正确。
- 快照 store:save/list/load/prune 生命周期,index.json 一致性。

验收:cargo test --lib 全绿;npm run build 通过;手动全盘扫一次确认列表合理且耗时在 SSD < 5 分钟。

## 假设与默认

- 管理员权限:复用现有 manifest,walker 可访问大多数目录;少数拒绝访问静默跳过。
- UAC 下环境变量:UAC 提升是同用户高权限 token,%LOCALAPPDATA% 指向当前用户 profile。
- 性能预算:SSD 50 万文件约 1-2 分钟、200 万约 3-5 分钟;机械盘 2-3 倍。快照每份 < 20 KB,14 天 56 份 < 1 MB。
- 监控目录上限 200 条:超限时按上次 delta 绝对值保留最活跃的 200 条。
- 不排除 WinSxS / Installer:扫描并展示。System Volume Information 因不可访问而排除。
