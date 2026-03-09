use std::sync::Once;

#[link(name = "kvmserverguest", kind = "static")]
unsafe extern "C" {
    fn kvmserverguest_remote_resume(buffer: *mut u8, len: isize) -> isize;
    fn kvmserverguest_storage_wait_paused(bufferptr: *mut *mut u8, ret: isize) -> isize;
}

static INIT: Once = Once::new();

/// Ensure libkvmserverguest is linked into the final binary. The guest runtime hooks
/// epoll/kqueue when the symbol is resolved, so touching it once at startup is sufficient.
pub fn ensure_linked() {
    INIT.call_once(|| unsafe {
        let mut byte = 0u8;
        let _ = kvmserverguest_remote_resume(&mut byte as *mut u8, 0);
    });
}

pub fn remote_resume(buffer: &mut [u8], len: usize) -> Result<usize, isize> {
    let response_len = unsafe { kvmserverguest_remote_resume(buffer.as_mut_ptr(), len as isize) };
    if response_len < 0 {
        Err(response_len)
    } else {
        Ok(response_len as usize)
    }
}

pub struct Storage {
    _private: (),
}

impl Storage {
    pub fn wait_paused(&mut self, return_value: isize) -> Result<Option<&mut [u8]>, isize> {
        let mut buffer_ptr = std::ptr::null_mut();
        let len = unsafe { kvmserverguest_storage_wait_paused(&mut buffer_ptr, return_value) };
        if len < 0 {
            return Err(len);
        }
        if buffer_ptr.is_null() {
            return Ok(None);
        }
        let slice = unsafe { std::slice::from_raw_parts_mut(buffer_ptr, len as usize) };
        Ok(Some(slice))
    }
}

thread_local! {
    static STORAGE: std::cell::Cell<Option<()>> = const { std::cell::Cell::new(Some(())) };
}

pub fn storage() -> Option<Storage> {
    STORAGE
        .with(|cell| cell.take())
        .map(|_| Storage { _private: () })
}

impl Drop for Storage {
    fn drop(&mut self) {
        STORAGE.with(|cell| cell.set(Some(())));
    }
}
