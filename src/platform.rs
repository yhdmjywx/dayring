//! Window-system helpers that are not drawing and not the schedule.

use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::{
            Console::*,
            DataExchange::*,
            Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
        },
        UI::{HiDpi::*, Input::Ime::*, WindowsAndMessaging::*},
    },
    core::*,
};

pub fn attach_console() {
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

pub fn error_box(message: &str) {
    let text = HSTRING::from(message);
    let title = HSTRING::from("一天的形状");
    unsafe {
        let _ = MessageBoxW(None, &text, &title, MB_OK | MB_ICONERROR);
    }
}

pub struct Monitor {
    pub work: RECT,
    pub dpi: f32,
}

pub fn cursor_monitor() -> Monitor {
    unsafe {
        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        monitor_info(monitor)
    }
}

fn monitor_info(monitor: HMONITOR) -> Monitor {
    unsafe {
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(monitor, &mut info);
        let mut dpi_x = 96u32;
        let mut dpi_y = 96u32;
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        Monitor {
            work: info.rcWork,
            dpi: dpi_x as f32 / 96.0,
        }
    }
}

pub fn window_dpi(hwnd: HWND) -> f32 {
    unsafe { GetDpiForWindow(hwnd) as f32 / 96.0 }
}

/// Outer window size for a client area, in physical pixels.
pub fn outer_size(client_w: i32, client_h: i32, dpi: u32) -> (i32, i32) {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: client_w,
        bottom: client_h,
    };
    unsafe {
        let _ = AdjustWindowRectExForDpi(&mut rect, WS_OVERLAPPEDWINDOW, false, WINDOW_EX_STYLE(0), dpi);
    }
    (rect.right - rect.left, rect.bottom - rect.top)
}

pub fn ime_follow(hwnd: HWND, x: i32, y: i32) {
    unsafe {
        let context = ImmGetContext(hwnd);
        if context.is_invalid() {
            return;
        }
        let form = COMPOSITIONFORM {
            dwStyle: CFS_POINT,
            ptCurrentPos: POINT { x, y },
            ..Default::default()
        };
        let _ = ImmSetCompositionWindow(context, &form);
        let _ = ImmReleaseContext(hwnd, context);
    }
}

const CF_UNICODETEXT: u32 = 13;

pub fn clipboard_get() -> Option<String> {
    unsafe {
        if OpenClipboard(None).is_err() {
            return None;
        }
        let handle = match GetClipboardData(CF_UNICODETEXT) {
            Ok(handle) => handle,
            Err(_) => {
                let _ = CloseClipboard();
                return None;
            }
        };
        let pointer = GlobalLock(HGLOBAL(handle.0));
        if pointer.is_null() {
            let _ = CloseClipboard();
            return None;
        }
        let mut len = 0usize;
        while *pointer.cast::<u16>().add(len) != 0 {
            len += 1;
            if len > 1_000_000 {
                break;
            }
        }
        let slice = std::slice::from_raw_parts(pointer.cast::<u16>(), len);
        let text = String::from_utf16_lossy(slice);
        let _ = GlobalUnlock(HGLOBAL(handle.0));
        let _ = CloseClipboard();
        Some(text)
    }
}

pub fn clipboard_set(text: &str) -> bool {
    unsafe {
        if OpenClipboard(None).is_err() {
            return false;
        }
        let _ = EmptyClipboard();
        let units: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        let bytes = units.len() * 2;
        let memory = match GlobalAlloc(GMEM_MOVEABLE, bytes) {
            Ok(memory) => memory,
            Err(_) => {
                let _ = CloseClipboard();
                return false;
            }
        };
        let pointer = GlobalLock(memory);
        if pointer.is_null() {
            let _ = CloseClipboard();
            return false;
        }
        std::ptr::copy_nonoverlapping(units.as_ptr(), pointer.cast(), units.len());
        let _ = GlobalUnlock(memory);
        let ok = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(memory.0))).is_ok();
        let _ = CloseClipboard();
        ok
    }
}
