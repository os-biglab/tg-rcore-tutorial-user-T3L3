#![no_std]
//!
//! 教程阅读建议：
//!
//! - `_start` 展示用户程序最小运行时（初始化控制台、堆、调用 main）；
//! - 其余辅助函数（sleep/pipe_*）展示了常见 syscall 组合用法。

mod heap;
mod tangram;

extern crate alloc;

use alloc::alloc::alloc;
use core::alloc::Layout;
use core::sync::atomic::{AtomicUsize, Ordering};
use tg_console::log;

pub use tg_console::{print, println};
pub use tg_syscall::*;

const SYSCALL_FRAMEBUFFER: usize = 0x1000_0001;
const SYSCALL_FRAMEBUFFER_FLUSH: usize = 0x1000_0002;
const TG_ALLOC_ARENA_SIZE: usize = 16 << 20;

#[repr(align(16))]
struct TgAllocArena([u8; TG_ALLOC_ARENA_SIZE]);

static mut TG_ALLOC_ARENA: TgAllocArena = TgAllocArena([0; TG_ALLOC_ARENA_SIZE]);
static TG_ALLOC_OFFSET: AtomicUsize = AtomicUsize::new(0);

#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
pub extern "C" fn _start() -> ! {
    // 用户态运行时初始化顺序与内核类似：先 I/O，再堆，再进入 main。
    tg_console::init_console(&Console);
    tg_console::set_log_level(option_env!("LOG"));
    heap::init();

    unsafe extern "C" {
        fn main() -> i32;
    }

    // SAFETY: main 函数由用户程序提供，链接器保证其存在且符合 C ABI
    exit(unsafe { main() });
    unreachable!()
}

#[panic_handler]
fn panic_handler(panic_info: &core::panic::PanicInfo) -> ! {
    let err = panic_info.message();
    if let Some(location) = panic_info.location() {
        log::error!("Panicked at {}:{}, {err}", location.file(), location.line());
    } else {
        log::error!("Panicked: {err}");
    }
    exit(1);
    unreachable!()
}

pub fn getchar() -> u8 {
    getchar_blocking()
}

#[cfg(target_arch = "riscv64")]
pub fn framebuffer_info() -> Option<(*mut u8, usize, usize, usize)> {
    let fb_ptr: isize;
    let fb_len: usize;
    let width: usize;
    let height: usize;
    unsafe {
        core::arch::asm!(
            "ecall",
            inlateout("a0") 0isize => fb_ptr,
            lateout("a1") fb_len,
            lateout("a2") width,
            lateout("a3") height,
            in("a7") SYSCALL_FRAMEBUFFER,
        );
    }
    if fb_ptr <= 0 || fb_len == 0 || width == 0 || height == 0 {
        None
    } else {
        Some((fb_ptr as *mut u8, fb_len, width, height))
    }
}

#[cfg(not(target_arch = "riscv64"))]
pub fn framebuffer_info() -> Option<(*mut u8, usize, usize, usize)> {
    None
}

#[cfg(target_arch = "riscv64")]
pub fn framebuffer_flush() -> isize {
    let ret: isize;
    unsafe {
        core::arch::asm!(
            "ecall",
            inlateout("a0") 0isize => ret,
            in("a7") SYSCALL_FRAMEBUFFER_FLUSH,
        );
    }
    ret
}

#[cfg(not(target_arch = "riscv64"))]
pub fn framebuffer_flush() -> isize {
    -1
}

pub fn render_block(block: usize) -> isize {
    let Some((fb_ptr, fb_len, width, height)) = framebuffer_info() else {
        return -1;
    };

    let used_len = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .unwrap_or(0);
    if used_len == 0 || used_len > fb_len {
        return -1;
    }

    let framebuffer = unsafe { core::slice::from_raw_parts_mut(fb_ptr, used_len) };
    tangram::render_block_by_index(framebuffer, width, height, block);
    framebuffer_flush()
}

pub fn getchar_poll() -> Option<u8> {
    let mut c = [0u8; 1];
    match read(STDIN, &mut c) {
        1 => Some(c[0]),
        _ => None,
    }
}

pub fn getchar_blocking() -> u8 {
    loop {
        if let Some(c) = getchar_poll() {
            return c;
        }
        sched_yield();
    }
}

struct Console;

impl tg_console::Console for Console {
    #[inline]
    fn put_char(&self, c: u8) {
        tg_syscall::write(STDOUT, &[c]);
    }

    #[inline]
    fn put_str(&self, s: &str) {
        tg_syscall::write(STDOUT, s.as_bytes());
    }
}

pub fn sleep(period_ms: usize) {
    // 轮询时钟 + 主动让出 CPU 的教学实现，便于理解 time/yield 系统调用协作。
    let mut time: TimeSpec = TimeSpec::ZERO;
    clock_gettime(ClockId::CLOCK_MONOTONIC, &mut time as *mut _ as _);
    let time = time + TimeSpec::from_millsecond(period_ms);
    loop {
        let mut now: TimeSpec = TimeSpec::ZERO;
        clock_gettime(ClockId::CLOCK_MONOTONIC, &mut now as *mut _ as _);
        if now > time {
            break;
        }
        sched_yield();
    }
}

pub fn get_time() -> isize {
    let mut time: TimeSpec = TimeSpec::ZERO;
    clock_gettime(ClockId::CLOCK_MONOTONIC, &mut time as *mut _ as _);
    (time.tv_sec * 1000 + time.tv_nsec / 1_000_000) as isize
}

