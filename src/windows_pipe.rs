//! Duplex named-pipe IPC uses overlapped handles. A synchronous read on a cloned
//! Windows File can block writes to the same file object; do not replace this
//! with OpenOptions + a blocking reader thread.
use std::{
    io::{self, Read, Write},
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
    ptr,
    sync::Arc,
};
use windows_sys::Win32::{
    Foundation::{ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{CreateFileW, ReadFile, WriteFile, FILE_FLAG_OVERLAPPED, OPEN_EXISTING},
    System::{
        Threading::CreateEventW,
        IO::{GetOverlappedResult, OVERLAPPED},
    },
};

pub struct Pipe(Arc<OwnedHandle>);
impl Pipe {
    pub fn connect(path: &Path) -> io::Result<Self> {
        let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: NUL-terminated path; ownership transferred exactly once on success.
        let raw = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                ptr::null_mut(),
            )
        };
        if raw == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(Arc::new(unsafe { OwnedHandle::from_raw_handle(raw) })))
    }
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self(self.0.clone()))
    }
    fn transfer(&self, buffer: *mut u8, len: usize, read: bool) -> io::Result<usize> {
        // SAFETY: independent event/OVERLAPPED per operation, valid live buffer until
        // GetOverlappedResult completes; handle is shared only for overlapped IO.
        unsafe {
            let event = CreateEventW(ptr::null(), 1, 0, ptr::null());
            if event.is_null() {
                return Err(io::Error::last_os_error());
            }
            let _event = OwnedHandle::from_raw_handle(event);
            let mut ov: OVERLAPPED = std::mem::zeroed();
            ov.hEvent = event;
            let mut count = 0;
            let handle = self.0.as_raw_handle();
            let len = len.min(u32::MAX as usize) as u32;
            let ok = if read {
                ReadFile(handle, buffer, len, &mut count, &mut ov)
            } else {
                WriteFile(handle, buffer, len, &mut count, &mut ov)
            };
            if ok == 0 {
                let err = io::Error::last_os_error();
                if err.raw_os_error() != Some(ERROR_IO_PENDING as i32) {
                    return Err(err);
                }
                if GetOverlappedResult(handle, &ov, &mut count, 1) == 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(count as usize)
        }
    }
}
impl Read for Pipe {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.transfer(buf.as_mut_ptr(), buf.len(), true)
    }
}
impl Write for Pipe {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.transfer(buf.as_ptr() as *mut u8, buf.len(), false)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
