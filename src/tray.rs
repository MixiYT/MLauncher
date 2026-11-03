use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::UI::{
    Shell::{Shell_NotifyIconW, NIF_ICON, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW},
    WindowsAndMessaging::{
        LoadIconW, SendMessageW, ShowWindow, ShowWindowAsync, ICON_BIG, IDI_APPLICATION, SW_HIDE,
        SW_RESTORE, WM_GETICON,
    },
};

#[derive(Default)]
pub struct Tray {
    window: isize,
    added: bool,
}

impl Tray {
    pub fn bind(&mut self, frame: &eframe::Frame) {
        if let Ok(handle) = frame.window_handle() {
            if let RawWindowHandle::Win32(handle) = handle.as_raw() {
                self.window = handle.hwnd.get();
            }
        }
    }

    pub fn window(&self) -> isize {
        self.window
    }

    fn data(&self) -> NOTIFYICONDATAW {
        let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = self.window as _;
        data.uID = 1;
        data
    }

    pub fn hide(&mut self) -> bool {
        if self.window == 0 {
            return false;
        }
        let mut data = self.data();
        data.uFlags = NIF_ICON | NIF_TIP;
        for (slot, unit) in data.szTip.iter_mut().zip("MLauncher".encode_utf16()) {
            *slot = unit;
        }
        unsafe {
            data.hIcon = SendMessageW(data.hWnd, WM_GETICON, ICON_BIG as usize, 0) as _;
            if data.hIcon.is_null() {
                data.hIcon = LoadIconW(std::ptr::null_mut(), IDI_APPLICATION);
            }
            self.added = Shell_NotifyIconW(NIM_ADD, &data) != 0;
            if self.added {
                ShowWindow(data.hWnd, SW_HIDE);
            }
        }
        self.added
    }

    pub fn remove(&mut self) {
        if self.added {
            unsafe {
                Shell_NotifyIconW(NIM_DELETE, &self.data());
            }
            self.added = false;
            restore(self.window);
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.remove();
    }
}

pub fn restore(window: isize) {
    if window != 0 {
        unsafe {
            ShowWindowAsync(window as _, SW_RESTORE);
        }
    }
}
