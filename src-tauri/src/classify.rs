use crate::models::CleanAdvice;

/// Classify a file or directory path into a cleanability level with a short
/// human-readable reason. The checks run most-specific-first: a file inside
/// `\Cache\` wins over the broader `\AppData\` rule, etc.
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
    if is_safe_cache(&lower, filename) {
        return safe(match safe_reason(&lower, filename) {
            Some(r) => r,
            None => "缓存文件，安全可删".into(),
        });
    }

    // ---- 3. Downloads folder: caution (user may want these) ----
    if lower.contains(r"\downloads\") {
        let ext = ext_lower(filename);
        if matches!(ext.as_str(), "exe" | "msi" | "msix" | "iso" | "zip" | "rar" | "7z" | "pkg") {
            return caution("下载的安装包/压缩包，确认不再需要后可删".into());
        }
        return caution("下载目录文件，请确认后删除".into());
    }

    // ---- 4. Windows.old: safe to remove ----
    if lower.starts_with(r"c:\windows.old") {
        return safe("旧系统备份，确认不需要回退后可删".into());
    }

    // ---- 5. Windows Installer: caution ----
    if lower.starts_with(r"c:\windows\installer") {
        if lower.contains(r"$patchcache$") {
            return safe("Installer 补丁缓存，安全可删".into());
        }
        return caution("Windows Installer 文件，删除可能影响软件卸载/修复".into());
    }

    // ---- 6. WinSxS: keep ----
    if lower.contains(r"\winsxs\") || lower.ends_with(r"\winsxs") {
        return keep("组件存储，手动删除会损坏系统，可用 DISM 清理".into());
    }

    // ---- 7. Program Files: keep ----
    if lower.starts_with(r"c:\program files\") || lower.starts_with(r"c:\program files (x86)\") {
        return keep("已安装程序目录，删除会导致软件无法运行".into());
    }

    // ---- 8. Windows system core: keep ----
    if lower.starts_with(r"c:\windows\system32\") || lower.starts_with(r"c:\windows\syswow64\") {
        return keep("Windows 系统核心目录，不建议删除".into());
    }

    // ---- 9. $Recycle.Bin: safe ----
    if lower.starts_with(r"c:\$recycle.bin") {
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

fn is_safe_cache(lower: &str, filename: &str) -> bool {
    // Temp directories
    lower.contains(r"\temp\")
        || lower.ends_with(r"\temp")
        || lower.contains(r"\tmp\")
        || lower.contains(r"\crashdumps")
        // Browser / app cache directories
        || lower.contains(r"\cache\")
        || lower.contains(r"\code cache\")
        || lower.contains(r"\gpucache\")
        || lower.contains(r"\shadercache\")
        || lower.contains(r"\grshadercache\")
        || lower.contains(r"\dawncache\")
        || lower.contains(r"\dawngraphitecache\")
        || lower.contains(r"\service worker\")
        || lower.contains(r"\startupcache\")
        // Windows Update / delivery optimization
        || lower.contains(r"\softwaredistribution\download")
        || lower.contains(r"\softwaredistribution\deliveryoptimization")
        // WER
        || lower.contains(r"\wer\")
        || lower.ends_with(r"\wer")
        // Dumps
        || lower.contains(r"\minidump\")
        || lower.contains(r"\livekernelreports")
        || filename == "memory.dmp"
        || filename.ends_with(".dmp")
        // Prefetch
        || lower.contains(r"\prefetch\")
        || filename.ends_with(".pf")
        // Panther (upgrade logs)
        || lower.contains(r"\panther\")
        // Log files
        || filename.ends_with(".log")
        || lower.contains(r"\logs\cbs\")
        || lower.contains(r"\logs\windowsupdate\")
        // Thumbnails
        || filename.starts_with("thumbcache_")
        || filename.starts_with("iconcache_")
        // Dev caches
        || lower.contains(r"\npm-cache\")
        || lower.contains(r"\pip\cache\")
        || lower.contains(r"\.cargo\registry\cache\")
        || lower.contains(r"\.cargo\registry\src\")
        || lower.contains(r"\nuget\v3-cache\")
        // Old temp extensions
        || filename.ends_with(".tmp")
        || filename.ends_with(".temp")
}

fn safe_reason(lower: &str, filename: &str) -> Option<String> {
    if lower.contains(r"\temp\") || lower.ends_with(r"\temp") {
        return Some("临时文件，安全可删".into());
    }
    if lower.contains(r"\cache\") || lower.contains(r"\code cache\")
        || lower.contains(r"\gpucache\") || lower.contains(r"\shadercache\")
        || lower.contains(r"\dawncache\") || lower.contains(r"\dawngraphitecache\")
        || lower.contains(r"\grshadercache\")
    {
        return Some("浏览器/应用缓存，安全可删".into());
    }
    if lower.contains(r"\softwaredistribution\download") {
        return Some("Windows Update 下载缓存，安全可删".into());
    }
    if lower.contains(r"\softwaredistribution\deliveryoptimization") {
        return Some("传递优化缓存，安全可删".into());
    }
    if lower.contains(r"\wer\") {
        return Some("Windows 错误报告，安全可删".into());
    }
    if lower.contains(r"\minidump\") || lower.contains(r"\livekernelreports")
        || filename == "memory.dmp" || filename.ends_with(".dmp")
    {
        return Some("崩溃转储文件，安全可删（删除丢失崩溃现场）".into());
    }
    if lower.contains(r"\prefetch\") || filename.ends_with(".pf") {
        return Some("预读文件，系统会自动重建".into());
    }
    if lower.contains(r"\panther\") {
        return Some("Windows 升级日志，安全可删".into());
    }
    if filename.ends_with(".log") {
        return Some("日志文件，安全可删".into());
    }
    if filename.starts_with("thumbcache_") || filename.starts_with("iconcache_") {
        return Some("缩略图/图标缓存，系统会自动重建".into());
    }
    if lower.contains(r"\npm-cache\") {
        return Some("npm 缓存，安全可删（下次安装会重建）".into());
    }
    if lower.contains(r"\pip\cache\") {
        return Some("pip 缓存，安全可删（下次安装会重建）".into());
    }
    if lower.contains(r"\.cargo\registry\") {
        return Some("Cargo 注册表缓存，安全可删".into());
    }
    if lower.contains(r"\nuget\v3-cache\") {
        return Some("NuGet 缓存，安全可删".into());
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
}