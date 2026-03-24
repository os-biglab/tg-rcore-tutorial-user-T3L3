#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::{
    close, framebuffer_flush, framebuffer_info, get_time, getchar_blocking, getchar_poll, open,
    read, sched_yield, write, OpenFlags,
};

const SYSCALL_SET_INPUT_MODE: usize = 0x1000_0003;

const TICK_MS: isize = 20;
const SAVE_FILE: &str = "breakout.sav\0";

const BRICK_ROWS: usize = 5;
const BRICK_COLS: usize = 10;
const BRICK_COUNT: usize = BRICK_ROWS * BRICK_COLS;
const SAVE_MAGIC: u32 = 0x4252_4b54;
const SAVE_BYTES: usize = 44;

const COLOR_BG: u32 = 0xff0f172a;
const COLOR_FIELD: u32 = 0xff111827;
const COLOR_BORDER: u32 = 0xff334155;
const COLOR_PADDLE: u32 = 0xff22c55e;
const COLOR_BALL: u32 = 0xfff8fafc;
const COLOR_BRICK_A: u32 = 0xffef4444;
const COLOR_BRICK_B: u32 = 0xfff59e0b;
const COLOR_BRICK_C: u32 = 0xff3b82f6;
const COLOR_BRICK_D: u32 = 0xffa855f7;
const COLOR_BRICK_E: u32 = 0xff14b8a6;
const COLOR_SCORE: u32 = 0xffeab308;
const COLOR_LIFE: u32 = 0xff22d3ee;
const COLOR_END: u32 = 0xfff97316;
const COLOR_PAUSE: u32 = 0xff94a3b8;
const COLOR_MODE_POLL: u32 = 0xffffc107;
const COLOR_MODE_IRQ: u32 = 0xff42a5f5;

#[derive(Clone, Copy)]
enum InputMode {
    Polling,
    InterruptLike,
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
    brick_top: usize,
    brick_gap: usize,
    brick_w: usize,
    brick_h: usize,
}

#[derive(Clone, Copy)]
struct Ball {
    x: i32,
    y: i32,
    vx: i32,
    vy: i32,
}

struct Game {
    paddle_x: i32,
    ball: Ball,
    score: usize,
    lives: usize,
    bricks: [bool; BRICK_COUNT],
    running: bool,
    paused: bool,
    quit: bool,
}

struct RenderState {
    inited: bool,
    prev_paddle_x: i32,
    prev_ball_x: i32,
    prev_ball_y: i32,
    prev_score: usize,
    prev_lives: usize,
    prev_running: bool,
    prev_paused: bool,
    prev_mode: u8,
    prev_bricks: [bool; BRICK_COUNT],
}

