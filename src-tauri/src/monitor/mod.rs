//! Native activity monitoring (spec §8, §32): foreground window, idle, lock.
//! The probe reports facts; classification happens elsewhere.

pub mod demo;
#[cfg(not(windows))]
pub mod noop;
#[cfg(windows)]
pub mod windows;

/// One raw reading from the OS (or the demo simulator).
#[derive(Debug, Clone)]
pub struct RawSample {
    pub app_name: String,
    pub process_name: String,
    pub window_title: String,
    pub idle_seconds: u64,
    pub locked: bool,
    /// Only the demo probe fills these; on real hardware browser metadata
    /// comes from the extension bridge.
    pub browser_domain: Option<String>,
    pub browser_title: Option<String>,
}

pub enum ProbeReading {
    Sample(RawSample),
    /// Monitoring can't run (missing permission / unsupported OS).
    /// Only the non-Windows noop probe constructs this today.
    #[cfg_attr(windows, allow(dead_code))]
    Unavailable(String),
}

pub trait ActivityProbe: Send {
    fn read(&mut self) -> ProbeReading;
}

/// The real probe for this OS.
pub fn os_probe() -> Box<dyn ActivityProbe> {
    #[cfg(windows)]
    {
        Box::new(windows::WindowsProbe::new())
    }
    #[cfg(not(windows))]
    {
        Box::new(noop::NoopProbe)
    }
}

/// Friendly app name from a process executable name.
#[cfg_attr(not(windows), allow(dead_code))] // only the Windows probe calls it
pub fn friendly_app_name(process_name: &str) -> String {
    let stem = process_name
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(process_name)
        .trim_end_matches(".exe")
        .trim_end_matches(".EXE");
    match stem.to_lowercase().as_str() {
        "chrome" => "Chrome".into(),
        "msedge" => "Edge".into(),
        "firefox" => "Firefox".into(),
        "olk" | "outlook" => "Outlook".into(),
        "code" => "VS Code".into(),
        "devenv" => "Visual Studio".into(),
        "explorer" => "File Explorer".into(),
        "winword" => "Word".into(),
        "excel" => "Excel".into(),
        "powerpnt" => "PowerPoint".into(),
        "teams" | "ms-teams" => "Teams".into(),
        "slack" => "Slack".into(),
        "notion" => "Notion".into(),
        "figma" => "Figma".into(),
        "spotify" => "Spotify".into(),
        "steam" => "Steam".into(),
        "discord" => "Discord".into(),
        _ => {
            let mut chars = stem.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => "Unknown".into(),
            }
        }
    }
}

/// Browser processes whose sessions get extension metadata merged in.
pub fn is_browser_process(process_name: &str) -> bool {
    let p = process_name.to_lowercase();
    p.contains("chrome") || p.contains("msedge") || p.contains("edge") || p.contains("brave")
}
