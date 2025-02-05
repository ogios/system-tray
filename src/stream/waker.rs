use std::{
    sync::{Arc, Mutex},
    task::Waker,
};

use cooked_waker::{IntoWaker, WakeRef};

use super::stream_loop::Token;

/// This represents the wake source of an item.
#[derive(Debug, Clone)]
pub enum ItemWakeFrom {
    Disconnect,
    PropertyChange,
    LayoutUpdate,
}

/// This represents the wake source of the loop.
#[derive(Debug, Clone)]
pub enum WakeFrom {
    NewItem,
    FutureEvent(usize),
    ItemUpdate {
        token: Token,
        item_wake_from: ItemWakeFrom,
    },
}

#[derive(Debug)]
pub struct WakerData {
    pub ready_tokens: Vec<WakeFrom>,
    pub root_waker: Waker,
}

/// This is the core.
/// It will be distributed to all the streams,
/// They all share one common WakerData.
///
/// Once the waker is called, it will record which source it is been called from,
/// into the `ready_tokens`.
///
/// And the next time the loop is polled,
/// it will drain the `ready_tokens` and directly poll the matching stream.
#[derive(Debug, Clone)]
pub struct LoopWaker {
    pub waker_data: Arc<Mutex<WakerData>>,
    pub wake_from: WakeFrom,
}
impl LoopWaker {
    pub(crate) fn new_waker(waker_data: Arc<Mutex<WakerData>>, wake_from: WakeFrom) -> Waker {
        Arc::new(Self {
            waker_data,
            wake_from,
        })
        .into_waker()
    }
}

impl WakeRef for LoopWaker {
    fn wake_by_ref(&self) {
        println!("wake by ref: {:?}", self.wake_from);
        let mut data = self.waker_data.lock().unwrap();
        data.ready_tokens.push(self.wake_from.clone());
        data.root_waker.wake_by_ref();
    }
}
