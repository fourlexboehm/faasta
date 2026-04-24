#![allow(dead_code)]

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use bincode::{Decode, Encode};
use faasta_types::{
    FaastaRequest, FaastaResponse, Header,
    stabby::alloc::{string::String as StableString, vec::Vec as StableVec},
};

pub const DEFAULT_SLOT_COUNT: usize = 4;
pub const DEFAULT_SLOT_BYTES: usize = 16 * 1024 * 1024;
pub const READY_POLL_INTERVAL: Duration = Duration::from_millis(25);

const SHM_MAGIC: u32 = 0x4653_484d;
const SHM_VERSION: u32 = 1;
const CACHELINE: usize = 64;
const FAST_SPINS: usize = 256;

pub const STATE_EMPTY: u32 = 0;
pub const STATE_READY: u32 = 1;
pub const STATE_RUNNING: u32 = 2;
pub const STATE_DONE: u32 = 3;
pub const STATE_ERROR: u32 = 4;
pub const STATE_WRITING: u32 = 5;

#[derive(Clone, Debug, Encode, Decode)]
pub struct WorkerRequest {
    pub method: u8,
    pub uri: String,
    pub headers: Vec<WireHeader>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, Encode, Decode)]
pub struct WorkerResponse {
    pub status: u16,
    pub headers: Vec<WireHeader>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, Encode, Decode)]
pub struct WireHeader {
    pub name: String,
    pub value: String,
}

#[repr(C, align(64))]
pub struct SharedHeader {
    magic: u32,
    version: u32,
    slot_count: u32,
    slot_bytes: u32,
    slot_stride: u32,
    notify: AtomicU32,
}

#[repr(C, align(64))]
pub struct SlotHeader {
    pub state: AtomicU32,
    pub generation: AtomicU32,
    pub request_len: AtomicU32,
    pub response_len: AtomicU32,
    pub error_len: AtomicU32,
}

pub struct SharedRegion {
    ptr: NonNull<u8>,
    len: usize,
    slot_count: usize,
    slot_bytes: usize,
    slot_stride: usize,
    _file: File,
}

unsafe impl Send for SharedRegion {}
unsafe impl Sync for SharedRegion {}

impl WorkerRequest {
    pub fn into_faasta(self) -> FaastaRequest {
        let mut headers: StableVec<Header> = StableVec::new();
        for header in self.headers {
            headers.push(Header {
                name: StableString::from(header.name.as_str()),
                value: StableString::from(header.value.as_str()),
            });
        }

        FaastaRequest {
            method: self.method,
            uri: StableString::from(self.uri.as_str()),
            headers,
            body: self.body.into_iter().collect(),
        }
    }
}

impl From<FaastaResponse> for WorkerResponse {
    fn from(response: FaastaResponse) -> Self {
        let headers = response
            .headers
            .iter()
            .map(|header| WireHeader {
                name: header.name.as_str().to_string(),
                value: header.value.as_str().to_string(),
            })
            .collect();

        Self {
            status: response.status,
            headers,
            body: response.body.iter().copied().collect(),
        }
    }
}

impl SharedHeader {
    fn initialize(
        &mut self,
        slot_count: usize,
        slot_bytes: usize,
        slot_stride: usize,
    ) -> Result<()> {
        self.magic = SHM_MAGIC;
        self.version = SHM_VERSION;
        self.slot_count = slot_count
            .try_into()
            .context("slot count does not fit in u32")?;
        self.slot_bytes = slot_bytes
            .try_into()
            .context("slot size does not fit in u32")?;
        self.slot_stride = slot_stride
            .try_into()
            .context("slot stride does not fit in u32")?;
        self.notify.store(0, Ordering::Relaxed);
        Ok(())
    }

    fn validate(&self) -> Result<(usize, usize, usize)> {
        if self.magic != SHM_MAGIC {
            bail!("invalid shared memory magic");
        }
        if self.version != SHM_VERSION {
            bail!("unsupported shared memory version {}", self.version);
        }
        Ok((
            self.slot_count as usize,
            self.slot_bytes as usize,
            self.slot_stride as usize,
        ))
    }
}

impl SharedRegion {
    pub fn create(path: &Path, slot_count: usize, slot_bytes: usize) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let slot_stride = align_up(std::mem::size_of::<SlotHeader>() + slot_bytes, CACHELINE);
        let len =
            align_up(std::mem::size_of::<SharedHeader>(), CACHELINE) + slot_count * slot_stride;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .with_context(|| format!("failed to open shared memory file {}", path.display()))?;
        file.set_len(len as u64)
            .with_context(|| format!("failed to size shared memory file {}", path.display()))?;

