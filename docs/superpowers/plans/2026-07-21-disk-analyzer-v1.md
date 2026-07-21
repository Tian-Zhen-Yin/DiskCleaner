# C 盘占用分析器 v1 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** 实现 C 盘占用分析子系统:原生 walker 扫描 + 双层快照(监控目录 + Top-N 大文件)+ 可视化页签 + 手动 diff。

**Architecture:** 新增 analyzer.rs 负责扫描引擎和快照存储;lib.rs 加 7 个 Tauri 命令;前端拆成清理和占用分析两个 tab,后者含监控目录列表、大文件清单、diff 视图。原生 FindFirstFileExW walker 替代 walkdir 避免二次 stat。

**Tech Stack:** Rust(windows crate 0.58,已有 Storage_FileSystem 特性)、Tauri 2、React 18 + TypeScript。

**Spec:** docs/superpowers/specs/2026-07-21-disk-analyzer-v1-design.md

---

## 文件结构

**新增:** src-tauri/src/analyzer.rs(扫描引擎、快照存储、diff、配置);src/AnalyzerTab.tsx(占用分析页签)。

**修改:** src-tauri/src/models.rs(新结构);src-tauri/src/lib.rs(注册模块 + 7 个命令);src/App.tsx(顶部 tab 切换);capabilities/default.json 不动。
**修改:** src-tauri/src/models.rs(新结构);src-tauri/src/lib.rs(注册模块 + 7 个命令);src/App.tsx(顶部 tab 切换);capabilities/default.json 不动。

---

## Task 1: 数据模型(models.rs)

**Files:** Modify src-tauri/src/models.rs

- [ ] Step 1: 在 models.rs 末尾追加新结构(全部 derive Serialize 和 Deserialize,AnalyzerConfig 加 Default)。
  - MonitorEntry: path(String), size_bytes(u64), file_count(u64), exists(bool)。
  - LargeFileEntry: path(String), size_bytes(u64)。
  - MonitorSnapshot: timestamp(String), scan_type(String), drive_total(u64), drive_used(u64), monitor_dirs(Vec<MonitorEntry>), large_files(Vec<LargeFileEntry>)。
  - DirDelta: path(String), kind(String), prev_bytes(u64), curr_bytes(u64), delta_bytes(i64), pct(f64)。
  - AnalyzerConfig: monitor_dirs(Vec<String>), large_file_min_bytes(u64), large_file_top_n(u32), snapshot_keep_days(u32)。
  - Default:monitor_dirs 用 spec 默认集(硬编码 + env 展开);large_file_min_bytes = 500 MB;large_file_top_n = 50;snapshot_keep_days = 14。
- [ ] Step 2: cargo build --manifest-path src-tauri/Cargo.toml 通过(dead_code 警告可忽略)。
- [ ] Step 3: git commit -m "feat(analyzer): data models for monitor snapshots"

## Task 2: 原生 walker(analyzer.rs 核心扫描)

**Files:** Create src-tauri/src/analyzer.rs;Modify src-tauri/src/lib.rs(加 mod analyzer)

v1 最核心部分,用 TDD 驱动正确性。

- [ ] Step 1: 写失败测试。analyzer.rs 底部加 cfg(test) mod tests。用 std::env::temp_dir 构造 root 下 a.txt(10 字节)、sub 下 b.txt(20 字节)、sub 下 deeper 下 c.txt(5 字节)。断言 walk_dir_fast(root) 返回 size_bytes 等于 35、file_count 等于 3。
- [ ] Step 2: cargo test --manifest-path src-tauri/Cargo.toml --lib analyzer — 编译失败(walk_dir_fast 未定义)。
- [ ] Step 3: 实现 walk_dir_fast。签名 pub struct DirSummary { pub size_bytes: u64, pub file_count: u64 } 和 pub fn walk_dir_fast(root: &Path) -> DirSummary。Windows 分支:FindFirstFileExW 枚举 root 下所有项,level FIND_FIRST_EX_LARGE_FETCH;跳过点和点点;跳过 FILE_ATTRIBUTE_REPARSE_POINT;文件累加 nFileSize,目录递归;access-denied 返回 0。非平台分支用 walkdir crate fallback(工程便利,保证测试跨平台)。
- [ ] Step 4: cargo test --lib analyzer 通过。
- [ ] Step 5: 写 reparse point 跳过测试。构造 tempdir + junction,断言不死递归。CI 受限则标 #[ignore]。
- [ ] Step 6: 运行或确认 ignored。
- [ ] Step 7: 实现 list_children(path) 返回 Vec<MonitorEntry>。扫一层,目录用 walk_dir_fast 算大小,文件直接读大小。给 Task 6 drilldown 用。
- [ ] Step 8: git commit -m "feat(analyzer): native FindFirstFileExW walker"
- [ ] Step 8: git commit -m "feat(analyzer): native FindFirstFileExW walker"

