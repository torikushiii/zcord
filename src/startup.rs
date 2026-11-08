#[cfg(windows)]
mod platform {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::{
            Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR},
            System::Registry::{
                HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RRF_RT_REG_SZ, RegCloseKey,
                RegDeleteValueW, RegGetValueW, RegOpenKeyExW, RegSetValueExW,
            },
        },
        core::PCWSTR,
    };

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE_NAME: &str = "zcord";

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }

    fn quoted_exe() -> Result<Vec<u16>, String> {
        let path = std::env::current_exe()
            .map_err(|error| format!("Could not find the executable: {error}"))?;
        let mut value = vec![u16::from(b'"')];
        value.extend(path.as_os_str().encode_wide());
        value.push(u16::from(b'"'));
        value.push(0);
        Ok(value)
    }

    fn startup_command() -> Result<Vec<u16>, String> {
        let mut value = quoted_exe()?;
        value.pop();
        value.extend(" --minimized".encode_utf16());
        value.push(0);
        Ok(value)
    }

    fn registry_error(status: WIN32_ERROR, action: &str) -> String {
        format!("{action} failed (Windows error {})", status.0)
    }

    pub fn enabled() -> bool {
        let Ok(expected) = quoted_exe() else {
            return false;
        };
        let Ok(minimized) = startup_command() else {
            return false;
        };
        let subkey = wide(RUN_KEY);
        let value = wide(VALUE_NAME);
        let mut size = 0u32;
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                PCWSTR(value.as_ptr()),
                RRF_RT_REG_SZ,
                None,
                None,
                Some(&mut size),
            )
        };
        if status != ERROR_SUCCESS || size < 2 {
            return false;
        }
        let mut data = vec![0u16; (size as usize).div_ceil(2)];
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                PCWSTR(value.as_ptr()),
                RRF_RT_REG_SZ,
                None,
                Some(data.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if status != ERROR_SUCCESS {
            return false;
        }
        let length = data
            .iter()
            .position(|character| *character == 0)
            .unwrap_or(data.len());
        let stored = String::from_utf16_lossy(&data[..length]).to_lowercase();
        let matches = |command: &[u16]| {
            String::from_utf16_lossy(&command[..command.len() - 1]).to_lowercase() == stored
        };
        matches(&expected) || matches(&minimized)
    }

    pub fn set_enabled(on: bool) -> Result<(), String> {
        let data = on.then(startup_command).transpose()?;
        let subkey = wide(RUN_KEY);
        let value = wide(VALUE_NAME);
        let mut key = HKEY::default();
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                None,
                KEY_SET_VALUE,
                &mut key,
            )
        };
        if status != ERROR_SUCCESS {
            if !on && status == ERROR_FILE_NOT_FOUND {
                return Ok(());
            }
            return Err(registry_error(status, "Opening the startup registry key"));
        }
        let result = match data {
            Some(data) => {
                let bytes = unsafe {
                    std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), data.len() * 2)
                };
                let status = unsafe {
                    RegSetValueExW(key, PCWSTR(value.as_ptr()), None, REG_SZ, Some(bytes))
                };
                if status == ERROR_SUCCESS {
                    Ok(())
                } else {
                    Err(registry_error(status, "Writing the startup registry value"))
                }
            }
            None => {
                let status = unsafe { RegDeleteValueW(key, PCWSTR(value.as_ptr())) };
                if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
                    Ok(())
                } else {
                    Err(registry_error(
                        status,
                        "Removing the startup registry value",
                    ))
                }
            }
        };
        unsafe {
            let _ = RegCloseKey(key);
        }
        result
    }
}

#[cfg(windows)]
pub fn enabled() -> bool {
    platform::enabled()
}

#[cfg(windows)]
pub fn set_enabled(on: bool) -> Result<(), String> {
    platform::set_enabled(on)
}

#[cfg(not(windows))]
pub fn enabled() -> bool {
    false
}

#[cfg(not(windows))]
pub fn set_enabled(_on: bool) -> Result<(), String> {
    Ok(())
}