        let mut region = Self::map(file, len, slot_count, slot_bytes, slot_stride)?;
        unsafe {
            region
                .header_mut()
                .initialize(slot_count, slot_bytes, slot_stride)?;
            for index in 0..slot_count {
                let slot = region.slot_header(index);
                slot.state.store(STATE_EMPTY, Ordering::Relaxed);
                slot.generation.store(0, Ordering::Relaxed);
                slot.request_len.store(0, Ordering::Relaxed);
                slot.response_len.store(0, Ordering::Relaxed);
                slot.error_len.store(0, Ordering::Relaxed);
            }
        }
        Ok(region)
    }

    pub fn open(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .with_context(|| format!("failed to open shared memory file {}", path.display()))?;
        let len = file
            .metadata()
            .with_context(|| format!("failed to stat shared memory file {}", path.display()))?
            .len() as usize;
        let ptr = mmap_file(&file, len)?;
        let (slot_count, slot_bytes, slot_stride) =
            unsafe { (&*(ptr.as_ptr() as *const SharedHeader)).validate()? };
        Ok(Self {
            ptr,
            len,
            slot_count,
            slot_bytes,
            slot_stride,
            _file: file,
        })
    }

    fn map(
        file: File,
        len: usize,
        slot_count: usize,
        slot_bytes: usize,
        slot_stride: usize,
    ) -> Result<Self> {
        let ptr = mmap_file(&file, len)?;
        Ok(Self {
            ptr,
            len,
            slot_count,
            slot_bytes,
            slot_stride,
            _file: file,
        })
    }

    pub fn slot_count(&self) -> usize {
        self.slot_count
    }

    pub fn slot_bytes(&self) -> usize {
        self.slot_bytes
    }

    pub unsafe fn header(&self) -> &SharedHeader {
        unsafe { &*(self.ptr.as_ptr() as *const SharedHeader) }
    }

    unsafe fn header_mut(&mut self) -> &mut SharedHeader {
        unsafe { &mut *(self.ptr.as_ptr() as *mut SharedHeader) }
    }

    pub fn notify_worker(&self) {
        let notify = unsafe { &self.header().notify };
        notify.fetch_add(1, Ordering::Release);
        wake_atomic(notify, 1);
    }

    pub fn wait_for_work(&self, seen: u32) {
        let notify = unsafe { &self.header().notify };
        spin_then_wait(notify, seen, Duration::from_millis(1));
    }

    pub fn notify_value(&self) -> u32 {
        unsafe { self.header().notify.load(Ordering::Acquire) }
    }

    pub fn slot_header(&self, index: usize) -> &SlotHeader {
        assert!(index < self.slot_count);
        let offset =
            align_up(std::mem::size_of::<SharedHeader>(), CACHELINE) + index * self.slot_stride;
        unsafe { &*(self.ptr.as_ptr().add(offset) as *const SlotHeader) }
    }

    pub fn slot_payload(&self, index: usize) -> &[u8] {
        assert!(index < self.slot_count);
        let offset = align_up(std::mem::size_of::<SharedHeader>(), CACHELINE)
            + index * self.slot_stride
            + std::mem::size_of::<SlotHeader>();
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr().add(offset), self.slot_bytes) }
    }

    fn slot_payload_ptr(&self, index: usize) -> *mut u8 {
        assert!(index < self.slot_count);
        let offset = align_up(std::mem::size_of::<SharedHeader>(), CACHELINE)
            + index * self.slot_stride
            + std::mem::size_of::<SlotHeader>();
        unsafe { self.ptr.as_ptr().add(offset) }
    }

    pub fn encode_request(&self, index: usize, request: &WorkerRequest) -> Result<()> {
        let bytes = bincode::encode_to_vec(request, bincode::config::standard())
            .context("failed to encode worker request")?;
        self.write_request_bytes(index, &bytes)
    }

    pub fn read_request(&self, index: usize) -> Result<WorkerRequest> {
        let slot = self.slot_header(index);
        let len = slot.request_len.load(Ordering::Acquire) as usize;
        if len > self.slot_bytes {
            bail!("request length exceeds slot size");
        }
        let payload = &self.slot_payload(index)[..len];
        let (request, _) = bincode::decode_from_slice(payload, bincode::config::standard())
            .context("failed to decode worker request")?;
        Ok(request)
    }

    pub fn write_response(&self, index: usize, response: &WorkerResponse) -> Result<()> {
        let bytes = bincode::encode_to_vec(response, bincode::config::standard())
            .context("failed to encode worker response")?;
        if bytes.len() > self.slot_bytes {
            bail!(
                "worker response is too large: {} > {} bytes",
                bytes.len(),
                self.slot_bytes
            );
        }
        self.write_payload(index, &bytes);
        let slot = self.slot_header(index);
        slot.response_len
            .store(bytes.len() as u32, Ordering::Release);
        slot.state.store(STATE_DONE, Ordering::Release);
        wake_atomic(&slot.state, 1);
        Ok(())
    }

    pub fn write_error(&self, index: usize, message: &str) {
        let bytes = message.as_bytes();
        let len = bytes.len().min(self.slot_bytes);
        self.write_payload(index, &bytes[..len]);
        let slot = self.slot_header(index);
        slot.error_len.store(len as u32, Ordering::Release);
        slot.state.store(STATE_ERROR, Ordering::Release);
        wake_atomic(&slot.state, 1);
    }

    pub fn read_response(&self, index: usize) -> Result<WorkerResponse> {
        let slot = self.slot_header(index);
        let len = slot.response_len.load(Ordering::Acquire) as usize;
        if len > self.slot_bytes {
            bail!("response length exceeds slot size");
        }
        let payload = &self.slot_payload(index)[..len];
        let (response, _) = bincode::decode_from_slice(payload, bincode::config::standard())
            .context("failed to decode worker response")?;
        Ok(response)
    }

    pub fn read_error(&self, index: usize) -> String {
        let slot = self.slot_header(index);
        let len = (slot.error_len.load(Ordering::Acquire) as usize).min(self.slot_bytes);
        String::from_utf8_lossy(&self.slot_payload(index)[..len]).into_owned()
    }

    fn write_request_bytes(&self, index: usize, bytes: &[u8]) -> Result<()> {
        if bytes.len() > self.slot_bytes {
            bail!(
                "worker request is too large: {} > {} bytes",
                bytes.len(),
                self.slot_bytes
            );
        }
        self.write_payload(index, bytes);
        let slot = self.slot_header(index);
        slot.request_len
            .store(bytes.len() as u32, Ordering::Release);
        slot.response_len.store(0, Ordering::Release);
        slot.error_len.store(0, Ordering::Release);
        Ok(())
    }

    fn write_payload(&self, index: usize, bytes: &[u8]) {
        assert!(bytes.len() <= self.slot_bytes);
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                self.slot_payload_ptr(index),
                bytes.len(),
            );
        }
    }
}

