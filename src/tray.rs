#![cfg(windows)]

use std::sync::{
    LazyLock, OnceLock,
    atomic::{AtomicBool, Ordering},
};

use futures::{StreamExt, channel::mpsc};
use gpui::{AnyWindowHandle, App, Global, Window};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
};
use windows::{
    Win32::{
        Foundation::{
            CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM,
        },
        System::Threading::CreateMutexW,
        UI::{
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::{
                AllowSetForegroundWindow, FindWindowW, GetWindowThreadProcessId, PostMessageW,
                RegisterWindowMessageW, SW_HIDE, SW_SHOW, SetForegroundWindow, ShowWindow,
                WM_NCDESTROY,
            },
        },
    },
    core::w,
};

static NEVER_SHOWN: AtomicBool = AtomicBool::new(false);
static SAVED_MAXIMIZED: AtomicBool = AtomicBool::new(false);
static SHOW_MESSAGE: LazyLock<u32> =
    LazyLock::new(|| unsafe { RegisterWindowMessageW(w!("zcord-show-main-window")) });
static EVENTS: OnceLock<mpsc::UnboundedSender<Message>> = OnceLock::new();

struct Tray {
    icon: Option<TrayIcon>,
    open: MenuId,
    quit: MenuId,
    main_window: Option<AnyWindowHandle>,
}

impl Global for Tray {}

enum Message {
    Open,
    Icon(TrayIconEvent),
    Menu(MenuEvent),
}

fn create_icon(open_id: &MenuId, quit_id: &MenuId) -> anyhow::Result<TrayIcon> {
    let icon = Icon::from_resource(1, None)?;
    let open = MenuItem::with_id(open_id.clone(), "Open zcord", true, None);
    let separator = PredefinedMenuItem::separator();
    let quit = MenuItem::with_id(quit_id.clone(), "Quit zcord", true, None);
    let menu = Menu::with_items(&[&open, &separator, &quit])?;
    Ok(TrayIconBuilder::new()
        .with_icon(icon)
        .with_tooltip("zcord")
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()?)
}

pub fn init(cx: &mut App) {
    let open_id = MenuId::new("open-zcord");
    let quit_id = MenuId::new("quit-zcord");
    let icon = match create_icon(&open_id, &quit_id) {
        Ok(icon) => Some(icon),
        Err(error) => {
            log::warn!("Could not create the tray icon: {error}");
            None
        }
    };

    let (tx, mut rx) = mpsc::unbounded();
    let _ = EVENTS.set(tx.clone());
    let icon_tx = tx.clone();
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        let _ = icon_tx.unbounded_send(Message::Icon(event));
    }));
    let menu_tx = tx.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let _ = menu_tx.unbounded_send(Message::Menu(event));
    }));

    cx.set_global(Tray {
        icon,
        open: open_id,
        quit: quit_id,
        main_window: None,
    });

    cx.spawn(async move |cx| {
        while let Some(message) = rx.next().await {
            cx.update(|cx| match message {
                Message::Open => show_main_window(cx),
                Message::Icon(TrayIconEvent::Click {
                    button: tray_icon::MouseButton::Left,
                    button_state: tray_icon::MouseButtonState::Up,
                    ..
                }) => show_main_window(cx),
                Message::Menu(event) => {
                    let (open, quit) = cx
                        .try_global::<Tray>()
                        .map(|tray| (event.id() == &tray.open, event.id() == &tray.quit))
                        .unwrap_or((false, false));
                    if open {
                        show_main_window(cx);
                    } else if quit {
                        crate::settings::flush(cx);
                        cx.quit();
                    }
                }
                _ => {}
            });
        }
    })
    .detach();
}

pub fn available(cx: &App) -> bool {
    cx.try_global::<Tray>()
        .is_some_and(|tray| tray.icon.is_some())
}

pub fn set_main_window(cx: &mut App, handle: AnyWindowHandle) {
    cx.global_mut::<Tray>().main_window = Some(handle);
    let _ = handle.update(cx, |_, window, _| {
        let Some(hwnd) = hwnd(window) else {
            log::warn!("Could not get the main window handle for second-launch activation");
            return;
        };
        if *SHOW_MESSAGE == 0
            || !unsafe { SetWindowSubclass(hwnd, Some(activation_proc), 1, 0) }.as_bool()
        {
            log::warn!("Could not install second-launch activation handler");
        }
    });
}

unsafe extern "system" fn activation_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    subclass_id: usize,
    _data: usize,
) -> LRESULT {
    if message == *SHOW_MESSAGE {
        if let Some(tx) = EVENTS.get() {
            let _ = tx.unbounded_send(Message::Open);
        }
        return LRESULT(0);
    }
    if message == WM_NCDESTROY {
        unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(activation_proc), subclass_id);
        }
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

pub fn start_hidden(maximized: bool) {
    NEVER_SHOWN.store(true, Ordering::Relaxed);
    SAVED_MAXIMIZED.store(maximized, Ordering::Relaxed);
}

fn hwnd(window: &Window) -> Option<HWND> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return None;
    };
    Some(HWND(handle.hwnd.get() as *mut _))
}

pub fn hide(window: &Window) {
    let Some(hwnd) = hwnd(window) else {
        return;
    };
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

pub fn show(window: &Window) {
    if NEVER_SHOWN.swap(false, Ordering::Relaxed) {
        // A window opened with `show: false` is revealed by `activate_window`, which
        // also applies the placement zui deferred until the first show.
        window.activate_window();
        if SAVED_MAXIMIZED.load(Ordering::Relaxed) {
            window.zoom_window();
        }
        return;
    }
    let Some(hwnd) = hwnd(window) else {
        return;
    };
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
    }
    window.activate_window();
    unsafe {
        let _ = SetForegroundWindow(hwnd);
    }
}

fn show_main_window(cx: &mut App) {
    let handle = cx.try_global::<Tray>().and_then(|tray| tray.main_window);
    if let Some(handle) = handle {
        let _ = handle.update(cx, |_, window, _| show(window));
    }
}

pub fn activate_existing_instance() -> bool {
    let mutex = match unsafe { CreateMutexW(None, false, w!("Local\\zcord-single-instance")) } {
        Ok(mutex) => mutex,
        Err(error) => {
            log::warn!("Could not create the single-instance mutex: {error}");
            return false;
        }
    };
    if unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
        // The first instance keeps the named mutex alive until process exit.
        return false;
    }
    unsafe {
        let _ = CloseHandle(mutex);
    }
    if std::env::args().any(|arg| arg == "--minimized") {
        return true;
    }
    if let Ok(hwnd) = unsafe { FindWindowW(w!("Zed::Window"), w!("zcord")) } {
        unsafe {
            let mut process_id = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut process_id));
            if process_id != 0 {
                let _ = AllowSetForegroundWindow(process_id);
            }
            if *SHOW_MESSAGE == 0 {
                log::warn!("Could not register second-launch activation message");
            } else if let Err(error) = PostMessageW(Some(hwnd), *SHOW_MESSAGE, WPARAM(0), LPARAM(0))
            {
                log::warn!("Could not activate the existing instance: {error}");
            }
        }
    }
    true
}
