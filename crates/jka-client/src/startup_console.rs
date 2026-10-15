//! Show the native console before creating the game window so its taskbar
//! button comes first. Never change a parent terminal. Terminal painting still
//! waits for the first game frame; latest.log is written during startup.
use std::sync::atomic::{AtomicBool, Ordering};

static STARTUP_PENDING: AtomicBool = AtomicBool::new(false);

#[cfg(windows)]
mod win {
    use std::ffi::c_void;

    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, WM_SETICON, ICON_BIG, ICON_SMALL, SMTO_ABORTIFHUNG,
    };

    type Hwnd = *mut c_void;
    #[link(name = "Kernel32")]
    unsafe extern "system" {
        pub(super) fn GetConsoleWindow() -> Hwnd;
        pub(super) fn GetConsoleProcessList(list: *mut u32, count: u32) -> u32;
        pub(super) fn GetCurrentProcessId() -> u32;
        fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
    }
    #[link(name = "Shell32")]
    unsafe extern "system" {
        fn ExtractIconExW(file: *const u16, index: i32, large: *mut Hwnd, small: *mut Hwnd, count: u32) -> u32;
    }
    #[link(name = "User32")]
    unsafe extern "system" {
        pub(super) fn ShowWindow(hwnd: Hwnd, command: i32) -> i32;
        pub(super) fn SetWindowPos(
            hwnd: Hwnd, insert_after: Hwnd,
            x: i32, y: i32, width: i32, height: i32, flags: u32,
        ) -> i32;
    }

    pub(super) fn set_console_icon(hwnd: Hwnd) {
        // Use Windows' own Command Prompt icon independently of the game icon.
        // Keep these two handles alive until process exit: conhost displays them.
        unsafe {
            let mut directory = [0u16; 260];
            let len = GetSystemDirectoryW(directory.as_mut_ptr(), directory.len() as u32) as usize;
            if len == 0 || len >= directory.len() {
                return;
            }
            let mut path = directory[..len].to_vec();
            path.extend("\\cmd.exe\0".encode_utf16());
            let mut large = std::ptr::null_mut();
            let mut small = std::ptr::null_mut();
            ExtractIconExW(path.as_ptr(), 0, &mut large, &mut small, 1);
            for (kind, icon) in [(ICON_BIG, large), (ICON_SMALL, small)] {
                if !icon.is_null() {
                    // The console HWND belongs to conhost. Bound the wait if
                    // that host is unresponsive instead of blocking startup.
                    SendMessageTimeoutW(
                        hwnd, WM_SETICON, kind as usize, icon as isize,
                        SMTO_ABORTIFHUNG, 100, std::ptr::null_mut(),
                    );
                }
            }
        }
    }
}

/// Show our existing console before logging, GPU work, or game-window creation.
/// Leave shared PowerShell/CMD terminals and explicit CLI runs alone.
pub(crate) fn prepare_own_console() {
    #[cfg(windows)] {
        if std::env::args_os().any(|arg| {
            let arg = arg.to_string_lossy();
            arg == "--console" || arg == "--validate-map"
        }) {
            return;
        }
        let mut ids = [0u32; 4];
        // An Explorer-launched console normally has only this process attached.
        // A shared parent console is left entirely alone.
        let count = unsafe { win::GetConsoleProcessList(ids.as_mut_ptr(), ids.len() as u32) };
        if count != 1 || ids[0] != unsafe { win::GetCurrentProcessId() } {
            return;
        }
        let hwnd = unsafe { win::GetConsoleWindow() };
        if hwnd.is_null() {
            return;
        }
        win::set_console_icon(hwnd);
        // Keep the console visible throughout startup. Hiding it and revealing
        // it at first present registers its taskbar button after the game's.
        unsafe { win::ShowWindow(hwnd, 4) }; // SW_SHOWNOACTIVATE
        STARTUP_PENDING.store(true, Ordering::Release);
    }
}

/// Put the startup console behind the presenting game without stealing focus.
pub(crate) fn reveal() {
    #[cfg(windows)] {
        if STARTUP_PENDING.swap(false, Ordering::AcqRel) {
            let hwnd = unsafe { win::GetConsoleWindow() };
            if !hwnd.is_null() {
                unsafe {
                    // Keep the restored companion behind the fullscreen game.
                    win::SetWindowPos(
                        hwnd, 1usize as *mut std::ffi::c_void, // HWND_BOTTOM
                        0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010,
                    ); // SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE
                }
            }
        }
    }
}

pub(crate) fn deferred() -> bool {
    STARTUP_PENDING.load(Ordering::Acquire)
}

/// Normal early startup errors also release queued console output.
pub(crate) struct RevealOnExit;
impl Drop for RevealOnExit {
    fn drop(&mut self) {
        reveal();
        crate::logging::activate_console_output();
    }
}
