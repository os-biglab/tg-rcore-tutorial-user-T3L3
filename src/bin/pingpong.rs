#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::{
    exit, fork, framebuffer_flush, framebuffer_info, get_time, getpid, getchar_blocking,
    getchar_poll, sched_yield, sleep, waitpid,
};

const SYSCALL_SET_INPUT_MODE: usize = 0x1000_0003;
const SYSCALL_MQ_SEND: usize = 0x1000_0010;
const SYSCALL_MQ_RECV: usize = 0x1000_0011;

const TICK_MS: usize = 16;
const SCORE_TO_WIN: usize = 5;
const BASE_BALL_SPEED_X: i32 = 2;
const MAX_BALL_SPEED_X: i32 = 6;
const MAX_BALL_SPEED_Y: i32 = 3;

const COLOR_BG: u32 = 0xff0f1720;
const COLOR_FIELD: u32 = 0xff1b2632;
const COLOR_BORDER: u32 = 0xff334155;
const COLOR_P1: u32 = 0xff22c55e;
const COLOR_P2: u32 = 0xff3b82f6;
const COLOR_BALL: u32 = 0xfff8fafc;
const COLOR_SCORE_L: u32 = 0xffa855f7;
const COLOR_SCORE_R: u32 = 0xfff59e0b;
const COLOR_WIN: u32 = 0xffef4444;
const COLOR_MODE_POLL: u32 = 0xffffc107;
const COLOR_MODE_IRQ: u32 = 0xff42a5f5;

const MSG_P1_UP: u32 = 1;
const MSG_P1_DOWN: u32 = 2;
const MSG_P2_UP: u32 = 3;
const MSG_P2_DOWN: u32 = 4;
const MSG_TICK: u32 = 5;
const MSG_QUIT: u32 = 255;

#[derive(Clone, Copy)]
enum InputMode {
    Polling,
    InterruptLike,
}

#[derive(Clone, Copy)]
struct Paddle {
    y: i32,
}

#[derive(Clone, Copy)]
struct Ball {
    x: i32,
    y: i32,
    vx: i32,
    vy: i32,
}

#[derive(Clone, Copy)]
struct Layout {
    width: usize,
    height: usize,
    field_x: usize,
    field_y: usize,
    field_w: usize,
    field_h: usize,
    paddle_w: usize,
    paddle_h: usize,
    ball_s: usize,
    p1_x: usize,
    p2_x: usize,
}

struct Game {
    p1: Paddle,
    p2: Paddle,
    ball: Ball,
    score_l: usize,
    score_r: usize,
    running: bool,
    quit: bool,
}

struct RenderState {
    inited: bool,
    prev_p1_y: i32,
    prev_p2_y: i32,
    prev_ball_x: i32,
    prev_ball_y: i32,
    prev_score_l: usize,
    prev_score_r: usize,
    prev_running: bool,
    prev_mode: u8,
}

impl RenderState {
    fn new() -> Self {
        Self {
            inited: false,
            prev_p1_y: 0,
            prev_p2_y: 0,
            prev_ball_x: 0,
            prev_ball_y: 0,
            prev_score_l: 0,
            prev_score_r: 0,
            prev_running: true,
            prev_mode: 0,
        }
    }
}

impl Game {
    fn new(layout: &Layout) -> Self {
        let center_y = (layout.field_h / 2) as i32;
        Self {
            p1: Paddle {
                y: center_y - (layout.paddle_h / 2) as i32,
            },
            p2: Paddle {
                y: center_y - (layout.paddle_h / 2) as i32,
            },
            ball: Ball {
                x: (layout.field_w / 2) as i32,
                y: (layout.field_h / 2) as i32,
                vx: BASE_BALL_SPEED_X,
                vy: 1,
            },
            score_l: 0,
            score_r: 0,
            running: true,
            quit: false,
        }
    }

