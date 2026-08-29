//! Windows foreground/idle/lock probe (spec §8) using Win32 APIs:
//! GetForegroundWindow + GetWindowTextW for the active window,
//! QueryFullProcessImageNameW for the owning process,
//! GetLastInputInfo for idle time, OpenInputDesktop for lock state.

use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, OpenInputDesktop, DESKTOP_ACCESS_FLAGS, DF_ALLOWOTHERACCOUNTHOOK,
};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId,
};

use super::{friendly_app_name, ActivityProbe, ProbeReading, RawSample};

pub struct WindowsProbe;

impl WindowsProbe {
    pub fn new() -> Self {
        Self
    }
}

impl ActivityProbe for WindowsProbe {
    fn read(&mut self) -> ProbeReading {
        let locked = is_locked();
        let idle_seconds = idle_seconds();

        if locked {
            return ProbeReading::Sample(RawSample {
                app_name: "Locked".into(),
                process_name: String::new(),
                window_title: String::new(),
                idle_seconds,
                locked: true,
                browser_domain: None,
                browser_title: None,
            });
        }

        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.is_invalid() {
            // No foreground window (secure desktop, transition): treat as idle-ish.
            return ProbeReading::Sample(RawSample {
                app_name: "Desktop".into(),
                process_name: "explorer.exe".into(),
                window_title: String::new(),
                idle_seconds,
                locked: false,
                browser_domain: None,
                browser_title: None,
            });
        }

        let window_title = window_title(hwnd);
        let process_name = process_name(hwnd).unwrap_or_else(|| "unknown".into());
        ProbeReading::Sample(RawSample {
            app_name: friendly_app_name(&process_name),
            process_name,
            window_title,
            idle_seconds,
            locked: false,
            browser_domain: None,
            browser_title: None,
        })
    }
}

fn window_title(hwnd: HWND) -> String {
    let mut buf = [0u16; 512];
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if len <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..len as usize])
}

fn process_name(hwnd: HWND) -> Option<String> {
    let mut pid: u32 = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == 0 {
        return None;
    }
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let result = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
    };
    unsafe {
        let _ = CloseHandle(handle);
    }
    result.ok()?;
    let path = String::from_utf16_lossy(&buf[..len as usize]);
    Some(
        path.rsplit('\\')
            .next()
            .unwrap_or(&path)
            .to_lowercase(),
    )
}

fn idle_seconds() -> u64 {
    let mut info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    let ok = unsafe { GetLastInputInfo(&mut info) };
    if !ok.as_bool() {
        return 0;
    }
    let now = unsafe { GetTickCount() };
    (now.wrapping_sub(info.dwTime) / 1000) as u64
}

/// The input desktop is inaccessible while the workstation is locked.
fn is_locked() -> bool {
    let desktop = unsafe {
        OpenInputDesktop(
            DF_ALLOWOTHERACCOUNTHOOK,
            false,
            DESKTOP_ACCESS_FLAGS(0x0100), // DESKTOP_SWITCHDESKTOP
        )
    };
    match desktop {
        Ok(h) => {
            unsafe {
                let _ = CloseDesktop(h);
            }
            false
        }
        Err(_) => true,
    }
}
