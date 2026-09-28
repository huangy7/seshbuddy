#[cfg(unix)]
use portable_pty::CommandBuilder;
#[cfg(unix)]
use std::collections::HashMap;

pub const STARTUP_ENV_KEY: &str = "SESHBUDDY_INTERNAL_STARTUP_CMD";

pub fn posix_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub fn build_startup_command(cli_path: &str, args: &[String]) -> String {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(posix_quote(cli_path));
    parts.extend(args.iter().map(|arg| posix_quote(arg)));
    parts.join(" ")
}

#[cfg(unix)]
pub fn build_posix_shell_command(
    shell_path: &str,
    startup_command: &str,
    env: &HashMap<String, String>,
) -> CommandBuilder {
    let mut cmd = CommandBuilder::new(shell_path);
    let shell_name = std::path::Path::new(shell_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    match shell_name {
        "zsh" | "bash" => {
            cmd.arg("-l");
            cmd.arg("-i");
            cmd.arg("-c");
            cmd.arg(format!("eval \"${}\"", STARTUP_ENV_KEY));
        }
        "fish" => {
            cmd.arg("-l");
            cmd.arg("-i");
            cmd.arg("-c");
            cmd.arg(format!("eval ${}", STARTUP_ENV_KEY));
        }
        _ => {
            cmd.arg("-l");
            cmd.arg("-c");
            cmd.arg(format!("eval \"${}\"", STARTUP_ENV_KEY));
        }
    }

    for (key, value) in env {
        cmd.env(key, value);
    }
    cmd.env(STARTUP_ENV_KEY, startup_command);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_quote_handles_spaces_and_single_quotes() {
        assert_eq!(posix_quote("/tmp/a b/claude"), "'/tmp/a b/claude'");
        assert_eq!(posix_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn startup_command_quotes_program_and_args() {
        let command = build_startup_command(
            "/Users/me/bin/claude",
            &["--settings".to_string(), "/tmp/a b.json".to_string()],
        );
        assert_eq!(
            command,
            "'/Users/me/bin/claude' '--settings' '/tmp/a b.json'"
        );
    }
}