    fn clamp_paddles(&mut self, layout: &Layout) {
        let max_y = layout.field_h.saturating_sub(layout.paddle_h) as i32;
        if self.p1.y < 0 {
            self.p1.y = 0;
        }
        if self.p2.y < 0 {
            self.p2.y = 0;
        }
        if self.p1.y > max_y {
            self.p1.y = max_y;
        }
        if self.p2.y > max_y {
            self.p2.y = max_y;
        }
    }

    fn reset_ball(&mut self, layout: &Layout, to_left: bool) {
        let total_score = (self.score_l + self.score_r) as i32;
        let speed_x = (BASE_BALL_SPEED_X + total_score / 2).min(MAX_BALL_SPEED_X);
        let speed_y = (1 + total_score / 3).min(MAX_BALL_SPEED_Y);
        self.ball.x = (layout.field_w / 2) as i32;
        self.ball.y = (layout.field_h / 2) as i32;
        self.ball.vx = if to_left { -speed_x } else { speed_x };
        self.ball.vy = if (self.score_l + self.score_r) % 2 == 0 {
            speed_y
        } else {
            -speed_y
        };
    }

    fn boost_after_paddle_hit(&mut self) {
        let vx_sign = if self.ball.vx >= 0 { 1 } else { -1 };
        let next_vx = (self.ball.vx.abs() + 1).min(MAX_BALL_SPEED_X);
        self.ball.vx = vx_sign * next_vx;

        let vy_sign = if self.ball.vy >= 0 { 1 } else { -1 };
        let next_vy = (self.ball.vy.abs() + 1).min(MAX_BALL_SPEED_Y);
        self.ball.vy = vy_sign * next_vy;
    }

    fn on_tick(&mut self, layout: &Layout) {
        if !self.running || self.quit {
            return;
        }

        self.ball.x += self.ball.vx;
        self.ball.y += self.ball.vy;

        if self.ball.y <= 0 {
            self.ball.y = 0;
            self.ball.vy = -self.ball.vy;
        }
        let max_ball_y = layout.field_h.saturating_sub(layout.ball_s) as i32;
        if self.ball.y >= max_ball_y {
            self.ball.y = max_ball_y;
            self.ball.vy = -self.ball.vy;
        }

        let ball_left = self.ball.x;
        let ball_right = self.ball.x + layout.ball_s as i32;
        let ball_top = self.ball.y;
        let ball_bottom = self.ball.y + layout.ball_s as i32;

        let p1_top = self.p1.y;
        let p1_bottom = self.p1.y + layout.paddle_h as i32;
        let p2_top = self.p2.y;
        let p2_bottom = self.p2.y + layout.paddle_h as i32;
        let p1_left = layout.p1_x.saturating_sub(layout.field_x) as i32;
        let p1_right = p1_left + layout.paddle_w as i32;
        let p2_left = layout.p2_x.saturating_sub(layout.field_x) as i32;
        let p2_right = p2_left + layout.paddle_w as i32;

        if ball_left <= p1_right
            && ball_right >= p1_left
            && ball_bottom >= p1_top
            && ball_top <= p1_bottom
            && self.ball.vx < 0
        {
            self.ball.x = p1_right;
            self.ball.vx = -self.ball.vx;
            self.boost_after_paddle_hit();
        }

        if ball_right >= p2_left
            && ball_left <= p2_right
            && ball_bottom >= p2_top
            && ball_top <= p2_bottom
            && self.ball.vx > 0
        {
            self.ball.x = p2_left - layout.ball_s as i32;
            self.ball.vx = -self.ball.vx;
            self.boost_after_paddle_hit();
        }

        if self.ball.x < 0 {
            self.score_r = self.score_r.saturating_add(1);
            self.reset_ball(layout, false);
        } else if self.ball.x as usize > layout.field_w {
            self.score_l = self.score_l.saturating_add(1);
            self.reset_ball(layout, true);
        }

        if self.score_l >= SCORE_TO_WIN || self.score_r >= SCORE_TO_WIN {
            self.running = false;
        }
    }
}