## Task 3: Top-N 大文件 + 全盘扫描

**Files:** Modify src-tauri/src/analyzer.rs

- [ ] Step 1: 写失败测试。构造 tempdir 放 5 个文件大小 [1,2,3,4,5] MB,top_n=3。断言 scan_large_files(root, min_bytes=0, top_n=3) 返回最大的 3 个(5,4,3),降序。
- [ ] Step 2: 确认失败(函数未定义)。
- [ ] Step 3: 实现 scan_large_files。用 BinaryHeap(最小堆,大小 N)维护,遍历完弹出排序。超过 N 的候选弹最小值。文件大小从 walk_dir_fast 路径上的 FindFirstFileExW 收集(需扩展 walker 同时输出候选文件,或独立扫描器;推荐:新增 walk_dir_collect(root, files_out: &mut Vec<(PathBuf,u64)>),scan_large_files 调它再堆筛选)。
- [ ] Step 4: 测试通过。
- [ ] Step 5: 实现 scan_full() -> MonitorSnapshot。组装:用 default 监控目录集 + 上一份快照自动追加的目录(加载 SnapshotStore.latest,取 large_files 父目录,去重,上限 200);对每个监控目录 walk_dir_fast 算 size;调 scan_large_files 拿 Top-N;drive 信息复用 cleaner::get_disk_info("C:");timestamp 用 chrono Local::now().to_rfc3339();scan_type = "full"。
- [ ] Step 6: 实现 scan_monitor_dirs(dirs) -> Vec<MonitorEntry>。只重扫监控目录,不枚举大文件。秒级。
- [ ] Step 7: 集成测试(#[ignore] 标,需真实 C 盘):scan_full() 在干净 tempdir 替身下跑通,字段非空。手动验证。
- [ ] Step 8: git commit -m "feat(analyzer): full scan + Top-N large files + monitor rescan"

## Task 4: Diff 逻辑(含移动抑制)

**Files:** Modify src-tauri/src/analyzer.rs

- [ ] Step 1: 写失败测试覆盖四种情况。tempdir 构造 prev 和 curr 两份 MonitorSnapshot:
  - added:curr 有 prev 无的文件。
  - removed:prev 有 curr 无的文件。
  - changed:同一 path 大小变化。
  - moved:prev 有 file_a.bin(1000 字节),curr 没有 file_a.bin 但有 file_b.bin(1000 字节,同扩展名),应标 kind=moved 而非 added+removed。
  断言 delta_bytes 符号、pct 计算、kind 分类正确。
- [ ] Step 2: 确认失败。
- [ ] Step 3: 实现 diff(prev: &MonitorSnapshot, curr: &MonitorSnapshot) -> Vec<DirDelta>。对 monitor_dirs 和 large_files 分别按 path 键合并(用 HashMap)。changed:delta = curr - prev。移动抑制:先收集 prev-only 和 curr-only,对大小在正负 5% 内且同扩展名的配对标 moved,从 added/removed 移除。降序输出(delta_bytes 绝对值大的在前)。
- [ ] Step 4: 测试通过,覆盖四个 case。
- [ ] Step 5: 边界测试:pct 当 prev_bytes=0 时(纯新增)定义为 +infinity 或 100.0,选 100.0 避免浮点无穷。写测试固定这个行为。
- [ ] Step 6: git commit -m "feat(analyzer): diff with move suppression"
- [ ] Step 6: git commit -m "feat(analyzer): diff with move suppression"

## Task 5: 快照存储(SnapshotStore)

**Files:** Modify src-tauri/src/analyzer.rs

- [ ] Step 1: 写失败测试。tempdir 当快照根,save 两份 MonitorSnapshot(不同 timestamp),list 返回两条摘要,load(ts) 能读回字段。prune(keep_days=0) 删除旧的,只留最新。
- [ ] Step 2: 确认失败。
- [ ] Step 3: 实现 SnapshotStore。路径 %APPDATA%\DiskClearTool\snapshots 下按 unix 时间戳命名 .json(每份 < 20 KB)。struct 持有 Mutex 保护写入(对应 spec P1-1 并发竞态)。方法:save(&self, snap), list(&self) -> Vec<SnapshotSummary>(从 index.json 读), load(&self, ts) -> Option<MonitorSnapshot>, latest(&self) -> Option<MonitorSnapshot>, prune(&self, keep_days)。
- [ ] Step 4: 实现 index.json 维护。每次 save 重写 index.json = [(ts, scan_type, drive_used)],list 不解析全量快照。
- [ ] Step 5: 并发测试。#[ignore] 或用线程池同时调 save 两次,确认 index.json 不损坏(简化:测 Mutex 存在,真正并发验证靠代码审查)。最小测试:连续 save 100 次后 list 返回 100 条,顺序正确。
- [ ] Step 6: git commit -m "feat(analyzer): snapshot store with index and prune"

## Task 6: 配置存储 + Tauri 命令(lib.rs)

**Files:** Modify src-tauri/src/lib.rs;可能小改 analyzer.rs(暴露 SnapshotStore handle)

- [ ] Step 1: AppState 加 analyzer 相关字段。参考现有 AppState(config: ConfigStore, scheduler, history)。加 analyzer_store: Arc<SnapshotStore> 和 analyzer_config 路径(%APPDATA%\DiskClearTool\analyzer.json)。config 加载用 serde,与 ConfigStore 同模式。
- [ ] Step 2: 实现 7 个 #[tauri::command]:
  - analyze_full_scan() -> Result<MonitorSnapshot, String>:spawn_blocking 调 analyzer::scan_full(),存快照,返回。
  - analyze_rescan_monitors() -> Result<Vec<MonitorEntry>, String>:spawn_blocking 调 scan_monitor_dirs,不存快照。
  - analyze_get_latest() -> Option<MonitorSnapshot>:读 latest。
  - analyze_list_snapshots() -> Vec<SnapshotSummary>。
  - analyze_drilldown(path: String) -> Result<Vec<MonitorEntry>, String>:spawn_blocking 调 analyzer::list_children。
  - analyze_diff(prev_ts: String, curr_ts: String) -> Result<Vec<DirDelta>, String>:加载两份快照跑 diff。
  - analyze_get_config() -> AnalyzerConfig 和 analyze_set_config(cfg)。
- [ ] Step 3: 注册到 invoke_handler!(加这 8 个名字)。
- [ ] Step 4: 编译通过。手动用 tauri dev 或简单 #[test] 验证 analyze_full_scan 在真实环境能跑(标 #[ignore],需真实 C 盘 + 管理员)。
- [ ] Step 5: 进度事件。scan_full 内通过 app.emit("analyze-scan-progress", payload) 推进度;命令签名需 AppHandle 参数。payload 简化:每扫完一个监控目录发一次 {done, total}。
- [ ] Step 6: git commit -m "feat(analyzer): Tauri commands wiring + AppState"
- [ ] Step 6: git commit -m "feat(analyzer): Tauri commands wiring + AppState"

## Task 7: 前端 tab 骨架 + 类型定义(App.tsx / AnalyzerTab.tsx)

**Files:** Create src/AnalyzerTab.tsx;Modify src/App.tsx

- [ ] Step 1: AnalyzerTab.tsx 定义 TypeScript 接口,镜像 Rust 结构:MonitorEntry, LargeFileEntry, MonitorSnapshot, DirDelta, AnalyzerConfig, SnapshotSummary。
- [ ] Step 2: App.tsx 顶部加 tab state(type Tab = "cleanup" | "analyzer"),两个按钮切换。现有全部清理 UI 包进条件渲染 {tab === "cleanup" && (...)};{tab === "analyzer" && <AnalyzerTab />}。先让 AnalyzerTab 渲染占位文字,确认 tab 切换不崩。
- [ ] Step 3: npm run build 通过(tsc 类型检查)。
- [ ] Step 4: git commit -m "feat(ui): analyzer tab skeleton"

## Task 8: AnalyzerTab 完整实现(监控目录列表 + 大文件 + diff + 配置)

**Files:** Modify src/AnalyzerTab.tsx

- [ ] Step 1: 实现顶部操作条:全盘扫描、重扫监控目录、快照选择器(对比) 三个按钮 + 一个下拉。调对应 invoke 命令,loading 状态用 useState。
- [ ] Step 2: 快照摘要区。展示最新快照的 drive_used/total、timestamp、scan_type。复用 App.tsx 的 formatBytes(抽到 utils.ts 共享,或复制)。
- [ ] Step 3: 监控目录列表。useEffect 加载 analyze_get_latest,渲染 monitor_dirs 降序。每行:路径、大小、占比进度条(样式复用 .disk-bar)、增量列(有 diff 时红涨绿降)。点击行调 analyze_drilldown 展开,子项内联显示。虚拟滚动:列表条数预期几十到两百,先不做虚拟化,超 500 条再加 react-window(spec 提了,但 v1 监控目录上限 200,先简化)。
- [ ] Step 4: 大文件清单。渲染 large_files,每行路径 + 大小。系统大文件(pagefile/hiberfil/swapfile)标灰色(后缀匹配)。
- [ ] Step 5: diff 视图。选两份快照后调 analyze_diff,监控目录列表右侧显示增量列;大文件清单切到"新增/消失/移动"三个分组展示。
- [ ] Step 6: 配置面板(可折叠)。展示 AnalyzerConfig,支持增删监控目录(输入框 + 添加/删除)、改阈值(数字输入)、Top-N、保留天数。保存调 analyze_set_config。
- [ ] Step 7: 进度事件。listen("analyze-scan-progress") 更新进度条(复用现有 loading 文案)。
- [ ] Step 8: npm run build 通过。
- [ ] Step 9: git commit -m "feat(ui): analyzer tab full implementation"

## Task 9: 端到端验证 + 文档

**Files:** README.md(使用说明追加)

- [ ] Step 1: cargo test --manifest-path src-tauri/Cargo.toml --lib 全绿(所有 analyzer 测试通过,ignored 的手动项列清单)。
- [ ] Step 2: npm run build 通过。
- [ ] Step 3: 手动验证清单(管理员运行 tauri dev):
  - 首次进占用分析 tab,点全盘扫描,确认进度条动、结束后列表填充合理大小。
  - 点某个监控目录行,确认 drilldown 展开一层子目录。
  - 点重扫监控目录,确认秒级返回。
  - 改大文件(造个 1GB 临时文件),再全盘扫,选两份快照 diff,确认增量列正确显示。
  - 改配置(加个监控目录、改阈值),保存,再扫确认生效。
- [ ] Step 4: README 追加"占用分析"使用说明(扫描、下钻、diff、配置)。
- [ ] Step 5: git commit -m "docs: analyzer v1 usage guide"
- [ ] Step 6: 打 tag v0.2.0,推 GitHub Actions 云端打包(参考现有 build.yml 流程)。

---

## 风险与回退点

- **FindFirstFileExW FFI 细节**:windows crate 0.58 的 IntoParam/Param 接口在 walker 实现时可能踩坑(类似之前 SHEmptyRecycleBinW 的经验)。若卡住,fallback 到 walkdir + metadata 先跑通 v1,性能优化放 v1.1。这是明确允许的降级,spec 的性能目标是"SSD < 5 分钟",walkdir 也能达标只是慢些。
- **监控目录 env 展开**:UAC 下 %LOCALAPPDATA% 应指向当前用户,但若 SchTasks 以 SYSTEM 运行(headless 模式)会指向 SYSTEM profile。v1 只有交互式 UI 用分析器,不接 headless,故无此问题。记录到 spec 假设里。
- **大文件 walker 输出设计**:Task 3 Step 3 提到 walk_dir_collect,若与 walk_dir_fast 合并导致 walker 接口复杂,拆成两个函数(walk_dir_fast 只算汇总;walk_dir_collect 同时收集文件清单),测试各自覆盖。
