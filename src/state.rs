use crate::tasks::{DbTask, UiTask};
use crossbeam::channel::{Receiver, Sender};

pub struct AppState {
    pub db_tx: Sender<DbTask>,
    pub ui_rx: Receiver<UiTask>,
}

impl AppState {
    pub fn new(db_tx: Sender<DbTask>, ui_rx: Receiver<UiTask>) -> Self {
        Self { db_tx, ui_rx }
    }
}
