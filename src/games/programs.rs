use super::registry;
use std::collections::HashSet;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub name: String,
    pub exe: String,
}

pub fn running() -> Vec<Program> {
    let mut system = System::new();
    let refresh = ProcessRefreshKind::new()
        .with_exe(UpdateKind::OnlyIfNotSet)
        .with_user(UpdateKind::OnlyIfNotSet);
    system.refresh_processes_specifics(ProcessesToUpdate::All, refresh);
    let own_pid = sysinfo::get_current_pid().ok();
    let own_user = own_pid
        .and_then(|pid| system.process(pid))
        .and_then(|process| process.user_id())
        .cloned();
    let registered: Vec<String> = registry::games().into_iter().map(|game| game.exe).collect();
    let mut seen = HashSet::new();
    let mut programs = Vec::new();
    for (pid, title) in titled_processes(&system) {
        if Some(pid) == own_pid {
            continue;
        }
        let Some(process) = system.process(pid) else {
            continue;
        };
        if own_user.is_some() && process.user_id() != own_user.as_ref() {
            continue;
        }
        let Some(exe) = process.exe() else {
            continue;
        };
        let exe = registry::normalize_exe(&exe.to_string_lossy());
        if registered
            .iter()
            .any(|known| registry::same_exe(known, &exe))
            || !seen.insert(exe.to_lowercase())
        {
            continue;
        }
        programs.push(Program { name: title, exe });
    }
    programs
}

#[cfg(windows)]
fn titled_processes(_: &System) -> Vec<(Pid, String)> {
    use windows::Win32::{
        Foundation::{HWND, LPARAM},
        UI::WindowsAndMessaging::{
            EnumWindows, GW_OWNER, GWL_EXSTYLE, GetWindow, GetWindowLongW, GetWindowTextW,
            GetWindowThreadProcessId, IsWindowVisible, WS_EX_TOOLWINDOW,
        },
    };
    use windows_core::BOOL;

    unsafe extern "system" fn collect(hwnd: HWND, list: LPARAM) -> BOOL {
        // SAFETY: `list` is the `Vec` passed to `EnumWindows` below, alive for the whole call.
        let list = unsafe { &mut *(list.0 as *mut Vec<(Pid, String)>) };
        // SAFETY: plain queries on a window handle EnumWindows just gave us.
        unsafe {
            let tool = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0;
            if !IsWindowVisible(hwnd).as_bool() || GetWindow(hwnd, GW_OWNER).is_ok() || tool {
                return true.into();
            }
            let mut text = [0u16; 512];
            let length = GetWindowTextW(hwnd, &mut text);
            let title = String::from_utf16_lossy(&text[..length.max(0) as usize]);
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if !title.trim().is_empty() && pid != 0 {
                list.push((Pid::from_u32(pid), title.trim().to_owned()));
            }
        }
        true.into()
    }

    let mut list: Vec<(Pid, String)> = Vec::new();
    // SAFETY: the callback only runs during this call, while `list` is borrowed mutably here.
    let _ = unsafe { EnumWindows(Some(collect), LPARAM(&mut list as *mut _ as isize)) };
    list
}

#[cfg(not(windows))]
fn titled_processes(system: &System) -> Vec<(Pid, String)> {
    let mut list: Vec<(Pid, String)> = system
        .processes()
        .iter()
        .filter(|(_, process)| process.thread_kind().is_none())
        .filter_map(|(pid, process)| {
            let name = process.exe()?.file_name()?.to_string_lossy().into_owned();
            Some((*pid, name))
        })
        .collect();
    list.sort_by_key(|(_, name)| name.to_lowercase());
    list
}
