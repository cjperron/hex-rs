use crate::{Agent, HexBoard, HexMove};
use rand::prelude::IndexedRandom;
use std::time::Duration;
use std::thread;

pub struct RandomAgent;

impl RandomAgent {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RandomAgent{
    fn default() -> Self {
        Self
    }
}

impl Agent for RandomAgent {
    fn get_move(&mut self, board: &HexBoard) -> HexMove {
        let mut valid_moves = Vec::new();
        for y in 0..board.height {
            for x in 0..board.width {
                let m = HexMove { x, y };
                if board.is_valid_move(&m) {
                    valid_moves.push(m);
                }
            }
        }
        
        // Un pequeño delay para que no juegue de forma instantánea
        thread::sleep(Duration::from_millis(200));

        let mut rng = rand::rng();
        *valid_moves.choose(&mut rng).unwrap_or(&HexMove { x: usize::MAX, y: usize::MAX })
    }
}