impl RenderState {
    fn new() -> Self {
        Self {
            inited: false,
            prev_paddle_x: 0,
            prev_ball_x: 0,
            prev_ball_y: 0,
            prev_score: 0,
            prev_lives: 0,
            prev_running: true,
            prev_paused: false,
            prev_mode: 0,
            prev_bricks: [true; BRICK_COUNT],
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

fn set_input_mode(mode: InputMode) -> isize {
    custom_syscall1(SYSCALL_SET_INPUT_MODE, mode_id(mode) as usize)
}

fn select_mode() -> InputMode {
    println!("Breakout mode: [p] polling / [i] interrupt");
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
    let field_y = (height.saturating_sub(field_h)) / 2 + 12;
    let paddle_w = core::cmp::max(80, field_w / 7);
    let paddle_h = core::cmp::max(10, field_h / 35);
    let ball_s = core::cmp::max(8, field_w / 100);
    let brick_gap = core::cmp::max(2, field_w / 220);
    let brick_w = (field_w.saturating_sub(brick_gap * (BRICK_COLS + 1))) / BRICK_COLS;
    let brick_h = core::cmp::max(14, field_h / 28);
    let brick_top = 16;
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
        brick_top,
        brick_gap,
        brick_w,
        brick_h,
    }
}

impl Game {
    fn new(layout: &Layout) -> Self {
        let mut game = Self {
            paddle_x: (layout.field_w / 2).saturating_sub(layout.paddle_w / 2) as i32,
            ball: Ball {
                x: (layout.field_w / 2) as i32,
                y: (layout.field_h.saturating_sub(layout.paddle_h + 24)) as i32,
                vx: 2,
                vy: -2,
            },
            score: 0,
            lives: 3,
            bricks: [true; BRICK_COUNT],
            running: true,
            paused: false,
            quit: false,
        };
        game.reset_ball(layout);
        game
    }

    fn reset_ball(&mut self, layout: &Layout) {
        self.ball.x = (layout.field_w / 2).saturating_sub(layout.ball_s / 2) as i32;
        self.ball.y = (layout
            .field_h
            .saturating_sub(layout.paddle_h + layout.ball_s + 10)) as i32;
        self.ball.vx = if self.score % 2 == 0 { 2 } else { -2 };
        self.ball.vy = -2;
    }

    fn paddle_y(&self, layout: &Layout) -> i32 {
        layout.field_h.saturating_sub(layout.paddle_h + 8) as i32
    }

    fn move_paddle(&mut self, layout: &Layout, delta: i32) {
        self.paddle_x += delta;
        let max_x = layout.field_w.saturating_sub(layout.paddle_w) as i32;
        if self.paddle_x < 0 {
            self.paddle_x = 0;
        }
        if self.paddle_x > max_x {
            self.paddle_x = max_x;
        }
    }

    fn won(&self) -> bool {
        self.bricks.iter().all(|alive| !*alive)
    }

    fn on_tick(&mut self, layout: &Layout) {
        if !self.running || self.paused || self.quit {
            return;
        }

        self.ball.x += self.ball.vx;
        self.ball.y += self.ball.vy;

        if self.ball.x <= 0 {
            self.ball.x = 0;
            self.ball.vx = -self.ball.vx;
        }
        let max_x = layout.field_w.saturating_sub(layout.ball_s) as i32;
        if self.ball.x >= max_x {
            self.ball.x = max_x;
            self.ball.vx = -self.ball.vx;
        }

        if self.ball.y <= 0 {
            self.ball.y = 0;
            self.ball.vy = -self.ball.vy;
        }

        let paddle_y = self.paddle_y(layout);
        let ball_left = self.ball.x;
        let ball_right = self.ball.x + layout.ball_s as i32;
        let ball_top = self.ball.y;
        let ball_bottom = self.ball.y + layout.ball_s as i32;

        let paddle_left = self.paddle_x;
        let paddle_right = self.paddle_x + layout.paddle_w as i32;
        let paddle_top = paddle_y;
        let paddle_bottom = paddle_y + layout.paddle_h as i32;

        if self.ball.vy > 0
            && ball_bottom >= paddle_top
            && ball_top <= paddle_bottom
            && ball_right >= paddle_left
            && ball_left <= paddle_right
        {
            self.ball.y = paddle_top - layout.ball_s as i32;
            self.ball.vy = -self.ball.vy.abs();
            let hit = (self.ball.x + layout.ball_s as i32 / 2) - (paddle_left + layout.paddle_w as i32 / 2);
            let mut next_vx = hit / core::cmp::max(1, layout.paddle_w as i32 / 6);
            next_vx = next_vx.clamp(-4, 4);
            if next_vx == 0 {
                next_vx = if self.score % 2 == 0 { 1 } else { -1 };
            }
            self.ball.vx = next_vx;
        }

        for row in 0..BRICK_ROWS {
            for col in 0..BRICK_COLS {
                let idx = row * BRICK_COLS + col;
                if !self.bricks[idx] {
                    continue;
                }
                let bx = (layout.brick_gap + col * (layout.brick_w + layout.brick_gap)) as i32;
                let by = (layout.brick_top + row * (layout.brick_h + layout.brick_gap)) as i32;
                let bw = layout.brick_w as i32;
                let bh = layout.brick_h as i32;

                let b_left = bx;
                let b_right = bx + bw;
                let b_top = by;
                let b_bottom = by + bh;

                if ball_right >= b_left
                    && ball_left <= b_right
                    && ball_bottom >= b_top
                    && ball_top <= b_bottom
                {
                    self.bricks[idx] = false;
                    self.score = self.score.saturating_add(10);

                    let overlap_x = core::cmp::min(ball_right, b_right) - core::cmp::max(ball_left, b_left);
                    let overlap_y = core::cmp::min(ball_bottom, b_bottom) - core::cmp::max(ball_top, b_top);
                    if overlap_x < overlap_y {
                        self.ball.vx = -self.ball.vx;
                    } else {
                        self.ball.vy = -self.ball.vy;
                    }

                    if self.won() {
                        self.running = false;
                    }
                    return;
                }
            }
        }

        if self.ball.y as usize > layout.field_h {
            if self.lives > 0 {
                self.lives -= 1;
            }
            if self.lives == 0 {
                self.running = false;
            } else {
                self.reset_ball(layout);
            }
        }
    }

    fn save(&self) -> bool {
        let mut buf = [0u8; SAVE_BYTES];
        encode_save(self, &mut buf);
        let fd = open(SAVE_FILE, OpenFlags::CREATE | OpenFlags::WRONLY);
        if fd < 0 {
            return false;
        }
        let fd = fd as usize;
        let ok = write_all(fd, &buf);
        let _ = close(fd);
        ok
    }

    fn load(&mut self) -> bool {
        let fd = open(SAVE_FILE, OpenFlags::RDONLY);
        if fd < 0 {
            return false;
        }
        let fd = fd as usize;
        let mut buf = [0u8; SAVE_BYTES];
        let ok = read_exact(fd, &mut buf);
        let _ = close(fd);
        if !ok {
            return false;
        }
        decode_save(self, &buf)
    }
}

fn write_all(fd: usize, data: &[u8]) -> bool {
    let mut offset = 0usize;
    while offset < data.len() {
        let ret = write(fd, &data[offset..]);
        if ret <= 0 {
            return false;
        }
        offset += ret as usize;
    }
    true
}

fn read_exact(fd: usize, data: &mut [u8]) -> bool {
    let mut offset = 0usize;
    while offset < data.len() {
        let ret = read(fd, &mut data[offset..]);
        if ret <= 0 {
            return false;
        }
        offset += ret as usize;
    }
    true
}

fn put_u32(buf: &mut [u8], at: &mut usize, value: u32) {
    let bytes = value.to_le_bytes();
    buf[*at..*at + 4].copy_from_slice(&bytes);
    *at += 4;
}

fn put_i32(buf: &mut [u8], at: &mut usize, value: i32) {
    put_u32(buf, at, value as u32);
}

fn get_u32(buf: &[u8], at: &mut usize) -> u32 {
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&buf[*at..*at + 4]);
    *at += 4;
    u32::from_le_bytes(bytes)
}

fn get_i32(buf: &[u8], at: &mut usize) -> i32 {
    get_u32(buf, at) as i32
}

fn encode_bricks(bricks: &[bool; BRICK_COUNT]) -> u64 {
    let mut bits = 0u64;
    for (i, alive) in bricks.iter().enumerate() {
        if *alive {
            bits |= 1u64 << i;
        }
    }
    bits
}

fn decode_bricks(bricks: &mut [bool; BRICK_COUNT], bits: u64) {
    for i in 0..BRICK_COUNT {
        bricks[i] = (bits & (1u64 << i)) != 0;
    }
}

fn encode_save(game: &Game, out: &mut [u8; SAVE_BYTES]) {
    let mut at = 0usize;
    put_u32(out, &mut at, SAVE_MAGIC);
    put_u32(out, &mut at, game.score as u32);
    put_u32(out, &mut at, game.lives as u32);
    put_i32(out, &mut at, game.paddle_x);
    put_i32(out, &mut at, game.ball.x);
    put_i32(out, &mut at, game.ball.y);
    put_i32(out, &mut at, game.ball.vx);
    put_i32(out, &mut at, game.ball.vy);
    let flags = (game.running as u32) | ((game.paused as u32) << 1) | ((game.quit as u32) << 2);
    put_u32(out, &mut at, flags);
    let brick_bits = encode_bricks(&game.bricks).to_le_bytes();
    out[at..at + 8].copy_from_slice(&brick_bits);
}

fn decode_save(game: &mut Game, input: &[u8; SAVE_BYTES]) -> bool {
    let mut at = 0usize;
    if get_u32(input, &mut at) != SAVE_MAGIC {
        return false;
    }
    game.score = get_u32(input, &mut at) as usize;
    game.lives = get_u32(input, &mut at) as usize;
    game.paddle_x = get_i32(input, &mut at);
    game.ball.x = get_i32(input, &mut at);
    game.ball.y = get_i32(input, &mut at);
    game.ball.vx = get_i32(input, &mut at);
    game.ball.vy = get_i32(input, &mut at);
    let flags = get_u32(input, &mut at);
    game.running = (flags & 1) != 0;
    game.paused = (flags & 2) != 0;
    game.quit = (flags & 4) != 0;
    let mut bits = [0u8; 8];
    bits.copy_from_slice(&input[at..at + 8]);
    decode_bricks(&mut game.bricks, u64::from_le_bytes(bits));
    true
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

fn fill_rect(framebuffer: &mut [u8], width: usize, x: usize, y: usize, w: usize, h: usize, color: u32) {
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

fn render(game: &Game, mode: InputMode, layout: &Layout, state: &mut RenderState, force_full: bool) {
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
    let fb = unsafe { core::slice::from_raw_parts_mut(fb_ptr, used_len) };

    let full = force_full || !state.inited;

    if full {
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

    if full || state.prev_mode != mode_id(mode) {
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

    if full || state.prev_score != game.score {
        let score_w = core::cmp::min(layout.field_w / 2, game.score.saturating_mul(2));
        fill_rect(
            fb,
            layout.width,
            layout.field_x + 8,
            layout.field_y.saturating_sub(24),
            layout.field_w / 2,
            8,
            COLOR_BORDER,
        );
        if score_w > 0 {
            fill_rect(
                fb,
                layout.width,
                layout.field_x + 8,
                layout.field_y.saturating_sub(24),
                score_w,
                8,
                COLOR_SCORE,
            );
        }
    }

    if full || state.prev_lives != game.lives {
        fill_rect(
            fb,
            layout.width,
            layout.field_x + layout.field_w.saturating_sub(120),
            layout.field_y.saturating_sub(28),
            120,
            14,
            COLOR_BG,
        );
        for i in 0..game.lives.min(5) {
            let x = layout.field_x + layout.field_w.saturating_sub(14 + i * 16);
            fill_rect(fb, layout.width, x, layout.field_y.saturating_sub(26), 10, 10, COLOR_LIFE);
        }
    }

    if full {
        for row in 0..BRICK_ROWS {
            for col in 0..BRICK_COLS {
                let idx = row * BRICK_COLS + col;
                if !game.bricks[idx] {
                    continue;
                }
                let x = layout.field_x + layout.brick_gap + col * (layout.brick_w + layout.brick_gap);
                let y = layout.field_y + layout.brick_top + row * (layout.brick_h + layout.brick_gap);
                let color = match row {
                    0 => COLOR_BRICK_A,
                    1 => COLOR_BRICK_B,
                    2 => COLOR_BRICK_C,
                    3 => COLOR_BRICK_D,
                    _ => COLOR_BRICK_E,
                };
                fill_rect(fb, layout.width, x, y, layout.brick_w, layout.brick_h, color);
            }
        }
    } else {
        for row in 0..BRICK_ROWS {
            for col in 0..BRICK_COLS {
                let idx = row * BRICK_COLS + col;
                if state.prev_bricks[idx] == game.bricks[idx] {
                    continue;
                }
                let x = layout.field_x + layout.brick_gap + col * (layout.brick_w + layout.brick_gap);
                let y = layout.field_y + layout.brick_top + row * (layout.brick_h + layout.brick_gap);
                if game.bricks[idx] {
                    let color = match row {
                        0 => COLOR_BRICK_A,
                        1 => COLOR_BRICK_B,
                        2 => COLOR_BRICK_C,
                        3 => COLOR_BRICK_D,
                        _ => COLOR_BRICK_E,
                    };
                    fill_rect(fb, layout.width, x, y, layout.brick_w, layout.brick_h, color);
                } else {
                    fill_rect(fb, layout.width, x, y, layout.brick_w, layout.brick_h, COLOR_FIELD);
                }
            }
        }
    }

    let paddle_y = game.paddle_y(layout) as usize;
    if full || state.prev_paddle_x != game.paddle_x {
        if !full {
            fill_rect(
                fb,
                layout.width,
                layout.field_x + state.prev_paddle_x.max(0) as usize,
                layout.field_y + paddle_y,
                layout.paddle_w,
                layout.paddle_h,
                COLOR_FIELD,
            );
        }
        fill_rect(
            fb,
            layout.width,
            layout.field_x + game.paddle_x.max(0) as usize,
            layout.field_y + paddle_y,
            layout.paddle_w,
            layout.paddle_h,
            COLOR_PADDLE,
        );
    }

    if full || state.prev_ball_x != game.ball.x || state.prev_ball_y != game.ball.y {
        if !full {
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

    if full || state.prev_paused != game.paused || state.prev_running != game.running {
        fill_rect(
            fb,
            layout.width,
            layout.field_x + layout.field_w / 4,
            layout.field_y + layout.field_h / 2 - 12,
            layout.field_w / 2,
            24,
            COLOR_FIELD,
        );
    }

    if game.paused && game.running {
        fill_rect(
            fb,
            layout.width,
            layout.field_x + layout.field_w / 3,
            layout.field_y + layout.field_h / 2 - 8,
            layout.field_w / 3,
            16,
            COLOR_PAUSE,
        );
    }

    if !game.running {
        fill_rect(
            fb,
            layout.width,
            layout.field_x + layout.field_w / 4,
            layout.field_y + layout.field_h / 2 - 10,
            layout.field_w / 2,
            20,
            COLOR_END,
        );
    }

    state.inited = true;
    state.prev_paddle_x = game.paddle_x;
    state.prev_ball_x = game.ball.x;
    state.prev_ball_y = game.ball.y;
    state.prev_score = game.score;
    state.prev_lives = game.lives;
    state.prev_running = game.running;
    state.prev_paused = game.paused;
    state.prev_mode = mode_id(mode);
    state.prev_bricks = game.bricks;

    let _ = framebuffer_flush();
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
    let mut game = Game::new(&layout);
    let mut render_state = RenderState::new();
    render(&game, mode, &layout, &mut render_state, true);

    println!("Breakout keys: A/D move, P pause, K save, R restore, Q quit");

    let mut last_tick = get_time();
    let mut last_log = get_time();

    while !game.quit {
        let mut dirty = false;

        while let Some(c) = getchar_poll() {
            match c {
                b'a' | b'A' | b'j' | b'J' => {
                    game.move_paddle(&layout, -18);
                    dirty = true;
                }
                b'd' | b'D' | b'l' | b'L' => {
                    game.move_paddle(&layout, 18);
                    dirty = true;
                }
                b'p' | b'P' => {
                    game.paused = !game.paused;
                    dirty = true;
                }
                b'k' | b'K' => {
                    let ok = game.save();
                    println!("breakout save: {}", if ok { "ok" } else { "failed" });
                }
                b'r' | b'R' => {
                    let ok = game.load();
                    println!("breakout restore: {}", if ok { "ok" } else { "failed" });
                    if ok {
                        render(&game, mode, &layout, &mut render_state, true);
                    }
                    dirty = true;
                }
                b'q' | b'Q' => {
                    game.quit = true;
                    dirty = true;
                }
                _ => {}
            }
        }

        let now = get_time();
        if now - last_tick >= TICK_MS {
            let mut steps = ((now - last_tick) / TICK_MS) as usize;
            if steps > 4 {
                steps = 4;
            }
            if steps == 0 {
                steps = 1;
            }
            for _ in 0..steps {
                game.on_tick(&layout);
            }
            last_tick = now;
            dirty = true;
        }

        if dirty {
            render(&game, mode, &layout, &mut render_state, false);
        }

        if now - last_log >= 1000 {
            println!(
                "Breakout score:{} lives:{}{}{}",
                game.score,
                game.lives,
                if game.paused { " [paused]" } else { "" },
                if !game.running { " [finished]" } else { "" }
            );
            last_log = now;
        }

        if !game.running {
            game.quit = true;
        }

        sched_yield();
    }

    render(&game, mode, &layout, &mut render_state, true);
    println!("Breakout end. score:{} lives:{}", game.score, game.lives);
    0
}
