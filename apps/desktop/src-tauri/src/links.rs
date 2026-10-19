//! Hyperlinks and the note author (WP7): `open_url` hands a web or mail
//! link to the OS default handler after strict validation, and `user_name`
//! returns the OS account name for new notes.
//!
//! `open_url` accepts only `http://host…`, `https://host…` and
//! `mailto:address…` URLs of at most [`MAX_URL`] bytes without whitespace,
//! control characters, quotes or backslashes. Everything else (`file:`,
//! `javascript:`, `data:`, custom schemes, relative paths, `#Sheet.A1`
//! links, which the UI handles itself) is `invalid_argument`. The URL is
//! passed as one argument to `open` (macOS), `xdg-open` (Linux) or
//! `cmd /C start "" <url>` (Windows, with cmd metacharacters `^`-escaped);
//! no shell parses it on macOS and Linux. See docs/security.md
//! ("Hyperlinks").

use std::process::Command;

use tally_api::ApiError;

use crate::commands::CmdResult;

/// Longest URL `open_url` accepts, in bytes.
pub const MAX_URL: usize = 2048;

/// Longest user name returned by `user_name`, in characters.
const MAX_USER: usize = 256;

fn bad(message: &str) -> ApiError {
    ApiError::invalid_argument(message)
}

/// Validates a URL for `open_url`; returns it trimmed.
pub fn check_url(url: &str) -> CmdResult<String> {
    let url = url.trim();
    if url.is_empty() {
        return Err(bad("the link is empty"));
    }
    if url.len() > MAX_URL {
        return Err(bad("the link is too long (the limit is 2048 bytes)"));
    }
    if url.chars().any(|ch| {
        ch.is_whitespace() || ch.is_control() || matches!(ch, '"' | '\\' | '<' | '>' | '`')
    }) {
        return Err(bad("the link contains characters that are not allowed"));
    }
    let lower = url.to_ascii_lowercase();
    let rest = if let Some(r) = lower.strip_prefix("https://") {
        r
    } else if let Some(r) = lower.strip_prefix("http://") {
        r
    } else if let Some(r) = lower.strip_prefix("mailto:") {
        if r.is_empty() || r.starts_with('/') {
            return Err(bad("the mail link has no address"));
        }
        return Ok(url.to_owned());
    } else {
        return Err(bad(
            "only web (http, https) and mail (mailto) links can be opened",
        ));
    };
    // A host must follow the scheme (no `http:///path`, no `http://@x`).
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .rsplit('@')
        .next()
        .unwrap_or("");
    if host.is_empty() || host.starts_with(':') {
        return Err(bad("the web link has no host"));
    }
    Ok(url.to_owned())
}

/// The command that opens `url` with the OS default handler.
pub fn launcher(url: &str) -> Command {
    if cfg!(target_os = "windows") {
        let mut escaped = String::with_capacity(url.len() + 8);
        for ch in url.chars() {
            if matches!(ch, '^' | '&' | '|' | '<' | '>' | '%' | '(' | ')' | '!') {
                escaped.push('^');
            }
            escaped.push(ch);
        }
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "start", ""]).arg(escaped);
        cmd
    } else if cfg!(target_os = "macos") {
        let mut cmd = Command::new("open");
        cmd.arg(url);
        cmd
    } else {
        let mut cmd = Command::new("xdg-open");
        cmd.arg(url);
        cmd
    }
}

/// Validates and opens `url`, via `launch` (the OS launcher in the app,
/// a recorder in tests).
pub fn open_url_with(url: &str, launch: impl FnOnce(&str) -> CmdResult<()>) -> CmdResult<()> {
    let url = check_url(url)?;
    launch(&url)
}

/// Runs the OS launcher for an already validated URL.
pub fn launch_os(url: &str) -> CmdResult<()> {
    let status = launcher(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| ApiError::new(ApiError::IO, format!("cannot open the link: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(ApiError::new(ApiError::IO, "the link could not be opened"))
    }
}

/// The OS account name (`USER`, `USERNAME` or `LOGNAME`), for note authors.
pub fn user_name() -> Option<String> {
    ["USER", "USERNAME", "LOGNAME"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .map(|s| {
            s.trim()
                .chars()
                .filter(|c| !c.is_control())
                .take(MAX_USER)
                .collect::<String>()
        })
        .find(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_web_and_mail_links() {
        for ok in [
            "https://example.com",
            "http://example.com/a?b=1&c=%20#x",
            "HTTPS://Example.com:8080/path",
            "mailto:someone@example.com?subject=Hi",
            "  https://example.com/trimmed  ",
        ] {
            assert!(check_url(ok).is_ok(), "{ok}");
        }
    }

    #[test]
    fn rejects_everything_else() {
        for bad in [
            "",
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,<b>x</b>",
            "vbscript:msgbox",
            "ftp://example.com",
            "#Sheet1.A1",
            "example.com",
            "http://",
            "http:///etc/passwd",
            "https://a b.com",
            "https://example.com/\"quoted\"",
            "https://example.com/\u{7}",
            "mailto:",
            "smb://server/share",
        ] {
            let e = check_url(bad).unwrap_err();
            assert_eq!(e.code, ApiError::INVALID_ARGUMENT, "{bad}");
        }
        let long = format!("https://example.com/{}", "a".repeat(MAX_URL));
        assert!(check_url(&long).is_err());
    }

    #[test]
    fn open_url_with_only_launches_valid_links() {
        let mut seen = Vec::new();
        assert!(open_url_with("file:///tmp/x", |u| {
            seen.push(u.to_owned());
            Ok(())
        })
        .is_err());
        assert!(open_url_with(" https://example.com ", |u| {
            seen.push(u.to_owned());
            Ok(())
        })
        .is_ok());
        assert_eq!(seen, vec!["https://example.com".to_owned()]);
    }

    #[test]
    fn launcher_passes_the_url_as_one_argument() {
        let cmd = launcher("https://example.com/?a=1&b=2");
        let args: Vec<_> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        if cfg!(target_os = "windows") {
            assert_eq!(
                args.last().map(String::as_str),
                Some("https://example.com/?a=1^&b=2")
            );
        } else {
            assert_eq!(args, vec!["https://example.com/?a=1&b=2".to_owned()]);
        }
    }
}
