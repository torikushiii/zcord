use std::{
    ffi::OsStr,
    fs,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
};

use tauri_winrt_notification::{IconCrop, Toast};
use windows::{
    Win32::{
        Foundation::PROPERTYKEY,
        System::{
            Com::{
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoUninitialize, IPersistFile, STGM_READWRITE,
                StructuredStorage::{PROPVARIANT, PropVariantClear},
            },
            Variant::VT_LPWSTR,
        },
        UI::Shell::{
            IShellLinkW, PropertiesSystem::IPropertyStore, SHStrDupW,
            SetCurrentProcessExplicitAppUserModelID, ShellLink,
        },
    },
    core::{GUID, Interface, PCWSTR},
};

const APP_ID: &str = "zcord";
const APP_USER_MODEL_ID: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
    pid: 5,
};

pub fn init() {
    let app_id = wide(OsStr::new(APP_ID));
    if let Err(error) = unsafe { SetCurrentProcessExplicitAppUserModelID(PCWSTR(app_id.as_ptr())) }
    {
        log::warn!("could not set notification app id: {error}");
    }
    let Some(shortcut) = shortcut_path() else {
        log::warn!("could not locate the Start Menu folder");
        return;
    };
    if let Err(error) = ensure_shortcut(&shortcut) {
        log::warn!("could not set up the notification shortcut: {error}");
    }
}

pub fn show(title: &str, body: &str, icon: Option<&Path>, channel_id: u64, message_id: u64) {
    let sender = crate::notify::activation_sender().clone();
    let mut toast = Toast::new(APP_ID)
        .title(title)
        .text1(body)
        .sound(None)
        .on_activated(move |_| {
            if let Err(error) = sender.send((channel_id, message_id)) {
                log::warn!("could not deliver notification activation: {error}");
            }
            Ok(())
        });
    if let Some(icon) = icon {
        toast = toast.icon(icon, IconCrop::Circular, "");
    }
    if let Err(error) = toast.show() {
        log::warn!("could not show notification: {error}");
    }
}

fn shortcut_path() -> Option<PathBuf> {
    Some(
        PathBuf::from(std::env::var_os("APPDATA")?)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("zcord.lnk"),
    )
}

/// Windows drops a desktop app's toasts unless a Start Menu shortcut carries its app id.
/// Creates the shortcut, or adds the id to one made without it, such as by an installer, while
/// keeping what that shortcut opens.
fn ensure_shortcut(shortcut: &Path) -> Result<(), String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let parent = shortcut
        .parent()
        .ok_or_else(|| "shortcut has no parent folder".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let shortcut = shortcut.to_path_buf();
    std::thread::Builder::new()
        .name("zcord-toast-identity".into())
        .spawn(move || ensure_shortcut_on_sta(&executable, &shortcut))
        .map_err(|error| error.to_string())?
        .join()
        .map_err(|_| "notification shortcut thread panicked".to_string())?
}

fn ensure_shortcut_on_sta(executable: &Path, shortcut_path: &Path) -> Result<(), String> {
    let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if result.is_err() {
        return Err(format!("CoInitializeEx failed: {result:?}"));
    }
    let _guard = ComGuard;

    let existing = shortcut_path.is_file();
    let executable_wide = wide(executable.as_os_str());
    let shortcut = wide(shortcut_path.as_os_str());
    let app_id = wide(OsStr::new(APP_ID));
    let description = wide(OsStr::new("zcord"));
    let working_directory = executable
        .parent()
        .ok_or_else(|| "executable has no parent folder".to_string())?;
    let working_directory = wide(working_directory.as_os_str());

    let shell_link: IShellLinkW =
        unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
            .map_err(|error| error.to_string())?;
    let file: IPersistFile = shell_link.cast().map_err(|error| error.to_string())?;
    let properties: IPropertyStore = shell_link.cast().map_err(|error| error.to_string())?;
    if existing {
        unsafe { file.Load(PCWSTR(shortcut.as_ptr()), STGM_READWRITE) }
            .map_err(|error| error.to_string())?;
        if stored_app_id(&properties).as_deref() == Some(APP_ID) {
            return Ok(());
        }
    } else {
        set_target(
            &shell_link,
            &executable_wide,
            &working_directory,
            &description,
        )?;
    }
    let mut app_id_value = prop_variant_from_string(&app_id)?;
    let set_result = unsafe { properties.SetValue(&APP_USER_MODEL_ID, &app_id_value) };
    unsafe {
        let _ = PropVariantClear(&mut app_id_value);
    }
    set_result.map_err(|error| error.to_string())?;
    unsafe { properties.Commit() }.map_err(|error| error.to_string())?;
    unsafe { file.Save(PCWSTR(shortcut.as_ptr()), true) }.map_err(|error| error.to_string())
}

fn set_target(
    shell_link: &IShellLinkW,
    executable_wide: &[u16],
    working_directory: &[u16],
    description: &[u16],
) -> Result<(), String> {
    unsafe {
        shell_link
            .SetPath(PCWSTR(executable_wide.as_ptr()))
            .map_err(|error| error.to_string())?;
        shell_link
            .SetWorkingDirectory(PCWSTR(working_directory.as_ptr()))
            .map_err(|error| error.to_string())?;
        shell_link
            .SetDescription(PCWSTR(description.as_ptr()))
            .map_err(|error| error.to_string())?;
        shell_link
            .SetIconLocation(PCWSTR(executable_wide.as_ptr()), 0)
            .map_err(|error| error.to_string())
    }
}

fn stored_app_id(properties: &IPropertyStore) -> Option<String> {
    let mut value = unsafe { properties.GetValue(&APP_USER_MODEL_ID) }.ok()?;
    let id = unsafe {
        let inner = &*value.Anonymous.Anonymous;
        (inner.vt == VT_LPWSTR && !inner.Anonymous.pwszVal.is_null())
            .then(|| inner.Anonymous.pwszVal.to_string().ok())
            .flatten()
    };
    unsafe {
        let _ = PropVariantClear(&mut value);
    }
    id
}

fn prop_variant_from_string(value: &[u16]) -> Result<PROPVARIANT, String> {
    let mut variant = PROPVARIANT::default();
    let duplicated =
        unsafe { SHStrDupW(PCWSTR(value.as_ptr())) }.map_err(|error| error.to_string())?;
    unsafe {
        let inner = &mut *variant.Anonymous.Anonymous;
        inner.vt = VT_LPWSTR;
        inner.Anonymous.pwszVal = duplicated;
    }
    Ok(variant)
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

struct ComGuard;

impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}
