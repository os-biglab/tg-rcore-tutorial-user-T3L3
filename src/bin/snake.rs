#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::{framebuffer_flush, framebuffer_info, get_time, getchar_blocking, getchar_poll, sleep};

const SYSCALL_SET_INPUT_MODE: usize = 0x1000_0003;

const WIDTH: i16 = 24;
const HEIGHT: i16 = 16;
const MAX_SNAKE_LEN: usize = (WIDTH as usize) * (HEIGHT as usize);
const POLL_TICK_MS: usize = 100;
const COLOR_BG: u32 = 0xff121418;
const COLOR_BOARD_BG: u32 = 0xff1e2228;
const COLOR_BORDER: u32 = 0xff3a3f4a;
const COLOR_HEAD: u32 = 0xff4caf50;
const COLOR_BODY: u32 = 0xff7bc67e;
const COLOR_FOOD: u32 = 0xffef5350;
const COLOR_MODE_POLL: u32 = 0xffffc107;
const COLOR_MODE_IRQ: u32 = 0xff42a5f5;
const COLOR_SCORE: u32 = 0xffab47bc;
const COLOR_OVER: u32 = 0xffe53935;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Pos {
    x: i16,
    y: i16,
}

#[derive(Clone, Copy)]
enum Direction {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy)]
enum InputMode {
    Polling,
    InterruptLike,
}

struct Game {
    snake: [Pos; MAX_SNAKE_LEN],
    len: usize,
    dir: Direction,
    food: Pos,
    seed: u32,
    quit: bool,
    over: bool,
}

impl Game {
    fn new(seed: u32) -> Self {
        let mid_x = WIDTH / 2;
        let mid_y = HEIGHT / 2;
        let mut game = Self {
            snake: [Pos { x: 0, y: 0 }; MAX_SNAKE_LEN],
            len: 3,
            dir: Direction::Right,
            food: Pos { x: 0, y: 0 },
            seed,
            quit: false,
            over: false,
        };
        game.snake[0] = Pos { x: mid_x, y: mid_y };
        game.snake[1] = Pos {
            x: mid_x - 1,
            y: mid_y,
        };
        game.snake[2] = Pos {
            x: mid_x - 2,
            y: mid_y,
        };
        game.place_food();
        game
    }

    fn place_food(&mut self) {
        loop {
            let x = (self.next_rand() % (WIDTH as u32)) as i16;
            let y = (self.next_rand() % (HEIGHT as u32)) as i16;
            let p = Pos { x, y };
            if !self.contains(p) {
                self.food = p;
                break;
            }
        }
    }

    fn next_rand(&mut self) -> u32 {
        self.seed = self.seed.wrapping_mul(1664525).wrapping_add(1013904223);
        self.seed
    }

    fn contains(&self, p: Pos) -> bool {
        let mut i = 0;
        while i < self.len {
            if self.snake[i] == p {
                return true;
            }
            i += 1;
        }
        false
    }

    fn set_direction(&mut self, next: Direction) {
        if matches!(
            (self.dir, next),
            (Direction::Up, Direction::Down)
                | (Direction::Down, Direction::Up)
                | (Direction::Left, Direction::Right)
                | (Direction::Right, Direction::Left)
        ) {
            return;
        }
        self.dir = next;
    }

    fn step(&mut self) {
        if self.over || self.quit {
            return;
        }

        let head = self.snake[0];
        let next = match self.dir {
            Direction::Up => Pos {
                x: head.x,
                y: head.y - 1,
            },
            Direction::Down => Pos {
                x: head.x,
                y: head.y + 1,
            },
            Direction::Left => Pos {
                x: head.x - 1,
                y: head.y,
            },
            Direction::Right => Pos {
                x: head.x + 1,
                y: head.y,
            },
        };

        if next.x < 0 || next.x >= WIDTH || next.y < 0 || next.y >= HEIGHT {
            self.over = true;
            return;
        }

        if self.contains(next) {
            self.over = true;
            return;
        }

        let grow = next == self.food;

        let mut i = if grow { self.len } else { self.len - 1 };
        while i > 0 {
            self.snake[i] = self.snake[i - 1];
            i -= 1;
        }
        self.snake[0] = next;

        if grow {
            if self.len < MAX_SNAKE_LEN - 1 {
                self.len += 1;
                self.place_food();
            } else {
                self.over = true;
            }
        }
    }

    fn score(&self) -> usize {
        self.len.saturating_sub(3)
    }
}

fn select_mode() -> InputMode {
    println!("Snake mode: [p] polling / [i] interrupt");

    loop {
        match getchar_blocking() {
            b'p' | b'P' => return InputMode::Polling,
            b'i' | b'I' => return InputMode::InterruptLike,
            _ => {}
        }
    }
}

fn handle_key(game: &mut Game, key: u8) {
    match key {
        b'w' | b'W' => game.set_direction(Direction::Up),
        b's' | b'S' => game.set_direction(Direction::Down),
        b'a' | b'A' => game.set_direction(Direction::Left),
        b'd' | b'D' => game.set_direction(Direction::Right),
        b'q' | b'Q' => game.quit = true,
        _ => {}
    }
}

