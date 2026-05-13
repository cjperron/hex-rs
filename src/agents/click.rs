use std::sync::{mpsc::Receiver, Arc, Mutex};
use crate::{Agent, HexBoard, HexMove};

pub struct ClickAgent {
    move_receiver: Arc<Mutex<Receiver<HexMove>>>,
}

impl ClickAgent {
    pub fn new(move_receiver: Arc<Mutex<Receiver<HexMove>>>) -> Self {
        Self { move_receiver }
    }
}

impl Agent for ClickAgent {
    fn get_move(&mut self, _board: &HexBoard) -> HexMove {
        self.move_receiver.lock().unwrap().recv().unwrap_or(HexMove { x: usize::MAX, y: usize::MAX })
    }
}
