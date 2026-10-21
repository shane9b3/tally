//! Window chrome (W5-5, §4 item 48): on Windows the main window runs without
//! OS decorations (`tauri.windows.conf.json`) and the React title bar in
//! `apps/desktop/src/TitleBar.tsx` draws the caption buttons, driven by the
//! `core:window:*` permissions of `capabilities/windows-titlebar.json`.
//! `window_chrome` tells the UI which chrome to draw; `setup` restores the
//! DWM shadow and resize borders that go away with the decorations. macOS
//! keeps its overlay title bar and Linux its native one.

use serde::Serialize;
use tauri::{Runtime, WebviewWindow};

/// What the UI asks on start (`window_chrome`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowChrome {
    /// The React title bar replaces the OS one.
    pub custom_title_bar: bool,
    /// `macos`, `windows`, `linux` or `other`.
    pub platform: &'static str,
}

/// The chrome for an OS name (`std::env::consts::OS`).
pub fn chrome_for(os: &str) -> WindowChrome {
    let platform = match os {
        "macos" | "windows" | "linux" => os,
        _ => "other",
    };
    let platform: &'static str = match platform {
        "macos" => "macos",
        "windows" => "windows",
        "linux" => "linux",
        _ => "other",
    };
    WindowChrome {
        custom_title_bar: os == "windows",
        platform,
    }
}

/// Platform setup of the main window (called from `lib.rs` setup).
pub fn setup<R: Runtime>(window: &WebviewWindow<R>) {
    if cfg!(target_os = "windows") {
        // Decorations are off: keep the DWM shadow so the window still has a visible edge.
        if let Err(e) = window.set_shadow(true) {
            eprintln!("tally: window shadow: {e}");
        }
    }
}

/// `window_chrome`: which title bar the UI draws.
#[tauri::command]
pub fn window_chrome() -> WindowChrome {
    chrome_for(std::env::consts::OS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_windows_gets_the_custom_title_bar() {
        assert_eq!(
            chrome_for("windows"),
            WindowChrome {
                custom_title_bar: true,
                platform: "windows"
            }
        );
        assert!(!chrome_for("macos").custom_title_bar);
        assert_eq!(chrome_for("macos").platform, "macos");
        assert!(!chrome_for("linux").custom_title_bar);
        assert_eq!(chrome_for("freebsd").platform, "other");
    }
}
