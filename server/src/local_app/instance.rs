//! One Windows controller across all data directories, including offline recovery.
use crate::{Error, Result};
#[cfg(windows)]
pub struct ControllerInstance(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl ControllerInstance {
    pub fn acquire() -> Result<Self> {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS},
            System::Threading::CreateMutexW,
        };
        let name: Vec<u16> = "Local\\CursorSub2APIByok.Controller"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe {
                CloseHandle(handle);
            }
            return Err(Error::ControllerConflict {
                code: "TAKEOVER_ACTIVE",
                message: "Cursor Sub2API BYOK 已在运行，请先退出已有控制器。".into(),
            });
        }
        Ok(Self(handle))
    }
}
#[cfg(windows)]
impl Drop for ControllerInstance {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
#[cfg(not(windows))]
pub struct ControllerInstance;
#[cfg(not(windows))]
impl ControllerInstance {
    pub fn acquire() -> Result<Self> {
        Err(Error::Config("Windows x64 is required".into()))
    }
}
