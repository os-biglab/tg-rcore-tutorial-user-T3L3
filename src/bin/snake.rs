#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

use user_lib::{get_time, getchar_blocking, getchar_poll, sleep};

const WIDTH: i16 = 24;
const HEIGHT: i16 = 16;
const MAX_SNAKE_LEN: usize = (WIDTH as usize) * (HEIGHT as usize);
const POLL_TICK_MS: usize = 130;

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
    println!("==============================");
    println!("      ch3-T3L3 Snake Game     ");
    println!("==============================");
    println!("Choose control mode:");
    println!("  [p] polling input");
    println!("  [i] interrupt-style input");
    println!("Press p or i to start...");

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
        return b'@';
    }
    let mut i = 1;
    while i < game.len {
        if game.snake[i] == p {
            return b'o';
        }
        i += 1;
    }
    if game.food == p {
        b'*'
    } else {
        b' '
    }
}

fn render(game: &Game, mode: InputMode) {
    print!("\x1b[2J\x1b[H");
    println!("Snake(ch3-T3L3)  score={}  q=quit", game.score());
    match mode {
        InputMode::Polling => println!("mode: polling input"),
        InputMode::InterruptLike => println!("mode: interrupt-style input"),
    }
    println!("control: W/A/S/D");

    let mut x = 0;
    while x < WIDTH + 2 {
        print!("#");
        x += 1;
    }
    println!("");

    let mut y = 0;
    while y < HEIGHT {
        print!("#");
        let mut x = 0;
        while x < WIDTH {
            print!("{}", cell(game, x, y) as char);
            x += 1;
        }
        println!("#");
        y += 1;
    }

    let mut x = 0;
    while x < WIDTH + 2 {
        print!("#");
        x += 1;
    }
    println!("");
}

fn run_polling(game: &mut Game) {
    while !game.over && !game.quit {
        while let Some(c) = getchar_poll() {
            handle_key(game, c);
        }
        game.step();
        render(game, InputMode::Polling);
        sleep(POLL_TICK_MS);
    }
}

fn run_interrupt_like(game: &mut Game) {
    while !game.over && !game.quit {
        let c = getchar_blocking();
        handle_key(game, c);
        game.step();
        render(game, InputMode::InterruptLike);
    }
}

#[unsafe(no_mangle)]
extern "C" fn main() -> i32 {
    let mode = select_mode();
    let seed = (get_time().max(1) as u32) ^ 0x5a5a_a5a5;
    let mut game = Game::new(seed);

    render(&game, mode);
    match mode {
        InputMode::Polling => run_polling(&mut game),
        InputMode::InterruptLike => run_interrupt_like(&mut game),
    }

    if game.quit {
        println!("Snake quit. score={}", game.score());
    } else {
        println!("Game over! score={}", game.score());
    }
    0
}
