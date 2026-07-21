use crate::models::{ScheduleConfig, ScheduleFrequency};
use std::process::Command;

const TASK_NAME: &str = "DiskClearTool_AutoClean";

pub fn register(config: &ScheduleConfig) -> Result<String, String> {
    if !cfg!(windows) {
        return Err("仅支持 Windows".to_string());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_str = exe.to_string_lossy().to_string();

    let schedule = match config.frequency {
        ScheduleFrequency::Daily => "DAILY",
        ScheduleFrequency::Weekly => "WEEKLY",
    };
    let start_time = format!("{:02}:{:02}", config.hour.min(23), config.minute.min(59));

    let category_args: Vec<String> = config
        .categories
        .iter()
        .map(|c| format!("{:?}", c))
        .collect();
    let tr_command = format!(
        "\"{}\" --headless-clean {}",
        exe_str,
        category_args.join(" ")
    );

    let output = Command::new("schtasks")
        .args([
            "/Create",
            "/F",
            "/SC",
            schedule,
            "/TN",
            TASK_NAME,
            "/TR",
            &tr_command,
            "/ST",
            &start_time,
            "/RL",
            "HIGHEST",
        ])
        .output()
        .map_err(|e| format!("无法启动 schtasks: {e}"))?;

    if output.status.success() {
        Ok(format!(
            "已注册系统计划任务 \"{}\"，将在每{}于 {} 执行",
            TASK_NAME,
            if config.frequency == ScheduleFrequency::Daily {
                "天"
            } else {
                "周"
            },
            start_time
        ))
    } else {
        Err(format!(
            "schtasks 失败: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

pub fn unregister() -> Result<String, String> {
    if !cfg!(windows) {
        return Err("仅支持 Windows".to_string());
    }
    let output = Command::new("schtasks")
        .args(["/Delete", "/F", "/TN", TASK_NAME])
        .output()
        .map_err(|e| format!("无法启动 schtasks: {e}"))?;

    if output.status.success() {
        Ok(format!("已移除系统计划任务 \"{}\"", TASK_NAME))
    } else {
        Err(format!(
            "schtasks 失败: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}
