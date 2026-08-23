//! The Win32 half, and nothing else.
//!
//! Everything here is unverifiable locally: there is no Rust on the dev box and
//! `check.sh` type-checks on Linux, so this file is proven by Windows CI alone.
//! That is the reason it answers three questions and holds no logic worth
//! testing: bucketing, naming, dedup and sealing all live in `mod.rs`, where a
//! container can reach them.

use std::path::Path;

use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, BOOL, HANDLE, HWND, LPARAM, MAX_PATH, RECT};
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::ProcessStatus::GetModuleBaseNameW;
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, OpenInputDesktop, DESKTOP_READOBJECTS,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetLastInputInfo, GetWindowTextW, GetWindowThreadProcessId, LASTINPUTINFO,
};

use super::{Probe, Shot, Source};

pub struct Screens;

impl Screens {
    pub fn new() -> Result<Self, String> {
        Ok(Self)
    }
}

impl Source for Screens {
    fn probe(&self) -> Result<Probe, String> {
        Ok(Probe {
            idle_secs: idle_seconds(),
            locked: is_locked(),
            process: foreground_process(),
            title: foreground_title(),
        })
    }

    fn shots(&self) -> Result<Vec<Shot>, String> {
        let monitors = xcap::Monitor::all().map_err(|e| e.to_string())?;
        let mut shots = Vec::with_capacity(monitors.len());

        // Left to right by the monitor's own x, so the stored image matches the
        // desk rather than whatever order the API happened to enumerate in.
        let mut ordered: Vec<_> = monitors.into_iter().collect();
        ordered.sort_by_key(|m| m.x().unwrap_or(0));

        for monitor in ordered {
            let image = monitor.capture_image().map_err(|e| e.to_string())?;
            let (x, y) = (monitor.x().unwrap_or(0), monitor.y().unwrap_or(0));
            shots.push(Shot {
                x,
                y,
                width: image.width(),
                height: image.height(),
                rgba: image.into_raw(),
            });
        }
        Ok(shots)
    }

    fn free_bytes(&self, path: &Path) -> u64 {
        let mut wide: Vec<u16> = path.to_string_lossy().encode_utf16().collect();
        wide.push(0);
        let mut free = 0u64;

        unsafe {
            match GetDiskFreeSpaceExW(
                windows::core::PCWSTR(wide.as_ptr()),
                Some(&mut free),
                None,
                None,
            ) {
                Ok(()) => free,
                // Refusing to capture is the safe answer to "how much room is
                // there"; claiming the disk is empty is not.
                Err(_) => 0,
            }
        }
    }
}

/// Milliseconds since the last keyboard or mouse input, machine-wide.
///
/// A meeting is idle input and real work, which is why this is RECORDED rather
/// than used to skip the frame: the hour hardest to reconstruct is the one
/// nobody typed during.
fn idle_seconds() -> u64 {
    let mut info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };

    unsafe {
        if GetLastInputInfo(&mut info).as_bool() {
            let now = windows::Win32::System::SystemInformation::GetTickCount();
            return now.saturating_sub(info.dwTime) as u64 / 1000;
        }
    }
    0
}

/// A locked workstation has no input desktop this process may read, so the open
/// failing IS the answer. It also keeps the lock screen itself out of the
/// archive, which is a picture of nothing worth keeping.
fn is_locked() -> bool {
    unsafe {
        match OpenInputDesktop(Default::default(), false, DESKTOP_READOBJECTS) {
            Ok(desktop) => {
                let _ = CloseDesktop(desktop);
                false
            }
            Err(_) => true,
        }
    }
}

fn foreground_title() -> String {
    let mut buffer = [0u16; 512];
    unsafe {
        let window = GetForegroundWindow();
        let len = GetWindowTextW(window, &mut buffer);
        if len <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buffer[..len as usize])
    }
}

/// The executable behind the foreground window.
///
/// "Inbox - …" says much less on its own than OUTLOOK.EXE beside it, and a
/// browser tab title is close to unreadable without knowing it is a browser.
fn foreground_process() -> String {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid));
        if pid == 0 {
            return String::new();
        }

        let Ok(handle) = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid)
        else {
            // Anything running as admin refuses to be opened by a normal
            // process. That is a name we do not get, not an error worth a line.
            return String::new();
        };

        let mut buffer = [0u16; MAX_PATH as usize];
        let len = GetModuleBaseNameW(handle, None, &mut buffer);
        let _ = CloseHandle(HANDLE(handle.0));

        if len == 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buffer[..len as usize])
    }
}
