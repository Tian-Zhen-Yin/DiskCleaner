# 分析器文件删除功能 - 设计文档

## 摘要

在占用分析器的大文件列表中,为 `Safe`(安全可删)类文件提供直接删除能力。支持单个删除和一键批量删除,删除后前端局部实时更新,无需重新扫描。

## 需求

- 仅 `CleanAdvice::Safe` 类文件可从分析器删除;Caution / Keep / Unknown 不提供删除入口
- 单个删除:每个 Safe 文件旁边有删除按钮,点击直接删除,不弹出确认框(用户主动点击即表示删除意图)
- 批量删除:汇总行有「一键清理全部安全可删」按钮,点击弹确认框(显示将删 X 个文件 Y GB),确认后执行
- 删除后前端从大文件列表移除已删项,汇总数字(安全可删 X 个 / Y GB)实时递减
- 批量删除仅针对当前大文件列表中的 Safe 项,不涉及监控目录下钻数据

## 安全设计

**后端安全门**:删除命令在后端对每个路径调 `classify::classify()` 做二次校验,非 `Safe` 的路径拒绝删除并进 errors 列表。前端无法绕过此校验。

**不可逆操作**:删除使用 `std::fs::remove_file`,不经过回收站。Safe 类均为缓存/临时文件,系统或应用会自动重建,删除可接受。

## 后端改动

### models.rs

新增 `DeleteResult`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteResult {
    pub deleted_count: u64,
    pub freed_bytes: u64,
    pub errors: Vec<String>,
}
```

### lib.rs

新增 Tauri 命令 `analyze_delete_files`:

```rust
#[tauri::command]
async fn analyze_delete_files(
    app: tauri::AppHandle,
    paths: Vec<String>,
) -> Result<DeleteResult, String>
```

逻辑:
1. 遍历 `paths`,对每个路径调 `classify::classify()` 校验
2. 非 `Safe` 的路径进 errors(`"拒绝删除(非安全类): {path}"`),跳过
3. `Safe` 的路径:读取文件大小 -> `std::fs::remove_file` -> 成功则累加 `freed_bytes` + `deleted_count`,失败则进 errors
4. 每删完一个文件发送 `analyze-delete-progress` 事件 `{ deleted, total, freed_bytes }`
5. 返回 `DeleteResult`

单删和批删共用此命令:单删传 `vec![path]`,批删传全部 Safe 路径。

### 不改动 cleaner.rs

删除逻辑内联在 `analyze_delete_files` 中,因为:
- 需要先做 `classify` 校验,`cleaner.rs` 的 `clean_targets` 不做校验
- 只涉及单文件删除(`remove_file`),不需要 `CleanTarget` 枚举的 Dir/Files/RecycleBin 变体
- 代码量很小(遍历 + remove_file),抽象成共享函数收益不大

## 前端改动

### types.ts

```typescript
export interface DeleteResult {
  deleted_count: number;
  freed_bytes: number;
  errors: string[];
}
```

### AnalyzerTab.tsx

**新增状态**:
- `deleting: boolean` -- 删除进行中,禁用按钮
- `deleteProgress: { deleted: number; total: number; freed_bytes: number } | null` -- 批量删除进度

**单个删除按钮**:
- 每个 Safe 类大文件项右侧加一个删除按钮(lucide `Trash2` 图标,红色)
- 点击调 `invoke<DeleteResult>("analyze_delete_files", { paths: [f.path] })`
- 成功后:从 `latest.large_files` 中移除该项,`safeFiles` / `safeBytes` 通过 `useMemo` 自动重算
- 失败:pushLog 错误信息

**批量删除按钮**:
- 汇总行加「一键清理全部安全可删」按钮,仅当 `safeFiles.length > 0` 时显示
- 点击:弹 `window.confirm` 确认框,文案 `确定删除 ${safeFiles.length} 个安全可删文件,共 ${formatBytes(safeBytes)}?此操作不可撤销。`
- 确认后:调 `invoke<DeleteResult>("analyze_delete_files", { paths: safeFiles.map(f => f.path) })`
- 监听 `analyze-delete-progress` 事件更新进度
- 成功后:从 `latest.large_files` 中移除所有已删路径
- 部分失败:pushLog 报告 errors

**局部更新逻辑**(不触发任何重扫描,纯前端状态变更):
- 维护一个 `deletedPaths: Set<string>` 状态,初始为空
- 删除成功后把已删路径加入 Set
- 大文件列表渲染时用 `.filter(f => !deletedPaths.has(f.path))` 过滤掉已删文件
- 汇总数字(`safeFiles` / `safeBytes`)基于过滤后的列表通过 `useMemo` 自动重算
- 不调用任何后端扫描命令,下次用户手动全盘扫描时数据自然刷新

**UI 防护**:
- 删除进行中所有删除按钮 disabled
- 批量删除时显示进度 `删除中 ${deleted}/${total}`

## 测试计划

### 后端单元测试

在 `lib.rs` 或新建 `delete.rs` 模块中测试核心删除函数(提取为纯函数便于测试):

```rust
#[cfg(test)]
mod tests {
    // 1. 删除 Safe 类文件成功,返回正确 freed_bytes
    // 2. 非 Safe 路径被拒绝,进 errors 不删除
    // 3. 文件不存在时进 errors 不 panic
    // 4. 混合 Safe + 非 Safe:只删 Safe,非 Safe 进 errors
}
```

用 tempdir 创建临时文件,路径设为包含 `\Cache\` 或 `\Temp\` 以确保 classify 返回 Safe。

### 前端验证

- `npm run build`(tsc + vite)类型检查通过
- 手动:扫描后点单个删除,确认列表项消失、汇总数字递减
- 手动:点批量删除,确认弹框、确认后列表清空 Safe 项
- 手动:删除不存在的文件(先手动删再点删除按钮),确认 error 进日志不崩溃

## 假设与约束

- 删除不经过回收站,直接 `remove_file`。Safe 类均为可重建的缓存文件。
- 删除命令需要管理员权限(应用 manifest 已 `requireAdministrator`)。
- `classify::classify()` 的路径匹配是大小写不敏感的(已实现),路径分隔符统一为 `\`。
- 旧版本前端不含删除按钮,向后兼容无影响。
- `DeleteResult` 是新增类型,不影响现有序列化。