fn cell(game: &Game, x: i16, y: i16) -> u8 {
    let p = Pos { x, y };
    if game.snake[0] == p {
        return 1;
    }
    let mut i = 1;
    while i < game.len {
        if game.snake[i] == p {
            return 2;
        }
        i += 1;
    }
    if game.food == p { 3 } else { 0 }
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
    let mode_id = match mode {
        InputMode::Polling => 0,
        InputMode::InterruptLike => 1,
    };
    custom_syscall1(SYSCALL_SET_INPUT_MODE, mode_id)
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

struct RenderState {
    initialized: bool,
    prev_cells: [u8; MAX_SNAKE_LEN],
    prev_mode: u8,
    prev_score_bar: usize,
    prev_over: bool,
}

impl RenderState {
    fn new() -> Self {
        Self {
            initialized: false,
            prev_cells: [0; MAX_SNAKE_LEN],
            prev_mode: 0,
            prev_score_bar: 0,
            prev_over: false,
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

fn render_gui(game: &Game, mode: InputMode, state: &mut RenderState) {
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
    let framebuffer = unsafe { core::slice::from_raw_parts_mut(fb_ptr, used_len) };

    let cell_px = core::cmp::max(
        8,
        core::cmp::min(width / (WIDTH as usize + 4), height / (HEIGHT as usize + 6)),
    );
    let board_w = WIDTH as usize * cell_px;
    let board_h = HEIGHT as usize * cell_px;
    let origin_x = (width.saturating_sub(board_w)) / 2;
    let origin_y = (height.saturating_sub(board_h)) / 2;

    if !state.initialized {
        fill_rect(framebuffer, width, 0, 0, width, height, COLOR_BG);
        fill_rect(
            framebuffer,
            width,
            origin_x.saturating_sub(4),
            origin_y.saturating_sub(4),
            board_w + 8,
            board_h + 8,
            COLOR_BORDER,
        );
        fill_rect(
            framebuffer,
            width,
            origin_x,
            origin_y,
            board_w,
            board_h,
            COLOR_BOARD_BG,
        );
    }

    let mut cur_cells = [0u8; MAX_SNAKE_LEN];
    let mut y = 0;
    while y < HEIGHT {
        let mut x = 0;
        while x < WIDTH {
            let idx = y as usize * WIDTH as usize + x as usize;
            let cur = cell(game, x, y);
            cur_cells[idx] = cur;
            if !state.initialized || state.prev_cells[idx] != cur {
                let color = match cur {
                    1 => COLOR_HEAD,
                    2 => COLOR_BODY,
                    3 => COLOR_FOOD,
                    _ => COLOR_BOARD_BG,
                };
                fill_rect(
                    framebuffer,
                    width,
                    origin_x + x as usize * cell_px,
                    origin_y + y as usize * cell_px,
                    cell_px.saturating_sub(1),
                    cell_px.saturating_sub(1),
                    color,
                );
            }
            x += 1;
        }
        y += 1;
    }
    state.prev_cells.copy_from_slice(&cur_cells);

    let cur_mode = mode_id(mode);
    if !state.initialized || state.prev_mode != cur_mode {
        fill_rect(
            framebuffer,
            width,
            origin_x,
            origin_y.saturating_sub(24),
            board_w,
            12,
            mode_color(mode),
        );
    }
    state.prev_mode = cur_mode;

    let score_bar = core::cmp::min(board_w, game.score().saturating_mul(cell_px / 2 + 1));
    if !state.initialized {
        if score_bar > 0 {
            fill_rect(
                framebuffer,
                width,
                origin_x,
                origin_y + board_h + 12,
                score_bar,
                10,
                COLOR_SCORE,
            );
        }
    } else if score_bar > state.prev_score_bar {
        fill_rect(
            framebuffer,
            width,
            origin_x + state.prev_score_bar,
            origin_y + board_h + 12,
            score_bar - state.prev_score_bar,
            10,
            COLOR_SCORE,
        );
    } else if score_bar < state.prev_score_bar {
        fill_rect(
            framebuffer,
            width,
            origin_x + score_bar,
            origin_y + board_h + 12,
            state.prev_score_bar - score_bar,
            10,
            COLOR_BG,
        );
    }
    state.prev_score_bar = score_bar;

    let cur_over = game.over || game.quit;
    if cur_over {
        fill_rect(
            framebuffer,
            width,
            origin_x,
            origin_y + board_h / 2 - 6,
            board_w,
            12,
            COLOR_OVER,
        );
    } else if state.prev_over {
        fill_rect(
            framebuffer,
            width,
            origin_x,
            origin_y + board_h / 2 - 6,
            board_w,
            12,
            COLOR_BOARD_BG,
        );
    }
    state.prev_over = cur_over;
    state.initialized = true;

    let _ = framebuffer_flush();
}

#[unsafe(no_mangle)]
extern "C" fn main() -> i32 {
    let seed = (get_time().max(1) as u32) ^ 0x5a5a_a5a5;
    let mut game = Game::new(seed);
    let mut render_state = RenderState::new();

    // 先渲染首帧，避免在选择输入模式前 GUI 窗口为空。
    render_gui(&game, InputMode::Polling, &mut render_state);

    let mode = select_mode();
    let mode_id = mode_id(mode);
    println!("Selected mode: {}", if mode_id == 0 { "Polling" } else { "Interrupt-like" });
    if set_input_mode(mode) != 0 {
        println!("set_input_mode failed");
        return -1;
    }

    render_gui(&game, mode, &mut render_state);
    while !game.over && !game.quit {
        while let Some(c) = getchar_poll() {
            handle_key(&mut game, c);
        }
        game.step();
        render_gui(&game, mode, &mut render_state);
        sleep(POLL_TICK_MS);
    }

    render_gui(&game, mode, &mut render_state);
    println!("Snake end. score={}", game.score());
    0
}