#[inline]
fn mode_id(mode: InputMode) -> u8 {
    match mode {
        InputMode::Polling => 0,
        InputMode::InterruptLike => 1,
    }
}

#[inline]
fn mode_color(mode: InputMode) -> u32 {
    match mode {
        InputMode::Polling => COLOR_MODE_POLL,
        InputMode::InterruptLike => COLOR_MODE_IRQ,
    }
}

fn custom_syscall2(id: usize, a0: usize, a1: usize) -> isize {
    let ret: isize;
    unsafe {
        core::arch::asm!(
            "ecall",
            inlateout("a0") a0 as isize => ret,
            in("a1") a1,
            in("a7") id,
        );
    }
    ret
}

fn custom_syscall1(id: usize, a0: usize) -> isize {
    let ret: isize;
    unsafe {
        core::arch::asm!(
            "ecall",
            inlateout("a0") a0 as isize => ret,
            in("a7") id,
        );
    }
    ret
}

fn custom_syscall0(id: usize) -> isize {
    let ret: isize;
    unsafe {
        core::arch::asm!(
            "ecall",
            inlateout("a0") 0isize => ret,
            in("a7") id,
        );
    }
    ret
}

fn set_input_mode(mode: InputMode) -> isize {
    custom_syscall1(SYSCALL_SET_INPUT_MODE, mode_id(mode) as usize)
}

fn mq_send(to_pid: usize, message: u32) -> isize {
    custom_syscall2(SYSCALL_MQ_SEND, to_pid, message as usize)
}

fn mq_recv() -> Option<u32> {
    let ret = custom_syscall0(SYSCALL_MQ_RECV);
    if ret < 0 || ret as usize == usize::MAX {
        None
    } else {
        Some(ret as u32)
    }
}

fn select_mode() -> InputMode {
    println!("PingPong mode: [p] polling / [i] interrupt");
    loop {
        match getchar_blocking() {
            b'p' | b'P' => return InputMode::Polling,
            b'i' | b'I' => return InputMode::InterruptLike,
            _ => {}
        }
    }
}

fn build_layout(width: usize, height: usize) -> Layout {
    let field_w = width.saturating_mul(9) / 10;
    let field_h = height.saturating_mul(8) / 10;
    let field_x = (width.saturating_sub(field_w)) / 2;
    let field_y = (height.saturating_sub(field_h)) / 2;
    let paddle_w = core::cmp::max(6, field_w / 80);
    let paddle_h = core::cmp::max(30, field_h / 6);
    let ball_s = core::cmp::max(6, field_w / 90);
    let p1_x = field_x + 12;
    let p2_x = field_x + field_w.saturating_sub(12 + paddle_w);
    Layout {
        width,
        height,
        field_x,
        field_y,
        field_w,
        field_h,
        paddle_w,
        paddle_h,
        ball_s,
        p1_x,
        p2_x,
    }
}

fn put_pixel(framebuffer: &mut [u8], width: usize, x: usize, y: usize, color: u32) {
    if x >= width {
        return;
    }
    let idx = (y * width + x) * 4;
    if idx + 4 <= framebuffer.len() {
        framebuffer[idx..idx + 4].copy_from_slice(&color.to_le_bytes());
    }
}

fn fill_rect(
    framebuffer: &mut [u8],
    width: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    color: u32,
) {
    let mut yy = y;
    while yy < y + h {
        let mut xx = x;
        while xx < x + w {
            put_pixel(framebuffer, width, xx, yy, color);
            xx += 1;
        }
        yy += 1;
    }
}

fn draw_score(framebuffer: &mut [u8], layout: &Layout, score: usize, left: bool, color: u32) {
    let max_seg_w = core::cmp::min(layout.field_w / 3, 120);
    let bar_w = core::cmp::min(max_seg_w, score.saturating_mul(14));
    let x = if left {
        layout.field_x + 8
    } else {
        layout.field_x + layout.field_w.saturating_sub(8 + max_seg_w)
    };
    let y = layout.field_y.saturating_sub(24);
    fill_rect(framebuffer, layout.width, x, y, max_seg_w, 10, COLOR_BORDER);
    if bar_w > 0 {
        fill_rect(framebuffer, layout.width, x, y, bar_w, 10, color);
    }
}