impl Drop for SharedRegion {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.ptr.as_ptr().cast(), self.len);
        }
    }
}

pub fn function_symbol_name(function_name: &str) -> String {
    let sanitized = function_name.replace('-', "_");
    format!("dy_{sanitized}")
}

pub fn wait_for_state_change(state: &AtomicU32, observed: u32, max_sleep: Duration) {
    spin_then_wait(state, observed, max_sleep);
}

pub fn wake_state(state: &AtomicU32) {
    wake_atomic(state, 1);
}

pub fn wait_until<F>(timeout: Duration, mut f: F) -> Result<()>
where
    F: FnMut() -> bool,
{
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if f() {
            return Ok(());
        }
        thread::sleep(READY_POLL_INTERVAL);
    }
    bail!("timed out")
}

fn spin_then_wait(state: &AtomicU32, observed: u32, max_sleep: Duration) {
    for _ in 0..FAST_SPINS {
        if state.load(Ordering::Acquire) != observed {
            return;
        }
        std::hint::spin_loop();
    }

    #[cfg(target_os = "linux")]
    {
        futex_wait(state, observed, max_sleep);
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = max_sleep;
        thread::yield_now();
    }
}

#[cfg(target_os = "linux")]
fn futex_wait(state: &AtomicU32, observed: u32, max_sleep: Duration) {
    let timeout = libc::timespec {
        tv_sec: max_sleep.as_secs() as libc::time_t,
        tv_nsec: max_sleep.subsec_nanos() as libc::c_long,
    };
    unsafe {
        libc::syscall(
            libc::SYS_futex,
            state as *const AtomicU32 as *const u32,
            libc::FUTEX_WAIT,
            observed,
            &timeout as *const libc::timespec,
        );
    }
}

#[cfg(target_os = "linux")]
fn wake_atomic(state: &AtomicU32, count: i32) {
    unsafe {
        libc::syscall(
            libc::SYS_futex,
            state as *const AtomicU32 as *const u32,
            libc::FUTEX_WAKE,
            count,
        );
    }
}

#[cfg(not(target_os = "linux"))]
fn wake_atomic(_state: &AtomicU32, _count: i32) {}

fn mmap_file(file: &File, len: usize) -> Result<NonNull<u8>> {
    if len == 0 {
        bail!("cannot mmap empty shared memory file");
    }
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            std::os::fd::AsRawFd::as_raw_fd(file),
            0,
        )
    };
    if ptr == libc::MAP_FAILED {
        return Err(io::Error::last_os_error()).context("mmap failed");
    }
    NonNull::new(ptr.cast()).context("mmap returned null")
}

const fn align_up(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}
