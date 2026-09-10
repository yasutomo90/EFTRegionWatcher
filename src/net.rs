//! Small blocking WinHTTP client. Call only from worker threads.
use std::{io, ptr};
use windows_sys::Win32::Networking::WinHttp::*;
struct Internet(*mut std::ffi::c_void);
impl Drop for Internet {
    fn drop(&mut self) {
        unsafe {
            WinHttpCloseHandle(self.0);
        }
    }
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn handle(h: *mut std::ffi::c_void) -> io::Result<Internet> {
    if h.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(Internet(h))
    }
}
pub fn get(url: &str, limit: usize) -> io::Result<Vec<u8>> {
    let (host, resource) = url
        .strip_prefix("https://")
        .and_then(|s| s.split_once('/'))
        .ok_or_else(|| io::Error::other("HTTPS URL が必要です"))?;
    if host.is_empty()
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        || resource.contains(['\r', '\n', '\0'])
    {
        return Err(io::Error::other("不正な URL"));
    }
    unsafe {
        let agent = wide(concat!("EFTRegionWatcher/", env!("CARGO_PKG_VERSION")));
        let session = handle(WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            ptr::null(),
            ptr::null(),
            0,
        ))?;
        if WinHttpSetTimeouts(session.0, 5000, 5000, 5000, 10000) == 0 {
            return Err(io::Error::last_os_error());
        }
        let host = wide(host);
        let connection = handle(WinHttpConnect(session.0, host.as_ptr(), 443, 0))?;
        let verb = wide("GET");
        let resource = wide(&format!("/{resource}"));
        let request = handle(WinHttpOpenRequest(
            connection.0,
            verb.as_ptr(),
            resource.as_ptr(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            WINHTTP_FLAG_SECURE,
        ))?;
        // Keep Windows' default certificate validation; prohibit TLS downgrade redirects.
        let policy = WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP;
        if WinHttpSetOption(
            request.0,
            WINHTTP_OPTION_REDIRECT_POLICY,
            (&policy as *const u32).cast(),
            4,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let headers = wide("Accept: application/json\r\n");
        if WinHttpSendRequest(request.0, headers.as_ptr(), u32::MAX, ptr::null(), 0, 0, 0) == 0
            || WinHttpReceiveResponse(request.0, ptr::null_mut()) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let mut status = 0u32;
        let mut size = 4;
        if WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            ptr::null(),
            (&mut status as *mut u32).cast(),
            &mut size,
            ptr::null_mut(),
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        if status != 200 {
            return Err(io::Error::other(format!("HTTP {status}")));
        }
        let start = std::time::Instant::now();
        let mut result = Vec::new();
        let mut buf = [0u8; 32768];
        loop {
            if start.elapsed() > std::time::Duration::from_secs(120) {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "ダウンロードがタイムアウトしました",
                ));
            }
            let mut read = 0;
            if WinHttpReadData(
                request.0,
                buf.as_mut_ptr().cast(),
                buf.len() as u32,
                &mut read,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            if read == 0 {
                break;
            }
            if result.len() + read as usize > limit {
                return Err(io::Error::other("応答サイズが上限を超えました"));
            }
            result.extend_from_slice(&buf[..read as usize]);
        }
        Ok(result)
    }
}