fn render(game: &Game, mode: InputMode, state: &mut RenderState) {
    let Some((fb_ptr, fb_len, width, height)) = framebuffer_info() else {
        return;
    };
    let used_len = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .unwrap_or(0);
    if used_len == 0 || used_len > fb_len {
        return;
    }

    let layout = build_layout(width, height);
    let fb = unsafe { core::slice::from_raw_parts_mut(fb_ptr, used_len) };

    if !state.inited {
        fill_rect(fb, layout.width, 0, 0, layout.width, layout.height, COLOR_BG);
        fill_rect(
            fb,
            layout.width,
            layout.field_x.saturating_sub(3),
            layout.field_y.saturating_sub(3),
            layout.field_w + 6,
            layout.field_h + 6,
            COLOR_BORDER,
        );
        fill_rect(
            fb,
            layout.width,
            layout.field_x,
            layout.field_y,
            layout.field_w,
            layout.field_h,
            COLOR_FIELD,
        );
    }

    if !state.inited || state.prev_mode != mode_id(mode) {
        fill_rect(
            fb,
            layout.width,
            layout.field_x,
            layout.field_y.saturating_sub(40),
            layout.field_w,
            8,
            mode_color(mode),
        );
    }

    if !state.inited || state.prev_p1_y != game.p1.y {
        if state.inited {
            fill_rect(
                fb,
                layout.width,
                layout.p1_x,
                layout.field_y + state.prev_p1_y.max(0) as usize,
                layout.paddle_w,
                layout.paddle_h,
                COLOR_FIELD,
            );
        }
        fill_rect(
            fb,
            layout.width,
            layout.p1_x,
            layout.field_y + game.p1.y.max(0) as usize,
            layout.paddle_w,
            layout.paddle_h,
            COLOR_P1,
        );
    }

    if !state.inited || state.prev_p2_y != game.p2.y {
        if state.inited {
            fill_rect(
                fb,
                layout.width,
                layout.p2_x,
                layout.field_y + state.prev_p2_y.max(0) as usize,
                layout.paddle_w,
                layout.paddle_h,
                COLOR_FIELD,
            );
        }
        fill_rect(
            fb,
            layout.width,
            layout.p2_x,
            layout.field_y + game.p2.y.max(0) as usize,
            layout.paddle_w,
            layout.paddle_h,
            COLOR_P2,
        );
    }

    if !state.inited || state.prev_ball_x != game.ball.x || state.prev_ball_y != game.ball.y {
        if state.inited {
            fill_rect(
                fb,
                layout.width,
                layout.field_x + state.prev_ball_x.max(0) as usize,
                layout.field_y + state.prev_ball_y.max(0) as usize,
                layout.ball_s,
                layout.ball_s,
                COLOR_FIELD,
            );
        }
        fill_rect(
            fb,
            layout.width,
            layout.field_x + game.ball.x.max(0) as usize,
            layout.field_y + game.ball.y.max(0) as usize,
            layout.ball_s,
            layout.ball_s,
            COLOR_BALL,
        );
    }

    if !state.inited || state.prev_score_l != game.score_l {
        draw_score(fb, &layout, game.score_l, true, COLOR_SCORE_L);
    }
    if !state.inited || state.prev_score_r != game.score_r {
        draw_score(fb, &layout, game.score_r, false, COLOR_SCORE_R);
    }

    if !game.running {
        fill_rect(
            fb,
            layout.width,
            layout.field_x + layout.field_w / 4,
            layout.field_y + layout.field_h / 2 - 8,
            layout.field_w / 2,
            16,
            COLOR_WIN,
        );
    } else if state.prev_running == false {
        fill_rect(
            fb,
            layout.width,
            layout.field_x + layout.field_w / 4,
            layout.field_y + layout.field_h / 2 - 8,
            layout.field_w / 2,
            16,
            COLOR_FIELD,
        );
    }

    state.inited = true;
    state.prev_p1_y = game.p1.y;
    state.prev_p2_y = game.p2.y;
    state.prev_ball_x = game.ball.x;
    state.prev_ball_y = game.ball.y;
    state.prev_score_l = game.score_l;
    state.prev_score_r = game.score_r;
    state.prev_running = game.running;
    state.prev_mode = mode_id(mode);

    let _ = framebuffer_flush();
}

