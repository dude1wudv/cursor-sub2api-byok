//! Read-only Cursor process detection used to guard reversible transitions.

use tokio::process::Command;

use crate::{Error, Result};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(windows)]
fn hide_console(command: &mut Command) {
    command.creation_flags(CREATE_NO_WINDOW);
}

pub async fn cursor_running() -> Result<bool> {
    cursor_running_platform().await
}

#[cfg(target_os = "windows")]
async fn cursor_running_platform() -> Result<bool> {
    let mut list = Command::new("tasklist");
    hide_console(&mut list);
    let output = list
        .args(["/FI", "IMAGENAME eq Cursor.exe", "/NH", "/FO", "CSV"])
        .output()
        .await?;
    if !output.status.success() {
        return Err(Error::Config(
            "failed to inspect the Cursor.exe process".into(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .to_ascii_lowercase()
        .contains("cursor.exe"))
}

#[cfg(target_os = "macos")]
async fn cursor_running_platform() -> Result<bool> {
    process_running("Cursor").await
}

#[cfg(target_os = "linux")]
async fn cursor_running_platform() -> Result<bool> {
    Ok(process_running("cursor").await? || process_running("Cursor").await?)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
async fn process_running(name: &str) -> Result<bool> {
    let status = Command::new("pgrep").args(["-x", name]).status().await?;
    match status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(Error::Config(format!(
            "failed to inspect the {name} process"
        ))),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
async fn cursor_running_platform() -> Result<bool> {
    Err(Error::Config(format!(
        "Cursor process inspection is unsupported on {}",
        std::env::consts::OS
    )))
}
