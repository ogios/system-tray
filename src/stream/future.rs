use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};

use futures::FutureExt;

use crate::event::Event;

use super::{
    handle::to_new_item_event,
    stream_loop::{Item, LoopInner, Token},
    waker::{LoopWaker, WakeFrom, WakerData},
};

#[derive(Debug)]
pub struct LoopEventAddInner {
    pub events: Vec<Event>,
    pub token: Token,
    pub item: Item,
}

#[derive(Debug)]
pub enum LoopEvent {
    Init(Vec<String>),
    Add(Box<LoopEventAddInner>),
    Remove(Event),
    Updata(Event),
}
impl LoopEvent {
    pub fn process_by_loop(self, lp: &mut LoopInner) -> Vec<Event> {
        match self {
            LoopEvent::Add(inner) => {
                let LoopEventAddInner {
                    events,
                    token,
                    item,
                } = *inner;
                lp.wrap_new_item_added(events, token, item)
            }
            LoopEvent::Remove(event) => {
                lp.item_removed(event.destination());
                vec![event]
            }
            LoopEvent::Updata(event) => vec![event],
            LoopEvent::Init(items) => items
                .into_iter()
                .filter_map(|address| {
                    let connection = lp.connection.clone();
                    lp.futures
                        .try_put(
                            async move { to_new_item_event(&address, &connection).await },
                            lp.waker_data.clone().unwrap(),
                        )
                        .map(|le| le.process_by_loop(lp))
                })
                .flatten()
                .collect(),
        }
    }
}

pub struct FutureMap {
    #[allow(clippy::type_complexity)]
    map: Vec<Option<Pin<Box<dyn Future<Output = Option<LoopEvent>>>>>>,
}
impl FutureMap {
    pub fn new() -> Self {
        Self { map: vec![] }
    }
    pub fn preserve_space(&mut self) -> usize {
        self.map
            .iter()
            .position(|f| f.is_none())
            .unwrap_or_else(|| {
                self.map.push(None);
                self.map.len() - 1
            })
    }
    pub fn get(
        &mut self,
        index: usize,
    ) -> &mut Option<Pin<Box<dyn Future<Output = Option<LoopEvent>>>>> {
        &mut self.map[index]
    }
    pub fn put<T>(&mut self, fut: T)
    where
        T: Future<Output = Option<LoopEvent>> + 'static,
    {
        let index = self.preserve_space();
        let fut = Box::pin(fut);
        self.map[index].replace(fut);
    }
    pub fn try_put<T>(&mut self, fut: T, waker_data: Arc<Mutex<WakerData>>) -> Option<LoopEvent>
    where
        T: Future<Output = Option<LoopEvent>> + 'static,
    {
        let index = self.preserve_space();
        let waker = LoopWaker::new_waker(waker_data, WakeFrom::FutureEvent(index));
        let mut fut = Box::pin(fut);
        let res = fut.poll_unpin(&mut std::task::Context::from_waker(&waker));

        if let std::task::Poll::Ready(e) = res {
            e
        } else {
            self.get(index).replace(Box::pin(fut));
            None
        }
    }
    pub fn try_poll(
        &mut self,
        index: usize,
        waker_data: Arc<Mutex<WakerData>>,
    ) -> Option<LoopEvent> {
        let fut_space = self.get(index);
        if let Some(mut fut) = fut_space.take() {
            let waker = LoopWaker::new_waker(waker_data, WakeFrom::FutureEvent(index));
            if let std::task::Poll::Ready(e) =
                fut.poll_unpin(&mut std::task::Context::from_waker(&waker))
            {
                e
            } else {
                fut_space.replace(fut);
                None
            }
        } else {
            None
        }
    }
}

impl Default for FutureMap {
    fn default() -> Self {
        Self::new()
    }
}