fn input_worker(game_pid: usize) -> ! {
    loop {
        while let Some(msg) = mq_recv() {
            if msg == MSG_QUIT {
                exit(0);
            }
        }
        while let Some(c) = getchar_poll() {
            let msg = match c {
                b'w' | b'W' => Some(MSG_P1_UP),
                b's' | b'S' => Some(MSG_P1_DOWN),
                b'i' | b'I' => Some(MSG_P2_UP),
                b'k' | b'K' => Some(MSG_P2_DOWN),
                b'q' | b'Q' => Some(MSG_QUIT),
                _ => None,
            };
            if let Some(msg) = msg {
                let _ = mq_send(game_pid, msg);
            }
        }
        sched_yield();
    }
}

fn ticker_worker(game_pid: usize) -> ! {
    loop {
        while let Some(msg) = mq_recv() {
            if msg == MSG_QUIT {
                exit(0);
            }
        }
        sleep(TICK_MS);
        let _ = mq_send(game_pid, MSG_TICK);
    }
}

#[unsafe(no_mangle)]
extern "C" fn main() -> i32 {
    let mode = select_mode();
    if set_input_mode(mode) != 0 {
        println!("set_input_mode failed");
        return -1;
    }

    let Some((_fb_ptr, _fb_len, width, height)) = framebuffer_info() else {
        println!("framebuffer unavailable");
        return -1;
    };
    let layout = build_layout(width, height);

    let game_pid = getpid() as usize;
    let input_pid = fork();
    if input_pid == 0 {
        input_worker(game_pid);
    }
    let ticker_pid = fork();
    if ticker_pid == 0 {
        ticker_worker(game_pid);
    }

    let mut game = Game::new(&layout);
    let mut render_state = RenderState::new();
    render(&game, mode, &mut render_state);

    let mut last_log = get_time();
    while !game.quit {
        let mut dirty = false;
        while let Some(msg) = mq_recv() {
            match msg {
                MSG_P1_UP => {
                    game.p1.y -= 10;
                    dirty = true;
                }
                MSG_P1_DOWN => {
                    game.p1.y += 10;
                    dirty = true;
                }
                MSG_P2_UP => {
                    game.p2.y -= 10;
                    dirty = true;
                }
                MSG_P2_DOWN => {
                    game.p2.y += 10;
                    dirty = true;
                }
                MSG_TICK => {
                    game.on_tick(&layout);
                    dirty = true;
                }
                MSG_QUIT => {
                    game.quit = true;
                    dirty = true;
                }
                _ => {}
            }
        }

        game.clamp_paddles(&layout);

        if dirty {
            render(&game, mode, &mut render_state);
        }

        let now = get_time();
        if now - last_log >= 1000 {
            println!(
                "PingPong score L:{} R:{}{}",
                game.score_l,
                game.score_r,
                if game.running { "" } else { " [finished]" }
            );
            last_log = now;
        }

        if !game.running {
            game.quit = true;
        }

        sched_yield();
    }

    let _ = mq_send(input_pid as usize, MSG_QUIT);
    let _ = mq_send(ticker_pid as usize, MSG_QUIT);

    let mut code: i32 = 0;
    let _ = waitpid(input_pid as isize, &mut code);
    let _ = waitpid(ticker_pid as isize, &mut code);

    render(&game, mode, &mut render_state);
    println!("PingPong end. L:{} R:{}", game.score_l, game.score_r);
    0
}
