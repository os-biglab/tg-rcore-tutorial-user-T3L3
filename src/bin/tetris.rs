#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::{framebuffer_flush, framebuffer_info, get_time, getchar_blocking, getchar_poll, sleep};

const SYSCALL_SET_INPUT_MODE: usize = 0x1000_0003;

const BOARD_W: i16 = 10;
const BOARD_H: i16 = 20;
const BOARD_CELLS: usize = (BOARD_W as usize) * (BOARD_H as usize);
const POLL_TICK_MS: usize = 16;

const COLOR_BG: u32 = 0xff11151a;
const COLOR_BOARD_BG: u32 = 0xff1b2026;
const COLOR_BORDER: u32 = 0xff3a424d;
const COLOR_OVERLAY: u32 = 0xffd32f2f;
const COLOR_MODE_POLL: u32 = 0xffffc107;
const COLOR_MODE_IRQ: u32 = 0xff42a5f5;
const COLOR_SCORE: u32 = 0xffab47bc;
const COLOR_LEVEL: u32 = 0xff66bb6a;

const CELL_COLORS: [u32; 8] = [
    COLOR_BOARD_BG,
    0xff26c6da,
    0xff42a5f5,
    0xffffca28,
    0xffab47bc,
    0xff66bb6a,
    0xffef5350,
    0xffff7043,
];

#[derive(Clone, Copy)]
enum InputMode {
    Polling,
    InterruptLike,
}

#[derive(Clone, Copy)]
struct Piece {
    kind: u8,
    rot: u8,
    x: i16,
    y: i16,
}

#[derive(Clone, Copy)]
struct Block {
    x: i16,
    y: i16,
}

#[derive(Clone, Copy)]
struct Layout {
    width: usize,
    height: usize,
    cell_px: usize,
    board_w_px: usize,
    board_h_px: usize,
    origin_x: usize,
    origin_y: usize,
}

struct Game {
    board: [u8; BOARD_CELLS],
    active: Piece,
    next_kind: u8,
    rng: u32,
    pieces: usize,
    score: usize,
    lines: usize,
    level: usize,
    quit: bool,
    over: bool,
}

struct RenderState {
    initialized: bool,
    prev_cells: [u8; BOARD_CELLS],
    prev_mode: u8,
    prev_score: usize,
    prev_level: usize,
    prev_over: bool,
}

impl RenderState {
    fn new() -> Self {
        Self {
            initialized: false,
            prev_cells: [0; BOARD_CELLS],
            prev_mode: 0,
            prev_score: 0,
            prev_level: 0,
            prev_over: false,
        }
    }
}

const SHAPES: [[[[i16; 2]; 4]; 4]; 7] = [
    [
        [[0, 1], [1, 1], [2, 1], [3, 1]],
        [[2, 0], [2, 1], [2, 2], [2, 3]],
        [[0, 2], [1, 2], [2, 2], [3, 2]],
        [[1, 0], [1, 1], [1, 2], [1, 3]],
    ],
    [
        [[0, 0], [0, 1], [1, 1], [2, 1]],
        [[1, 0], [2, 0], [1, 1], [1, 2]],
        [[0, 1], [1, 1], [2, 1], [2, 2]],
        [[1, 0], [1, 1], [0, 2], [1, 2]],
    ],
    [
        [[1, 0], [2, 0], [1, 1], [2, 1]],
        [[1, 0], [2, 0], [1, 1], [2, 1]],
        [[1, 0], [2, 0], [1, 1], [2, 1]],
        [[1, 0], [2, 0], [1, 1], [2, 1]],
    ],
    [
        [[1, 0], [0, 1], [1, 1], [2, 1]],
        [[1, 0], [1, 1], [2, 1], [1, 2]],
        [[0, 1], [1, 1], [2, 1], [1, 2]],
        [[1, 0], [0, 1], [1, 1], [1, 2]],
    ],
    [
        [[1, 0], [2, 0], [0, 1], [1, 1]],
        [[1, 0], [1, 1], [2, 1], [2, 2]],
        [[1, 1], [2, 1], [0, 2], [1, 2]],
        [[0, 0], [0, 1], [1, 1], [1, 2]],
    ],
    [
        [[0, 0], [1, 0], [1, 1], [2, 1]],
        [[2, 0], [1, 1], [2, 1], [1, 2]],
        [[0, 1], [1, 1], [1, 2], [2, 2]],
        [[1, 0], [0, 1], [1, 1], [0, 2]],
    ],
    [
        [[2, 0], [0, 1], [1, 1], [2, 1]],
        [[1, 0], [1, 1], [1, 2], [2, 2]],
        [[0, 1], [1, 1], [2, 1], [0, 2]],
        [[0, 0], [1, 0], [1, 1], [1, 2]],
    ],
];

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

