use std::process::Command;

const RUN_KEY_PATH: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE_NAME: &str = "DiskClearTool";

pub fn is_enabled() -> bool {
    let output = Command::new("reg")
        .args([
            "query",
            RUN_KEY_PATH,
            "/v",
            RUN_VALUE_NAME,
        ])
        .output();
    match output {
        Ok(o) => o.status.success(),
        Err(_) => false,
    }
}

pub fn enable(silent: bool) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_str = exe.to_string_lossy().to_string();
    let command = if silent {
        format!(
            "\"{}\" --headless-clean WindowsTemp BrowserCache WindowsUpdate",
            exe_str
        )
    } else {
        format!("\"{}\"", exe_str)
    };

    let output = Command::new("reg")
        .args([
            "add",
            RUN_KEY_PATH,
            "/v",
            RUN_VALUE_NAME,
            "/t",
            "REG_SZ",
            "/d",
            &command,
            "/f",
        ])
        .output()
        .map_err(|e| format!("无法启动 reg.exe: {e}"))?;

    if output.status.success() {
        Ok(if silent {
            "已启用开机自启（静默清理模式）".to_string()
        } else {
            "已启用开机自启（启动主界面）".to_string()
        })
    } else {
        Err(format!(
            "reg add 失败: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

pub fn disable() -> Result<String, String> {
    let output = Command::new("reg")
        .args([
            "delete",
            RUN_KEY_PATH,
            "/v",
            RUN_VALUE_NAME,
            "/f",
        ])
        .output()
        .map_err(|e| format!("无法启动 reg.exe: {e}"))?;

    if output.status.success() {
        Ok("已取消开机自启".to_string())
    } else {
        Err(format!(
            "reg delete 失败: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}
