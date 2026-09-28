use std::io::Read;
use std::time::Duration;

#[cfg(unix)]
pub fn current_parent_pid() -> Option<u32> {
    let pid = unsafe { libc::getppid() };
    (pid > 0).then_some(pid as u32)
}

#[cfg(windows)]
pub fn current_parent_pid() -> Option<u32> {
    let output = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!(
                "(Get-CimInstance Win32_Process -Filter \"ProcessId = {}\").ParentProcessId",
                std::process::id()
            ),
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u32>()
        .ok()
}

#[cfg(unix)]
pub fn is_process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(windows)]
pub fn is_process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {}", pid)])
        .output()
        .map(|output| {
            output.status.success()
                && String::from_utf8_lossy(&output.stdout).contains(&pid.to_string())
        })
        .unwrap_or(false)
}

pub fn spawn_parent_guard(parent_pid: u32) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(5));
        if !is_process_alive(parent_pid) {
            tracing::warn!("Parent process {} died, seshbuddy-proxy self-terminating", parent_pid);
            std::process::exit(0);
        }
    });
}

pub fn spawn_stdin_guard() {
    std::thread::spawn(|| {
        let mut buf = [0u8; 1];
        match std::io::stdin().read(&mut buf) {
            Ok(0) => {
                tracing::warn!("stdin closed, seshbuddy-proxy self-terminating");
                std::process::exit(0);
            }
            Ok(_) => {}
            Err(err) => {
                tracing::warn!("stdin guard read failed: {}", err);
            }
        }
    });
}

fn stdin_guard_enabled() -> bool {
    std::env::var("SESHBUDDY_PROXY_STDIN_GUARD")
        .or_else(|_| std::env::var("SESHBUDDY_PROXY_STDIN_GUARD"))
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

pub fn install_process_guards() {
    if let Some(parent_pid) = current_parent_pid() {
        spawn_parent_guard(parent_pid);
    } else {
        tracing::warn!("Unable to determine parent pid; parent guard disabled");
    }
    if stdin_guard_enabled() {
        spawn_stdin_guard();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_parent_pid_returns_some_process_id() {
        assert!(current_parent_pid().is_some());
    }

    #[test]
    fn current_process_is_alive() {
        let pid = std::process::id();
        assert!(is_process_alive(pid));
    }

    #[test]
    fn stdin_guard_is_opt_in() {
        std::env::remove_var("SESHBUDDY_PROXY_STDIN_GUARD");
        std::env::remove_var("SESHBUDDY_PROXY_STDIN_GUARD");
        assert!(!stdin_guard_enabled());

        std::env::set_var("SESHBUDDY_PROXY_STDIN_GUARD", "1");
        assert!(stdin_guard_enabled());

        std::env::set_var("SESHBUDDY_PROXY_STDIN_GUARD", "true");
        assert!(stdin_guard_enabled());

        std::env::set_var("SESHBUDDY_PROXY_STDIN_GUARD", "0");
        assert!(!stdin_guard_enabled());

        std::env::remove_var("SESHBUDDY_PROXY_STDIN_GUARD");
    }
}
