use crate::voice::stream::StreamFrame;
use windows::{
    Win32::{
        Foundation::{CloseHandle, HWND, LPARAM, WPARAM},
        Graphics::Gdi::{
            BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC,
            GetDIBits, GetObjectW, HBITMAP, ReleaseDC,
        },
        System::Threading::{
            OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
            QueryFullProcessImageNameW,
        },
        UI::{
            Shell::ExtractIconExW,
            WindowsAndMessaging::{
                DestroyIcon, GCLP_HICON, GCLP_HICONSM, GetClassLongPtrW, GetIconInfo,
                GetWindowThreadProcessId, HICON, ICON_BIG, ICON_SMALL2, ICONINFO, SMTO_ABORTIFHUNG,
                SendMessageTimeoutW, WM_GETICON,
            },
        },
    },
    core::{PCWSTR, PWSTR},
};

const MAX_ICON: i32 = 256;
const MESSAGE_TIMEOUT_MS: u32 = 50;

pub(crate) fn window_icon(id: u64) -> Option<StreamFrame> {
    let hwnd = HWND(id as usize as *mut _);
    // The window owns the icons it hands out; only one extracted from the exe is ours to free.
    if let Some(icon) = message_icon(hwnd).or_else(|| class_icon(hwnd)) {
        return pixels(icon);
    }
    let icon = program_icon(hwnd)?;
    let frame = pixels(icon);
    unsafe {
        let _ = DestroyIcon(icon);
    }
    frame
}

fn message_icon(hwnd: HWND) -> Option<HICON> {
    [ICON_BIG, ICON_SMALL2].into_iter().find_map(|kind| {
        let mut result = 0usize;
        let answered = unsafe {
            SendMessageTimeoutW(
                hwnd,
                WM_GETICON,
                WPARAM(kind as usize),
                LPARAM(0),
                SMTO_ABORTIFHUNG,
                MESSAGE_TIMEOUT_MS,
                Some(&raw mut result),
            )
        };
        (answered.0 != 0 && result != 0).then_some(HICON(result as *mut _))
    })
}

fn class_icon(hwnd: HWND) -> Option<HICON> {
    [GCLP_HICON, GCLP_HICONSM].into_iter().find_map(|index| {
        let icon = unsafe { GetClassLongPtrW(hwnd, index) };
        (icon != 0).then_some(HICON(icon as *mut _))
    })
}

fn program_icon(hwnd: HWND) -> Option<HICON> {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut pid)) };
    if pid == 0 {
        return None;
    }
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut path = [0u16; 1024];
    let mut length = path.len() as u32 - 1;
    let named = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(path.as_mut_ptr()),
            &mut length,
        )
    };
    unsafe {
        let _ = CloseHandle(process);
    }
    named.ok()?;
    let mut icon = HICON::default();
    let count = unsafe { ExtractIconExW(PCWSTR(path.as_ptr()), 0, Some(&raw mut icon), None, 1) };
    (count > 0 && !icon.0.is_null()).then_some(icon)
}

fn pixels(icon: HICON) -> Option<StreamFrame> {
    let mut info = ICONINFO::default();
    unsafe { GetIconInfo(icon, &mut info) }.ok()?;
    let frame = read(&info);
    unsafe {
        for bitmap in [info.hbmColor, info.hbmMask] {
            if !bitmap.0.is_null() {
                let _ = DeleteObject(bitmap.into());
            }
        }
    }
    frame
}

fn read(info: &ICONINFO) -> Option<StreamFrame> {
    if info.hbmColor.0.is_null() {
        return None;
    }
    let mut bitmap = BITMAP::default();
    let read = unsafe {
        GetObjectW(
            info.hbmColor.into(),
            size_of::<BITMAP>() as i32,
            Some((&raw mut bitmap).cast()),
        )
    };
    let (width, height) = (bitmap.bmWidth, bitmap.bmHeight);
    if read == 0 || !(1..=MAX_ICON).contains(&width) || !(1..=MAX_ICON).contains(&height) {
        return None;
    }
    let mut bgra = bits(info.hbmColor, width, height)?;
    if bgra.chunks_exact(4).all(|pixel| pixel[3] == 0) {
        let mask = bits(info.hbmMask, width, height)?;
        for (pixel, mask) in bgra.chunks_exact_mut(4).zip(mask.chunks_exact(4)) {
            pixel[3] = if mask[0] == 0 { 255 } else { 0 };
        }
    }
    Some(StreamFrame {
        width: width as u32,
        height: height as u32,
        bgra,
    })
}

fn bits(bitmap: HBITMAP, width: i32, height: i32) -> Option<Vec<u8>> {
    if bitmap.0.is_null() {
        return None;
    }
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut data = vec![0u8; width as usize * height as usize * 4];
    unsafe {
        let dc = GetDC(None);
        if dc.0.is_null() {
            return None;
        }
        let lines = GetDIBits(
            dc,
            bitmap,
            0,
            height as u32,
            Some(data.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        );
        ReleaseDC(None, dc);
        (lines == height).then_some(data)
    }
}
