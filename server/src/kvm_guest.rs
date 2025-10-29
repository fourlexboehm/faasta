use std::sync::Once;

#[link(name = "kvmserverguest", kind = "static")]
unsafe extern "C" {
    fn kvmserverguest_remote_resume(buffer: *mut u8, len: isize) -> isize;
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
