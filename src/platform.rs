use std::{io, path::Path, ptr};
use windows_sys::Win32::{
    Foundation::*,
    System::{Diagnostics::ToolHelp::*, Threading::*},
    UI::WindowsAndMessaging::*,
};
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
pub struct OwnedHandle(pub HANDLE);
// Kernel handles may be waited on concurrently; ownership closes the handle exactly once.
unsafe impl Send for OwnedHandle {}
unsafe impl Sync for OwnedHandle {}
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
pub fn process_handle(pid: u32) -> io::Result<OwnedHandle> {
    let h = unsafe {
        OpenProcess(
            PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            pid,
        )
    };
    if h.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(OwnedHandle(h))
    }
}
pub fn game_processes() -> io::Result<Vec<OwnedHandle>> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let _snapshot = OwnedHandle(snapshot);
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of_val(&entry) as u32;
    let mut result = Vec::new();
    let mut next = unsafe { Process32FirstW(snapshot, &mut entry) };
    while next != 0 {
        let end = entry
            .szExeFile
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..end]);
        if name.eq_ignore_ascii_case("EscapeFromTarkov.exe") {
            match process_handle(entry.th32ProcessID) {
                Ok(h) => result.push(h),
                Err(e) if e.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) => {}
                Err(e) => return Err(e),
            }
        }
        next = unsafe { Process32NextW(snapshot, &mut entry) };
    }
    Ok(result)
}
pub fn wait(handle: &OwnedHandle, timeout: u32) -> io::Result<bool> {
    match unsafe { WaitForSingleObject(handle.0, timeout) } {
        WAIT_OBJECT_0 => Ok(true),
        WAIT_TIMEOUT => Ok(false),
        _ => Err(io::Error::last_os_error()),
    }
}
pub fn executable_of(h: &OwnedHandle) -> io::Result<std::path::PathBuf> {
    let mut b = vec![0u16; 32768];
    let mut n = b.len() as u32;
    if unsafe { QueryFullProcessImageNameW(h.0, 0, b.as_mut_ptr(), &mut n) } == 0 {
        return Err(io::Error::last_os_error());
    }
    use std::os::windows::ffi::OsStringExt;
    Ok(std::ffi::OsString::from_wide(&b[..n as usize]).into())
}
pub fn error(message: &str) {
    unsafe {
        MessageBoxW(
            ptr::null_mut(),
            wide(message).as_ptr(),
            wide("EFTRegionWatcher").as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}
pub fn start(path: &Path) -> io::Result<std::process::Child> {
    std::process::Command::new(path)
        .current_dir(path.parent().unwrap_or(Path::new(".")))
        .spawn()
}
