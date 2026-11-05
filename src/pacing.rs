//! VSync drives visible animation; background updates are capped independently of repaint sources.
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
pub struct Pacing {
    pub throttled: bool,
    refresh_hz: f32,
    last_frame: Option<Instant>,
    watching: bool,
    stop: Arc<AtomicBool>,
    native_hz: Arc<AtomicU32>,
}
impl Default for Pacing {
    fn default() -> Self {
        Self {
            throttled: false,
            refresh_hz: 60.,
            last_frame: None,
            watching: false,
            stop: Arc::new(AtomicBool::new(false)),
            native_hz: Arc::new(AtomicU32::new(60)),
        }
    }
}
impl Drop for Pacing {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
impl Pacing {
    pub fn predicted_dt(&self) -> f32 {
        1. / if self.throttled { 30. } else { self.refresh_hz }
    }
    pub fn begin(&mut self, frame: &eframe::Frame, signal: Arc<AtomicBool>) {
        #[cfg(windows)]
        if !self.watching {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(handle) = frame.window_handle() {
                if let RawWindowHandle::Win32(handle) = handle.as_raw() {
                    let hwnd = handle.hwnd.get();
                    let stop = self.stop.clone();
                    let hz = self.native_hz.clone();
                    let signal = signal.clone();
                    // Independent polling continues while eframe suspends painting a hidden window.
                    std::thread::spawn(move || {
                        while !stop.load(Ordering::Relaxed) {
                            let (rate, covered) = native_state(hwnd);
                            hz.store(rate, Ordering::Relaxed);
                            signal.store(covered, Ordering::Relaxed);
                            std::thread::sleep(Duration::from_millis(100));
                        }
                    });
                    self.watching = true;
                }
            }
        }
        #[cfg(not(windows))]
        let _ = frame;
        self.throttled = signal.load(Ordering::Relaxed);
        self.refresh_hz = self.native_hz.load(Ordering::Relaxed) as f32;
        if self.throttled {
            if let Some(last) = self.last_frame {
                let remaining = Duration::from_secs_f64(1. / 30.).saturating_sub(last.elapsed());
                if !remaining.is_zero() {
                    std::thread::sleep(remaining);
                }
            }
        }
        self.last_frame = Some(Instant::now());
    }
}
#[cfg(windows)]
fn native_state(hwnd: isize) -> (u32, bool) {
    use windows_sys::Win32::{
        Foundation::RECT,
        Graphics::{Dwm::*, Gdi::*},
        UI::WindowsAndMessaging::*,
    };
    let hwnd = hwnd as *mut std::ffi::c_void;
    // All handles are borrowed from eframe; calls only query window/display state.
    unsafe {
        let mut info: MONITORINFOEXW = std::mem::zeroed();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        let mut mode: DEVMODEW = std::mem::zeroed();
        mode.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
        let hz = if GetMonitorInfoW(
            MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
            &mut info.monitorInfo,
        ) != 0
            && EnumDisplaySettingsW(info.szDevice.as_ptr(), ENUM_CURRENT_SETTINGS, &mut mode) != 0
            && mode.dmDisplayFrequency > 1
        {
            mode.dmDisplayFrequency
        } else {
            60
        };
        if IsIconic(hwnd) != 0 || IsWindowVisible(hwnd) == 0 {
            return (hz, true);
        }
        let mut own: RECT = std::mem::zeroed();
        if GetWindowRect(hwnd, &mut own) == 0 {
            return (hz, false);
        }
        let mut other = GetTopWindow(std::ptr::null_mut());
        while !other.is_null() && other != hwnd {
            let mut cloaked = 0u32;
            let _ =
                DwmGetWindowAttribute(other, DWMWA_CLOAKED as u32, &mut cloaked as *mut _ as _, 4);
            let style = GetWindowLongW(other, GWL_EXSTYLE) as u32;
            if IsWindowVisible(other) != 0
                && IsIconic(other) == 0
                && cloaked == 0
                && style & (WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) == 0
            {
                let mut rect: RECT = std::mem::zeroed();
                if GetWindowRect(other, &mut rect) != 0
                    && rect.left < own.right
                    && rect.right > own.left
                    && rect.top < own.bottom
                    && rect.bottom > own.top
                {
                    return (hz, true);
                }
            }
            other = GetWindow(other, GW_HWNDNEXT);
        }
        (hz, false)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn monitor_rates_and_background_limit() {
        let mut p = Pacing::default();
        for hz in [60., 144., 240., 360.] {
            p.refresh_hz = hz;
            assert!((p.predicted_dt() - 1. / hz).abs() < 0.000001);
        }
        p.throttled = true;
        assert_eq!(p.predicted_dt(), 1. / 30.);
    }
}