#[unsafe(no_mangle)]
pub extern "C" fn tg_sys_open(path: *const u8, flags: i32) -> i32 {
    if path.is_null() {
        return -1;
    }
    let mut len = 0usize;
    unsafe {
        while *path.add(len) != 0 {
            len += 1;
        }
    }
    let raw = unsafe { core::slice::from_raw_parts(path, len) };
    let mut start = 0usize;

    while start + 1 < raw.len() && raw[start] == b'.' && raw[start + 1] == b'/' {
        start += 2;
    }
    while start < raw.len() && (raw[start] == b'/' || raw[start] == b'\\') {
        start += 1;
    }

    let mut path_bytes = &raw[start..];
    if let Some(pos) = path_bytes.iter().rposition(|&b| b == b'/' || b == b'\\') {
        path_bytes = &path_bytes[pos + 1..];
    }
    if path_bytes.is_empty() {
        return -1;
    }

    let path_str = unsafe { core::str::from_utf8_unchecked(path_bytes) };
    open(path_str, OpenFlags::from_bits(flags as u32).unwrap_or(OpenFlags::RDONLY)) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn tg_sys_close(fd: i32) -> i32 {
    if fd < 0 {
        return -1;
    }
    close(fd as usize) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn tg_sys_read(fd: i32, buf: *mut u8, len: usize) -> i32 {
    if fd < 0 || buf.is_null() {
        return -1;
    }
    let data = unsafe { core::slice::from_raw_parts_mut(buf, len) };
    read(fd as usize, data) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn tg_sys_write(fd: i32, buf: *const u8, len: usize) -> i32 {
    
    if fd < 0 || buf.is_null() {
        return -1;
    }
    let data = unsafe { core::slice::from_raw_parts(buf, len) };
    write(fd as usize, data) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn tg_sys_unlink(path: *const u8) -> i32 {
    if path.is_null() {
        return -1;
    }
    let mut len = 0usize;
    unsafe {
        while *path.add(len) != 0 {
            len += 1;
        }
    }
    let raw = unsafe { core::slice::from_raw_parts(path, len) };
    let mut start = 0usize;

    while start + 1 < raw.len() && raw[start] == b'.' && raw[start + 1] == b'/' {
        start += 2;
    }
    while start < raw.len() && (raw[start] == b'/' || raw[start] == b'\\') {
        start += 1;
    }

    let mut path_bytes = &raw[start..];
    if let Some(pos) = path_bytes.iter().rposition(|&b| b == b'/' || b == b'\\') {
        path_bytes = &path_bytes[pos + 1..];
    }
    if path_bytes.is_empty() {
        return -1;
    }

    let path_str = unsafe { core::str::from_utf8_unchecked(path_bytes) };
    unlink(path_str) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn tg_alloc(size: usize) -> *mut u8 {
    let req = size.max(1);
    let align_mask = 8usize - 1;

    loop {
        let current = TG_ALLOC_OFFSET.load(Ordering::Relaxed);
        let aligned = (current + align_mask) & !align_mask;
        let Some(next) = aligned.checked_add(req) else {
            return core::ptr::null_mut();
        };
        if next > TG_ALLOC_ARENA_SIZE {
            return core::ptr::null_mut();
        }

        if TG_ALLOC_OFFSET
            .compare_exchange(current, next, Ordering::SeqCst, Ordering::Relaxed)
            .is_ok()
        {
            let base = core::ptr::addr_of_mut!(TG_ALLOC_ARENA) as *mut u8;
            unsafe {
                return base.add(aligned);
            }
        }
    }
}

pub fn trace_read(ptr: *const u8) -> Option<u8> {
    let ret = trace(0, ptr as usize, 0);
    if ret >= 0 && ret <= 255 {
        Some(ret as u8)
    } else {
        None
    }
}

pub fn trace_write(ptr: *const u8, value: u8) -> isize {
    trace(1, ptr as usize, value as usize)
}

pub fn count_syscall(syscall_id: usize) -> isize {
    trace(2, syscall_id, 0)
}

/// 从管道读取数据
/// 返回实际读取的总字节数，负数表示错误
pub fn pipe_read(pipe_fd: usize, buffer: &mut [u8]) -> isize {
    let mut total_read = 0usize;
    let len = buffer.len();
    loop {
        if total_read >= len {
            return total_read as isize;
        }
        let ret = read(pipe_fd, &mut buffer[total_read..]);
        if ret == -2 {
            // 暂时无数据，让出 CPU 后重试
            sched_yield();
            continue;
        } else if ret == 0 {
            // EOF，写端关闭
            return total_read as isize;
        } else if ret < 0 {
            // 其他错误
            return ret;
        } else {
            total_read += ret as usize;
        }
    }
}

/// 向管道写入数据
/// 返回实际写入的总字节数，负数表示错误
pub fn pipe_write(pipe_fd: usize, buffer: &[u8]) -> isize {
    let mut total_write = 0usize;
    let len = buffer.len();
    loop {
        if total_write >= len {
            return total_write as isize;
        }
        let ret = write(pipe_fd, &buffer[total_write..]);
        if ret == -2 {
            // 缓冲区满，让出 CPU 后重试
            sched_yield();
            continue;
        } else if ret < 0 {
            // 其他错误
            return ret;
        } else {
            total_write += ret as usize;
        }
    }
}
