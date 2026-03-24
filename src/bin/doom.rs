#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::{
    close, framebuffer_flush, framebuffer_info, get_time, getchar_poll, open, sched_yield,
    OpenFlags,
};

#[cfg(tg_doom_c)]
unsafe extern "C" {
    fn tg_doom_bind_framebuffer(fb: *mut u8, fb_len: usize, width: usize, height: usize);
    fn tg_doom_step(ticks_ms: u32, key: i32) -> i32;
    fn tg_doom_full_available() -> i32;
    fn tg_doom_full_init() -> i32;
    fn tg_doom_full_tick() -> i32;
}

const SYSCALL_SET_INPUT_MODE: usize = 0x1000_0003;
const MODE_POLLING: usize = 0;

#[repr(C)]
pub struct TgFramebufferInfo {
    pub ptr: *mut u8,
    pub len: usize,
    pub width: usize,
    pub height: usize,
}

#[inline]
fn custom_syscall1(id: usize, arg0: usize) -> isize {
    let ret: isize;
    unsafe {
        core::arch::asm!(
            "ecall",
            inlateout("a0") arg0 as isize => ret,
            in("a7") id,
        );
    }
    ret
}

#[inline]
fn set_input_mode_polling() -> isize {
    custom_syscall1(SYSCALL_SET_INPUT_MODE, MODE_POLLING)
}

#[unsafe(no_mangle)]
pub extern "C" fn tg_set_input_mode_polling() -> i32 {
    set_input_mode_polling() as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn tg_framebuffer_info(out: *mut TgFramebufferInfo) -> i32 {
    if out.is_null() {
        return -1;
    }
    let Some((fb_ptr, fb_len, width, height)) = framebuffer_info() else {
        return -1;
    };
    unsafe {
        (*out).ptr = fb_ptr;
        (*out).len = fb_len;
        (*out).width = width;
        (*out).height = height;
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn tg_framebuffer_flush() -> i32 {
    framebuffer_flush() as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn tg_get_ticks_ms() -> u32 {
    get_time().max(0) as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn tg_sleep_ms(ms: u32) {
    let start = get_time().max(0) as u32;
    let deadline = start.saturating_add(ms);
    while (get_time().max(0) as u32) < deadline {
        sched_yield();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn tg_getchar_poll() -> i32 {
    getchar_poll().map(|c| c as i32).unwrap_or(-1)
}

fn try_probe_wad() {
    let fd = open("doom1.wad\0", OpenFlags::RDONLY);
    if fd >= 0 {
        println!("doom: found doom1.wad (fd={fd})");
        let _ = close(fd as usize);
    } else {
        println!("doom: doom1.wad not found (open returned {fd})");
        println!("doom: continue with visual test");
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    if set_input_mode_polling() != 0 {
        println!("doom: set_input_mode(polling) failed");
    }

    let Some((fb_ptr, fb_len, width, height)) = framebuffer_info() else {
        println!("doom: framebuffer unavailable");
        return -1;
    };

    let used = width
        .checked_mul(height)
        .and_then(|px| px.checked_mul(4))
        .unwrap_or(0)
        .min(fb_len);
    if used == 0 {
        println!("doom: invalid framebuffer size");
        return -1;
    }

    let framebuffer = unsafe { core::slice::from_raw_parts_mut(fb_ptr, used) };
    let _ = framebuffer;

    #[cfg(not(tg_doom_c))]
    {
        println!("doom: tg_doom_c is not enabled");
        println!("doom: rebuild with TG_ENABLE_DOOM_C=1 TG_DOOM_FULL=1");
        return -1;
    }

    #[cfg(tg_doom_c)]
    unsafe {
        tg_doom_bind_framebuffer(fb_ptr, used, width, height);
    }

    println!("doom: bootstrap start");
    try_probe_wad();

    #[cfg(tg_doom_c)]
    let use_full_loop = unsafe { tg_doom_full_available() > 0 };

    #[cfg(not(tg_doom_c))]
    let use_full_loop = false;

    if !use_full_loop {
        println!("doom: full doomgeneric loop is unavailable");
        println!("doom: rebuild with TG_DOOM_FULL=1");
        return -1;
    }

    #[cfg(tg_doom_c)]
    {
        if unsafe { tg_doom_full_init() } != 0 {
            println!("doom: full-loop init failed");
            return -1;
        }
        println!("doom: running full doomgeneric loop");
        println!("doom: controls WASD move, J fire, SPACE use, Q acts as ESC/menu");
    }

    let mut next_frame_ms = get_time().max(0) as u32;
    const FRAME_TIME_MS: u32 = 28;

    loop {
        let now = get_time().max(0) as u32;
        if now < next_frame_ms {
            sched_yield();
            continue;
        }
        next_frame_ms = now.saturating_add(FRAME_TIME_MS);

        #[cfg(tg_doom_c)]
        {
            if use_full_loop {
                let ret = unsafe { tg_doom_full_tick() };
                if ret != 0 {
                    println!("doom: full-loop tick failed ({ret})");
                    return -1;
                }
            } else {
                let tick = get_time().max(0) as u32;
                let ret = unsafe { tg_doom_step(tick, -1) };
                if ret > 0 {
                    println!("doom: exit");
                    return 0;
                }
            }
        }
        sched_yield();
    }
}
