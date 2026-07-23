use crate::models::CleanAdvice;

/// Classify a file or directory path into a cleanability level with a short
/// human-readable reason. Path matching is SEGMENT-exact: a rule for the
/// `Cache` directory matches the path component `cache` but not `mycache` or
/// `cachex`, so user folders with cache-like names are never misread as safe.
/// Checks run most-specific-first: a file inside `\Cache\` wins over the
/// broader `\AppData\` rule, etc.
pub fn classify(path: &str) -> (CleanAdvice, String) {
    let lower = path.to_lowercase().replace('/', r"\");
    let filename = lower.rsplit('\\').next().unwrap_or("");

    // ---- 1. System critical files: never touch ----
    match filename {
        "pagefile.sys" => return keep("系统页面文件，运行时占用，不建议手动删除".into()),
        "swapfile.sys" => return keep("系统交换文件，不建议手动删除".into()),
        "hiberfil.sys" => return caution("休眠文件，可用 powercfg /h off 关闭休眠来释放".into()),
        "ntuser.dat" | "ntuser.dat.log1" | "ntuser.dat.log2" => {
            return keep("用户注册表配置单元，删除会导致用户配置损坏".into())
        }
        "bootmgr" | "bootsect.bak" => return keep("启动引导文件，删除会导致无法开机".into()),
        _ => {}
    }

    // ---- 2. Known-safe locations: cache / temp / logs / dumps ----
    if let Some(reason) = safe_cache_reason(&lower, filename) {
        return safe(reason);
    }

    // ---- 3. Downloads folder: caution (user may want these) ----
    if has_seg(&lower, "downloads") {
        let ext = ext_lower(filename);
        if matches!(ext.as_str(), "exe" | "msi" | "msix" | "iso" | "zip" | "rar" | "7z" | "pkg") {
            return caution("下载的安装包/压缩包，确认不再需要后可删".into());
        }
        return caution("下载目录文件，请确认后删除".into());
    }

    // ---- 4. Windows.old: safe to remove ----
    if is_under(&lower, r"c:\windows.old") {
        return safe("旧系统备份，确认不需要回退后可删".into());
    }

    // ---- 5. Windows Installer: caution ----
    if is_under(&lower, r"c:\windows\installer") {
        if has_seg(&lower, "$patchcache$") {
            return safe("Installer 补丁缓存，安全可删".into());
        }
        return caution("Windows Installer 文件，删除可能影响软件卸载/修复".into());
    }

    // ---- 6. WinSxS: keep ----
    if has_seg(&lower, "winsxs") {
        return keep("组件存储，手动删除会损坏系统，可用 DISM 清理".into());
    }

    // ---- 7. Program Files: keep ----
    if is_under(&lower, r"c:\program files") || is_under(&lower, r"c:\program files (x86)") {
        return keep("已安装程序目录，删除会导致软件无法运行".into());
    }

    // ---- 8. Windows system core: keep ----
    if is_under(&lower, r"c:\windows\system32") || is_under(&lower, r"c:\windows\syswow64") {
        return keep("Windows 系统核心目录，不建议删除".into());
    }

    // ---- 9. $Recycle.Bin: safe ----
    if is_under(&lower, r"c:\$recycle.bin") {
        return safe("回收站内容，清空后不可恢复".into());
    }

    // ---- 10. Large media / docs in user profile: caution ----
    let ext = ext_lower(filename);
    if matches!(ext.as_str(), "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv") {
        return caution("视频文件，确认不需要后可删".into());
    }
    if matches!(ext.as_str(), "iso" | "vmdk" | "vdi" | "qcow2") {
        return caution("磁盘镜像文件，确认不需要后可删".into());
    }

    // ---- 11. Default: unknown ----
    (CleanAdvice::Unknown, "无法判断，请自行评估".into())
}

/// True if `seg` appears as an exact `\`-delimited path component in `lower`
/// (already lowercased, with `/` normalized to `\`). Segment-precise: "cache"
/// matches the component `cache` but never `mycache` or `cachex`. The leading
/// drive token (e.g. `c:`) is just another component, which is harmless since
/// no rule keys on it.
fn has_seg(lower: &str, seg: &str) -> bool {
    lower.split('\\').any(|p| p == seg)
}

/// True if a `child` component appears immediately after a `parent` component
/// somewhere in the path (`...\parent\child\...` or ending `...\parent\child`).
/// Both names must match their components exactly; this is what makes
/// `softwaredistribution\download` precise (the plural `downloads` won't match).
fn has_seg_child(lower: &str, parent: &str, child: &str) -> bool {
    let mut prev_match = false;
    for p in lower.split('\\') {
        if prev_match && p == child {
            return true;
        }
        prev_match = p == parent;
    }
    false
}

/// True if `lower` is exactly `dir` or lives under it (`dir\...`). Replaces the
/// old `starts_with("c:\\windows.old")` form that also matched
/// `c:\windows.oldbackup` because it lacked a trailing path delimiter.
fn is_under(lower: &str, dir: &str) -> bool {
    lower == dir || lower.starts_with(&format!("{}\\", dir))
}

/// Single source of truth for "known-safe" locations: returns the human reason
/// when the path matches a safe rule, else `None`. Merging the old
/// `is_safe_cache` gate and `safe_reason` lookup into one pass removes the drift
/// where the two could disagree. More specific rules (dev caches) run before
/// generic ones (temp) so the reason text stays accurate.
fn safe_cache_reason(lower: &str, filename: &str) -> Option<String> {
    // --- Dev caches (specific first, so their reason survives) ---
    if has_seg(lower, "npm-cache") {
        return Some("npm 缓存，安全可删（下次安装会重建）".into());
    }
    if has_seg_child(lower, "pip", "cache") {
        return Some("pip 缓存，安全可删（下次安装会重建）".into());
    }
    if has_seg(lower, ".cargo")
        && (has_seg_child(lower, "registry", "cache") || has_seg_child(lower, "registry", "src"))
    {
        return Some("Cargo 注册表缓存，安全可删".into());
    }
    if has_seg_child(lower, "nuget", "v3-cache") {
        return Some("NuGet 缓存，安全可删".into());
    }

    // --- Windows Update / delivery optimization ---
    if has_seg_child(lower, "softwaredistribution", "download") {
        return Some("Windows Update 下载缓存，安全可删".into());
    }
    if has_seg_child(lower, "softwaredistribution", "deliveryoptimization") {
        return Some("传递优化缓存，安全可删".into());
    }

    // --- Browser / app caches (segment-exact: "MyCache" no longer matches) ---
    if has_seg(lower, "cache")
        || has_seg(lower, "code cache")
        || has_seg(lower, "gpucache")
        || has_seg(lower, "shadercache")
        || has_seg(lower, "grshadercache")
        || has_seg(lower, "dawncache")
        || has_seg(lower, "dawngraphitecache")
        || has_seg(lower, "service worker")
        || has_seg(lower, "startupcache")
    {
        return Some("浏览器/应用缓存，安全可删".into());
    }

    // --- WER ---
    if has_seg(lower, "wer") {
        return Some("Windows 错误报告，安全可删".into());
    }

    // --- Crash dumps ---
    if has_seg(lower, "minidump")
        || has_seg(lower, "crashdumps")
        || has_seg(lower, "livekernelreports")
        || filename == "memory.dmp"
        || filename.ends_with(".dmp")
    {
        return Some("崩溃转储文件，安全可删（删除丢失崩溃现场）".into());
    }

    // --- Prefetch ---
    if has_seg(lower, "prefetch") || filename.ends_with(".pf") {
        return Some("预读文件，系统会自动重建".into());
    }

    // --- Panther (upgrade logs) ---
    if has_seg(lower, "panther") {
        return Some("Windows 升级日志，安全可删".into());
    }

    // --- Windows log subdirs ---
    if has_seg_child(lower, "logs", "cbs") || has_seg_child(lower, "logs", "windowsupdate") {
        return Some("Windows 日志，安全可删".into());
    }

    // --- Generic log files ---
    if filename.ends_with(".log") {
        return Some("日志文件，安全可删".into());
    }

    // --- Thumbnails / icon cache ---
    if filename.starts_with("thumbcache_") || filename.starts_with("iconcache_") {
        return Some("缩略图/图标缓存，系统会自动重建".into());
    }

    // --- Temp (generic, last so dev caches above keep their own reason) ---
    if has_seg(lower, "temp") || has_seg(lower, "tmp") {
        return Some("临时文件，安全可删".into());
    }
    if filename.ends_with(".tmp") || filename.ends_with(".temp") {
        return Some("临时文件，安全可删".into());
    }

    None
}

fn ext_lower(filename: &str) -> String {
    match filename.rsplit_once('.') {
        Some((_, ext)) => ext.to_lowercase(),
        None => String::new(),
    }
}

fn safe(reason: String) -> (CleanAdvice, String) {
    (CleanAdvice::Safe, reason)
}
fn caution(reason: String) -> (CleanAdvice, String) {
    (CleanAdvice::Caution, reason)
}
fn keep(reason: String) -> (CleanAdvice, String) {
    (CleanAdvice::Keep, reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_files_are_keep() {
        let (a, _) = classify(r"C:\pagefile.sys");
        assert_eq!(a, CleanAdvice::Keep);
        let (a, _) = classify(r"C:\Windows\System32\drivers\etc\hosts");
        // hosts is in system32 -> keep
        assert_eq!(a, CleanAdvice::Keep);
    }

    #[test]
    fn hibernation_is_caution() {
        let (a, r) = classify(r"C:\hiberfil.sys");
        assert_eq!(a, CleanAdvice::Caution);
        assert!(r.contains("powercfg"));
    }

    #[test]
    fn temp_files_are_safe() {
        let (a, _) = classify(r"C:\Users\test\AppData\Local\Temp\foo.tmp");
        assert_eq!(a, CleanAdvice::Safe);
        let (a, _) = classify(r"C:\Windows\Temp\setup.log");
        assert_eq!(a, CleanAdvice::Safe);
    }

    #[test]
    fn browser_cache_is_safe() {
        let (a, r) = classify(r"C:\Users\test\AppData\Local\Google\Chrome\User Data\Default\Cache\data_0");
        assert_eq!(a, CleanAdvice::Safe);
        assert!(r.contains("缓存"));
    }

    #[test]
    fn program_files_are_keep() {
        let (a, _) = classify(r"C:\Program Files\Google\Chrome\chrome.exe");
        assert_eq!(a, CleanAdvice::Keep);
        let (a, _) = classify(r"C:\Program Files (x86)\Steam\steam.exe");
        assert_eq!(a, CleanAdvice::Keep);
    }

    #[test]
    fn downloads_installers_are_caution() {
        let (a, r) = classify(r"C:\Users\test\Downloads\setup.exe");
        assert_eq!(a, CleanAdvice::Caution);
        assert!(r.contains("安装包"));
    }

    #[test]
    fn windows_old_is_safe() {
        let (a, _) = classify(r"C:\Windows.old\Windows\System32\kernel32.dll");
        assert_eq!(a, CleanAdvice::Safe);
    }

    #[test]
    fn winsxs_is_keep() {
        let (a, _) = classify(r"C:\Windows\WinSxS\amd64_microsoft-windows-something");
        assert_eq!(a, CleanAdvice::Keep);
    }

    #[test]
    fn dmp_files_are_safe() {
        let (a, r) = classify(r"C:\Windows\Minidump\mini.dmp");
        assert_eq!(a, CleanAdvice::Safe);
        assert!(r.contains("转储"));
    }

    #[test]
    fn recycle_bin_is_safe() {
        let (a, _) = classify(r"C:\$Recycle.Bin\S-1-5-21\$R12345");
        assert_eq!(a, CleanAdvice::Safe);
    }

    #[test]
    fn unknown_file_is_unknown() {
        let (a, _) = classify(r"C:\Users\test\Documents\report.docx");
        assert_eq!(a, CleanAdvice::Unknown);
    }

    #[test]
    fn installer_patch_cache_is_safe() {
        let (a, _) = classify(r"C:\Windows\Installer\$PatchCache$\MSP\abc.msp");
        assert_eq!(a, CleanAdvice::Safe);
    }

    #[test]
    fn installer_base_is_caution() {
        let (a, _) = classify(r"C:\Windows\Installer\abc123.msi");
        assert_eq!(a, CleanAdvice::Caution);
    }

    #[test]
    fn video_files_are_caution() {
        let (a, _) = classify(r"C:\Users\test\Videos\movie.mp4");
        assert_eq!(a, CleanAdvice::Caution);
    }

    #[test]
    fn npm_cache_is_safe() {
        let (a, r) = classify(r"C:\Users\test\AppData\Local\npm-cache\_cacache\tmp\tmp.tar");
        assert_eq!(a, CleanAdvice::Safe);
        assert!(r.contains("npm"));
    }

    // ===== Segment-precision regression tests =====
    // These pin the fix for substring false-positives: a directory whose NAME
    // merely contains a safe keyword (rather than being exactly that keyword)
    // must not be classified Safe. Deletion is irreversible, so these guard
    // against the classifier green-lighting user data.

    #[test]
    fn mycache_substring_is_not_safe() {
        // Segment is "mycache", not "cache".
        let (a, _) = classify(r"C:\Users\test\Documents\MyCache\report.docx");
        assert_ne!(a, CleanAdvice::Safe);
    }

    #[test]
    fn mytemp_substring_is_not_safe() {
        // Segment is "mytemp", not "temp".
        let (a, _) = classify(r"C:\Users\test\Documents\MyTemp\notes.txt");
        assert_ne!(a, CleanAdvice::Safe);
    }

    #[test]
    fn windows_old_backup_is_not_safe() {
        // "windows.oldbackup" must not match the "c:\windows.old" rule.
        let (a, _) = classify(r"C:\Windows.oldbackup\foo.dll");
        assert_ne!(a, CleanAdvice::Safe);
    }

    #[test]
    fn windows_installer_variant_is_not_caution() {
        // "installer-backup" is a different segment than "installer".
        let (a, _) = classify(r"C:\Windows\installer-backup\thing.msi");
        assert_ne!(a, CleanAdvice::Caution);
    }

    #[test]
    fn recycle_bin_variant_is_not_safe() {
        // "$recycle.bin.bak" must not match "c:\$recycle.bin".
        let (a, _) = classify(r"C:\$Recycle.Bin.bak\foo");
        assert_ne!(a, CleanAdvice::Safe);
    }

    #[test]
    fn crashdumps_prefix_dir_is_not_safe() {
        // "crashdumps-archive" is not the "crashdumps" segment.
        let (a, _) = classify(r"C:\Users\test\AppData\Local\CrashDumps-Archive\report.txt");
        assert_ne!(a, CleanAdvice::Safe);
    }

    #[test]
    fn softwaredistribution_downloads_plural_is_not_safe() {
        // Canonical cache is ...\Download (singular); plural must not match.
        let (a, _) = classify(r"C:\Windows\SoftwareDistribution\Downloads\foo.exe");
        assert_ne!(a, CleanAdvice::Safe);
    }

    #[test]
    fn exact_cache_segment_still_safe() {
        // Positive control: a real "Cache" segment still classifies Safe.
        let (a, _) = classify(r"C:\Users\test\AppData\Local\Google\Chrome\User Data\Default\Cache\data_1");
        assert_eq!(a, CleanAdvice::Safe);
    }
}
