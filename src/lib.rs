use ratatui::{
    style::Color
};
use std::{
    fs,
    sync::mpsc::Receiver,
};
mod union_find;

use crate::union_find::UnionFind;

const CONFIG_FILE: &str = "hex_config.txt";

// --- Game Definitions ---

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Player {
    Red,
    Blue,
}

impl Player {
    pub fn opposite(&self) -> Self {
        match self {
            Player::Red => Player::Blue,
            Player::Blue => Player::Red,
        }
    }

    pub fn color(&self) -> Color {
        match self {
            Player::Red => Color::Red,
            Player::Blue => Color::Blue,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HexCell {
    Empty,
    Occupied(Player),
}

#[derive(Debug, Clone, Copy)]
pub struct HexMove {
    pub x: usize,
    pub y: usize,
}

impl std::fmt::Display for HexMove {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let col = (b'a' + (self.x as u8)) as char;
        write!(f, "{}{}", col, self.y + 1)
    }
}

#[derive(Debug, Clone)]
pub struct HexBoard {
    pub width: usize,
    pub height: usize,
    pub grid: Vec<Vec<HexCell>>,
    pub current_player: Player,
    pub move_count: usize,
    pub swap_rule: bool,
    pub winner: Option<Player>,
    pub move_history: Vec<HexMove>,
}

impl HexBoard {
    pub fn new(width: usize, height: usize, first_player: Player, swap_rule: bool) -> Self {
        Self {
            width,
            height,
            grid: vec![vec![HexCell::Empty; width]; height],
            current_player: first_player,
            move_count: 0,
            swap_rule,
            winner: None,
            move_history: Vec::new(),
        }
    }

    pub fn is_valid_move(&self, m: &HexMove) -> bool {
        if m.x >= self.width || m.y >= self.height {
            return false;
        }

        if self.move_count == 1 && self.swap_rule {
            return self.grid[m.y][m.x] == HexCell::Empty || self.grid[m.y][m.x] == HexCell::Occupied(self.current_player.opposite());
        }

        self.grid[m.y][m.x] == HexCell::Empty
    }

    pub fn apply_move(&mut self, m: &HexMove) -> Result<(), String> {
        if self.winner.is_some() {
            return Err("Game is already over".to_string());
        }

        if !self.is_valid_move(m) {
            return Err(format!("WARNING: Invalid move: {:?}", m));
        }

        self.grid[m.y][m.x] = HexCell::Occupied(self.current_player);

        self.move_history.push(*m);
        self.move_count += 1;
        self.check_win();
        if self.winner.is_none() {
            self.current_player = self.current_player.opposite();
        }

        Ok(())
    }

    fn check_win(&mut self) {
        let size = self.width * self.height;
        let mut uf = UnionFind::new(size + 4);

        let red_top = size;
        let red_bottom = size + 1;
        let blue_left = size + 2;
        let blue_right = size + 3;

        for y in 0..self.height {
            for x in 0..self.width {
                if let HexCell::Occupied(p) = self.grid[y][x] {
                    let idx = y * self.width + x;

                    if p == Player::Red {
                        if y == 0 { uf.union(idx, red_top); }
                        if y == self.height - 1 { uf.union(idx, red_bottom); }
                    } else {
                        if x == 0 { uf.union(idx, blue_left); }
                        if x == self.width - 1 { uf.union(idx, blue_right); }
                    }

                    let neighbors = [
                        (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1)
                    ];

                    for (dx, dy) in neighbors.iter() {
                        let nx = x as isize + dx;
                        let ny = y as isize + dy;
                        if nx >= 0 && nx < self.width as isize && ny >= 0 && ny < self.height as isize
                            && self.grid[ny as usize][nx as usize] == HexCell::Occupied(p) {
                                let n_idx = (ny as usize) * self.width + (nx as usize);
                                uf.union(idx, n_idx);
                            }
                    }
                }
            }
        }

        if uf.find(red_top) == uf.find(red_bottom) {
            self.winner = Some(Player::Red);
        } else if uf.find(blue_left) == uf.find(blue_right) {
            self.winner = Some(Player::Blue);
        }
    }
}


// --- Agent System ---

pub trait Agent: Send {
    fn get_move(&mut self, board: &HexBoard) -> HexMove;
}

pub struct ClickAgent {
    move_receiver: Receiver<HexMove>,
}

impl ClickAgent {
    pub fn new(move_receiver: Receiver<HexMove>) -> Self {
        Self { move_receiver }
    }
}

impl Agent for ClickAgent {
    fn get_move(&mut self, _board: &HexBoard) -> HexMove {
        self.move_receiver.recv().unwrap_or(HexMove { x: usize::MAX, y: usize::MAX })
    }
}

pub enum GameUpdate {
    State(HexBoard),
    Error(String),
}

pub enum AppState {
    Menu,
    Playing,
}

// --- Configuration & Saving ---


pub struct AppConfig {
    pub width: usize,
    pub height: usize,
    pub swap_rule: bool,
    pub first_player: Player,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            width: 11,
            height: 11,
            swap_rule: true,
            first_player: Player::Red,
        }
    }
}

impl AppConfig {
    pub fn load() -> Self {
        let mut cfg = Self::default();
        if let Ok(contents) = fs::read_to_string(CONFIG_FILE) {
            for line in contents.lines() {
                let parts: Vec<&str> = line.split('=').collect();
                if parts.len() == 2 {
                    match parts[0].trim() {
                        "width" => if let Ok(v) = parts[1].trim().parse() { cfg.width = v; },
                        "height" => if let Ok(v) = parts[1].trim().parse() { cfg.height = v; },
                        "swap_rule" => if let Ok(v) = parts[1].trim().parse() { cfg.swap_rule = v; },
                        "first_player" => {
                            if parts[1].trim() == "Blue" {
                                cfg.first_player = Player::Blue;
                            } else {
                                cfg.first_player = Player::Red;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        cfg
    }

    pub fn save(&self) {
        let content = format!(
            "width={}\nheight={}\nswap_rule={}\nfirst_player={}\n",
            self.width,
            self.height,
            self.swap_rule,
            if self.first_player == Player::Red { "Red" } else { "Blue" }
        );
        let _ = fs::write(CONFIG_FILE, content);
    }
}