#[inline]
fn board_idx(x: i16, y: i16) -> usize {
    y as usize * BOARD_W as usize + x as usize
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

fn block_at(piece: &Piece, i: usize) -> Block {
    let rel = SHAPES[piece.kind as usize][piece.rot as usize][i];
    Block {
        x: piece.x + rel[0],
        y: piece.y + rel[1],
    }
}

fn is_collide(board: &[u8; BOARD_CELLS], piece: &Piece) -> bool {
    let mut i = 0;
    while i < 4 {
        let b = block_at(piece, i);
        if b.x < 0 || b.x >= BOARD_W || b.y < 0 || b.y >= BOARD_H {
            return true;
        }
        if board[board_idx(b.x, b.y)] != 0 {
            return true;
        }
        i += 1;
    }
    false
}

fn lock_piece(board: &mut [u8; BOARD_CELLS], piece: &Piece) {
    let cell = piece.kind + 1;
    let mut i = 0;
    while i < 4 {
        let b = block_at(piece, i);
        if b.x >= 0 && b.x < BOARD_W && b.y >= 0 && b.y < BOARD_H {
            board[board_idx(b.x, b.y)] = cell;
        }
        i += 1;
    }
}

fn clear_lines(board: &mut [u8; BOARD_CELLS]) -> usize {
    let mut dst = BOARD_H - 1;
    let mut cleared = 0usize;

    let mut src = BOARD_H - 1;
    loop {
        let mut full = true;
        let mut x = 0;
        while x < BOARD_W {
            if board[board_idx(x, src)] == 0 {
                full = false;
                break;
            }
            x += 1;
        }

        if full {
            cleared += 1;
        } else {
            let mut copy_x = 0;
            while copy_x < BOARD_W {
                board[board_idx(copy_x, dst)] = board[board_idx(copy_x, src)];
                copy_x += 1;
            }
            dst -= 1;
        }

        if src == 0 {
            break;
        }
        src -= 1;
    }

    while dst >= 0 {
        let mut x = 0;
        while x < BOARD_W {
            board[board_idx(x, dst)] = 0;
            x += 1;
        }
        dst -= 1;
    }

    cleared
}

fn line_score(cleared: usize, level: usize) -> usize {
    let base = match cleared {
        1 => 100,
        2 => 300,
        3 => 500,
        4 => 800,
        _ => 0,
    };
    base * (level + 1)
}

fn calc_level(lines: usize, pieces: usize) -> usize {
    let by_lines = lines / 10;
    let by_pieces = pieces / 18;
    by_lines + by_pieces
}

fn drop_interval_ms(level: usize) -> usize {
    let base = 700usize;
    let dec = core::cmp::min(level * 55, 620);
    core::cmp::max(80, base.saturating_sub(dec))
}

impl Game {
    fn new(seed: u32) -> Self {
        let mut g = Self {
            board: [0; BOARD_CELLS],
            active: Piece {
                kind: 0,
                rot: 0,
                x: 3,
                y: 0,
            },
            next_kind: 0,
            rng: seed,
            pieces: 0,
            score: 0,
            lines: 0,
            level: 0,
            quit: false,
            over: false,
        };
        g.next_kind = g.rand_kind();
        g.spawn_piece();
        g
    }

    fn next_rand(&mut self) -> u32 {
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
        self.rng
    }

    fn rand_kind(&mut self) -> u8 {
        (self.next_rand() % 7) as u8
    }

    fn spawn_piece(&mut self) {
        self.active = Piece {
            kind: self.next_kind,
            rot: 0,
            x: 3,
            y: 0,
        };
        self.next_kind = self.rand_kind();
        if is_collide(&self.board, &self.active) {
            self.over = true;
        }
    }

    fn try_move(&mut self, dx: i16, dy: i16) -> bool {
        let mut next = self.active;
        next.x += dx;
        next.y += dy;
        if is_collide(&self.board, &next) {
            return false;
        }
        self.active = next;
        true
    }

    fn try_rotate(&mut self) {
        let mut next = self.active;
        next.rot = (next.rot + 1) % 4;
        if !is_collide(&self.board, &next) {
            self.active = next;
            return;
        }

        const KICKS: [i16; 4] = [-1, 1, -2, 2];
        let mut i = 0;
        while i < KICKS.len() {
            let mut kicked = next;
            kicked.x += KICKS[i];
            if !is_collide(&self.board, &kicked) {
                self.active = kicked;
                return;
            }
            i += 1;
        }
    }

    fn lock_and_advance(&mut self) {
        let prev_level = self.level;
        lock_piece(&mut self.board, &self.active);
        self.pieces = self.pieces.saturating_add(1);
        let cleared = clear_lines(&mut self.board);
        if cleared > 0 {
            self.score = self.score.saturating_add(line_score(cleared, self.level));
            self.lines = self.lines.saturating_add(cleared);
        }
        self.level = calc_level(self.lines, self.pieces);

        println!(
            "Placed={}, Cleared+{}, Lines={}, Score={}, Level={}, Drop={}ms",
            self.pieces,
            cleared,
            self.lines,
            self.score,
            self.level,
            drop_interval_ms(self.level)
        );
        if self.level > prev_level {
            println!("Level up: {} -> {}", prev_level, self.level);
        }

        self.spawn_piece();
    }

    fn hard_drop(&mut self) {
        if self.over || self.quit {
            return;
        }
        while self.try_move(0, 1) {}
        self.lock_and_advance();
    }

    fn tick_drop(&mut self) {
        if self.over || self.quit {
            return;
        }
        if !self.try_move(0, 1) {
            self.lock_and_advance();
        }
    }
}

fn select_mode() -> InputMode {
    println!("Tetris mode: [p] polling / [i] interrupt");
    loop {
        match getchar_blocking() {
            b'p' | b'P' => return InputMode::Polling,
            b'i' | b'I' => return InputMode::InterruptLike,
            _ => {}
        }
    }
}

fn handle_key(game: &mut Game, key: u8) -> bool {
    if game.over || game.quit {
        if key == b'q' || key == b'Q' {
            game.quit = true;
            return true;
        }
        return false;
    }

    match key {
        b'a' | b'A' => game.try_move(-1, 0),
        b'd' | b'D' => game.try_move(1, 0),
        b's' | b'S' => game.try_move(0, 1),
        b'w' | b'W' => {
            game.try_rotate();
            true
        }
        b' ' => {
            game.hard_drop();
            true
        }
        b'q' | b'Q' => {
            game.quit = true;
            true
        }
        _ => false,
    }
}

fn build_layout(width: usize, height: usize) -> Layout {
    let cell_px = core::cmp::max(
        8,
        core::cmp::min(width / (BOARD_W as usize + 8), height / (BOARD_H as usize + 8)),
    );
    let board_w_px = BOARD_W as usize * cell_px;
    let board_h_px = BOARD_H as usize * cell_px;
    let origin_x = (width.saturating_sub(board_w_px)) / 2;
    let origin_y = (height.saturating_sub(board_h_px)) / 2;
    Layout {
        width,
        height,
        cell_px,
        board_w_px,
        board_h_px,
        origin_x,
        origin_y,
    }
}

fn compose_cells(game: &Game, out: &mut [u8; BOARD_CELLS]) {
    out.copy_from_slice(&game.board);
    if game.over {
        return;
    }
    let mut i = 0;
    while i < 4 {
        let b = block_at(&game.active, i);
        if b.x >= 0 && b.x < BOARD_W && b.y >= 0 && b.y < BOARD_H {
            out[board_idx(b.x, b.y)] = game.active.kind + 1;
        }
        i += 1;
    }
}

fn draw_cell(framebuffer: &mut [u8], layout: &Layout, x: i16, y: i16, value: u8) {
    let color = CELL_COLORS[value as usize];
    fill_rect(
        framebuffer,
        layout.width,
        layout.origin_x + x as usize * layout.cell_px,
        layout.origin_y + y as usize * layout.cell_px,
        layout.cell_px.saturating_sub(1),
        layout.cell_px.saturating_sub(1),
        color,
    );
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

    let layout = build_layout(width, height);
    let framebuffer = unsafe { core::slice::from_raw_parts_mut(fb_ptr, used_len) };

    if !state.initialized {
        fill_rect(framebuffer, layout.width, 0, 0, layout.width, layout.height, COLOR_BG);
        fill_rect(
            framebuffer,
            layout.width,
            layout.origin_x.saturating_sub(4),
            layout.origin_y.saturating_sub(4),
            layout.board_w_px + 8,
            layout.board_h_px + 8,
            COLOR_BORDER,
        );
        fill_rect(
            framebuffer,
            layout.width,
            layout.origin_x,
            layout.origin_y,
            layout.board_w_px,
            layout.board_h_px,
            COLOR_BOARD_BG,
        );
    }

    let mut cur_cells = [0u8; BOARD_CELLS];
    compose_cells(game, &mut cur_cells);

    let mut y = 0;
    while y < BOARD_H {
        let mut x = 0;
        while x < BOARD_W {
            let idx = board_idx(x, y);
            let cur = cur_cells[idx];
            if !state.initialized || state.prev_cells[idx] != cur {
                draw_cell(framebuffer, &layout, x, y, cur);
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
            layout.width,
            layout.origin_x,
            layout.origin_y.saturating_sub(24),
            layout.board_w_px,
            10,
            mode_color(mode),
        );
    }
    state.prev_mode = cur_mode;

    let score_bar = core::cmp::min(layout.board_w_px, game.score / 20);
    let prev_score_bar = core::cmp::min(layout.board_w_px, state.prev_score / 20);
    if !state.initialized {
        if score_bar > 0 {
            fill_rect(
                framebuffer,
                layout.width,
                layout.origin_x,
                layout.origin_y + layout.board_h_px + 12,
                score_bar,
                8,
                COLOR_SCORE,
            );
        }
    } else if score_bar > prev_score_bar {
        fill_rect(
            framebuffer,
            layout.width,
            layout.origin_x + prev_score_bar,
            layout.origin_y + layout.board_h_px + 12,
            score_bar - prev_score_bar,
            8,
            COLOR_SCORE,
        );
    }
    state.prev_score = game.score;

    let level_bar = core::cmp::min(layout.board_w_px, game.level.saturating_mul(layout.cell_px));
    let prev_level_bar = core::cmp::min(layout.board_w_px, state.prev_level.saturating_mul(layout.cell_px));
    if !state.initialized {
        if level_bar > 0 {
            fill_rect(
                framebuffer,
                layout.width,
                layout.origin_x,
                layout.origin_y + layout.board_h_px + 24,
                level_bar,
                8,
                COLOR_LEVEL,
            );
        }
    } else if level_bar > prev_level_bar {
        fill_rect(
            framebuffer,
            layout.width,
            layout.origin_x + prev_level_bar,
            layout.origin_y + layout.board_h_px + 24,
            level_bar - prev_level_bar,
            8,
            COLOR_LEVEL,
        );
    }
    state.prev_level = game.level;

    let cur_over = game.over || game.quit;
    if cur_over {
        fill_rect(
            framebuffer,
            layout.width,
            layout.origin_x,
            layout.origin_y + layout.board_h_px / 2 - 6,
            layout.board_w_px,
            12,
            COLOR_OVERLAY,
        );
    } else if state.prev_over {
        fill_rect(
            framebuffer,
            layout.width,
            layout.origin_x,
            layout.origin_y + layout.board_h_px / 2 - 6,
            layout.board_w_px,
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
    let seed = (get_time().max(1) as u32) ^ 0x4d43_5a19;
    let mut game = Game::new(seed);
    let mut render_state = RenderState::new();

    render_gui(&game, InputMode::Polling, &mut render_state);

    let mode = select_mode();
    println!("Selected mode: {}", if mode_id(mode) == 0 { "Polling" } else { "Interrupt-like" });
    if set_input_mode(mode) != 0 {
        println!("set_input_mode failed");
        return -1;
    }

    let mut last_drop_ms = get_time();
    println!(
        "Start speed: level={}, drop={}ms",
        game.level,
        drop_interval_ms(game.level)
    );
    render_gui(&game, mode, &mut render_state);

    while !game.quit {
        let mut dirty = false;

        while let Some(c) = getchar_poll() {
            if handle_key(&mut game, c) {
                dirty = true;
            }
        }

        if game.over {
            render_gui(&game, mode, &mut render_state);
            break;
        }

        let now = get_time();
        if now - last_drop_ms >= drop_interval_ms(game.level) as isize {
            game.tick_drop();
            last_drop_ms = now;
            dirty = true;
        }

        if dirty {
            render_gui(&game, mode, &mut render_state);
        }

        sleep(POLL_TICK_MS);
    }

    render_gui(&game, mode, &mut render_state);
    println!("Tetris end. score={}, lines={}, level={}", game.score, game.lines, game.level);
    0
}
