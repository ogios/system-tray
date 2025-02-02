use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::Waker,
};

use cooked_waker::{IntoWaker, WakeRef};
use futures::{FutureExt, Stream, StreamExt};
use zbus::{fdo::NameOwnerChangedStream, proxy::SignalStream, Connection};

use crate::{
    client::Event,
    dbus::{
        dbus_menu_proxy::LayoutUpdatedStream,
        notifier_watcher_proxy::StatusNotifierItemRegisteredStream,
    },
    error::Result,
    handle::{to_layout_update_event, to_update_item_event},
};

/// Token is used to identify an item.
/// destination example: ":1.52"
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Token {
    destination: Arc<String>,
}
impl Token {
    pub(crate) fn new(destination: String) -> Self {
        Self {
            destination: Arc::new(destination),
        }
    }
}

/// This represents the wake source of an item.
#[derive(Debug, Clone)]
pub(crate) enum ItemWakeFrom {
    Disconnect,
    PropertyChange,
    LayoutUpdate,
}

/// This represents the wake source of the loop.
#[derive(Debug, Clone)]
pub(crate) enum WakeFrom {
    NewItem,
    FutureEvent(usize),
    ItemUpdate {
        token: Token,
        item_wake_from: ItemWakeFrom,
    },
}

#[derive(Debug)]
pub(crate) struct WakerData {
    ready_tokens: Vec<WakeFrom>,
    root_waker: Waker,
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
pub(crate) struct LoopWaker {
    waker_data: Arc<Mutex<WakerData>>,
    wake_from: WakeFrom,
}
impl LoopWaker {
    pub(crate) fn new(waker_data: Arc<Mutex<WakerData>>, wake_from: WakeFrom) -> Waker {
        Arc::new(Self {
            waker_data,
            wake_from,
        })
        .into_waker()
    }
}

impl WakeRef for LoopWaker {
    fn wake_by_ref(&self) {
        let mut data = self.waker_data.lock().unwrap();
        data.ready_tokens.push(self.wake_from.clone());
        data.root_waker.wake_by_ref();
    }
}

pub(crate) struct Item {
    disconnect_stream: NameOwnerChangedStream,
    property_change_stream: SignalStream<'static>,
    layout_updated_stream: LayoutUpdatedStream,
    // NOTE: dbus_menu_proxy.receive_items_properties_updated is not added,
    // as i find it useless or has bugs, everything it provides is None except for `visible` been
    // removed
}
impl Item {
    fn poll_disconnect(
        &mut self,
        token: Token,
        waker_data: Arc<Mutex<WakerData>>,
    ) -> std::task::Poll<Option<zbus::fdo::NameOwnerChanged>> {
        let waker = Arc::new(LoopWaker {
            waker_data: waker_data.clone(),
            wake_from: WakeFrom::ItemUpdate {
                token,
                item_wake_from: ItemWakeFrom::Disconnect,
            },
        })
        .into_waker();
        let mut cx = std::task::Context::from_waker(&waker);

        self.disconnect_stream.poll_next_unpin(&mut cx)
    }
    fn poll_property_change(
        &mut self,
        token: Token,
        waker_data: Arc<Mutex<WakerData>>,
    ) -> std::task::Poll<Option<Event>> {
        let waker = Arc::new(LoopWaker {
            waker_data: waker_data.clone(),
            wake_from: WakeFrom::ItemUpdate {
                token,
                item_wake_from: ItemWakeFrom::PropertyChange,
            },
        })
        .into_waker();
        let mut cx = std::task::Context::from_waker(&waker);

        self.property_change_stream
            .poll_next_unpin(&mut cx)
            .map(|m| m.map(to_update_item_event))
    }
    fn poll_layout_change(
        &mut self,
        token: Token,
        waker_data: Arc<Mutex<WakerData>>,
    ) -> std::task::Poll<Option<Event>> {
        let waker = Arc::new(LoopWaker {
            waker_data: waker_data.clone(),
            wake_from: WakeFrom::ItemUpdate {
                token,
                item_wake_from: ItemWakeFrom::LayoutUpdate,
            },
        })
        .into_waker();
        let mut cx = std::task::Context::from_waker(&waker);

        self.layout_updated_stream
            .poll_next_unpin(&mut cx)
            .map(|up| up.map(to_layout_update_event))
    }
}

pub(crate) struct FutureMap {
    map: Vec<Option<Pin<Box<dyn Future<Output = Option<Event>>>>>>,
}
impl FutureMap {
    pub(crate) fn preserve_space(&mut self) -> usize {
        self.map
            .iter()
            .position(|f| f.is_none())
            .unwrap_or_else(|| {
                self.map.push(None);
                self.map.len() - 1
            })
    }
    pub(crate) fn get(
        &mut self,
        index: usize,
    ) -> &mut Option<Pin<Box<dyn Future<Output = Option<Event>>>>> {
        &mut self.map[index]
    }
}

pub struct LoopInner {
    pub(crate) waker_data: Option<Arc<Mutex<WakerData>>>,
    pub(crate) ternimated: bool,
    pub(crate) polled: bool,
    pub(crate) futures: FutureMap,

    pub(crate) connection: Connection,
    pub(crate) watcher_stream_register_notifier_item_registered: StatusNotifierItemRegisteredStream,
    pub(crate) items: HashMap<Token, Item>,
    // NOTE: dbus_proxy.receive_name_acquired will not be added currently,
    // cosmic applet didn't do it.
}

impl Stream for LoopInner {
    type Item = Vec<Event>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let ready_events = if !self.polled {
            // first time poll

            self.polled = true;
            let waker_data = Arc::new(Mutex::new(WakerData {
                ready_tokens: Vec::new(),
                root_waker: cx.waker().clone(),
            }));
            self.waker_data = Some(waker_data.clone());

            let mut polls = vec![];

            self.items
                .iter_mut()
                .map(|(token, item)| {
                    (
                        token.clone(),
                        item.poll_disconnect(token.clone(), waker_data.clone()),
                        item.poll_property_change(token.clone(), waker_data.clone()),
                        item.poll_layout_change(token.clone(), waker_data.clone()),
                    )
                })
                .collect::<Vec<_>>()
                .into_iter()
                .for_each(|(token, dis, prop, layout)| {
                    if let std::task::Poll::Ready(Some(ev)) =
                        dis.map(|d| d.map(|d| self.handle_remove_item(&token, d)))
                    {
                        polls.push(ev);
                    };

                    if let std::task::Poll::Ready(Some(ev)) = prop {
                        polls.push(ev);
                    }

                    if let std::task::Poll::Ready(Some(ev)) = layout {
                        polls.push(ev);
                    }
                });

            if let std::task::Poll::Ready(Some(ev)) = self.poll_item_stream() {
                polls.push(ev);
            }

            polls
        } else {
            let data = self.waker_data.clone().unwrap();
            let mut waker_data = data.lock().unwrap();
            waker_data.root_waker = cx.waker().clone();
            waker_data
                .ready_tokens
                .drain(..)
                .filter_map(|f| {
                    if let std::task::Poll::Ready(Some(e)) = self.wake_from(f) {
                        Some(e)
                    } else {
                        None
                    }
                })
                .collect::<Vec<Event>>()
        };

        if self.ternimated {
            std::task::Poll::Ready(None)
        } else if ready_events.is_empty() {
            std::task::Poll::Pending
        } else {
            std::task::Poll::Ready(Some(ready_events))
        }
    }
}

impl LoopInner {
    fn wake_from(&mut self, wake_from: WakeFrom) -> std::task::Poll<Option<Event>> {
        match wake_from {
            WakeFrom::NewItem => self.poll_item_stream(),
            WakeFrom::FutureEvent(index) => {
                let fut_place = self.futures.get(index);
                let mut fut = fut_place.take().unwrap();
                let waker_data = self.waker_data.clone().unwrap();
                let wake_from = WakeFrom::FutureEvent(index);
                let waker = Arc::new(LoopWaker {
                    waker_data,
                    wake_from,
                })
                .into_waker();
                let res = fut.poll_unpin(&mut std::task::Context::from_waker(&waker));
                if res.is_pending() {
                    fut_place.replace(fut);
                }
                res
            }
            WakeFrom::ItemUpdate {
                token,
                item_wake_from,
            } => {
                let item = self.items.get_mut(&token).unwrap();
                let waker_data = self.waker_data.clone().unwrap();

                match item_wake_from {
                    ItemWakeFrom::Disconnect => {
                        let a = item.poll_disconnect(token.clone(), waker_data);
                        a.map(|e| e.map(|ev| self.handle_remove_item(&token, ev)))
                    }
                    ItemWakeFrom::PropertyChange => item.poll_property_change(token, waker_data),
                    ItemWakeFrom::LayoutUpdate => item.poll_layout_change(token, waker_data),
                }
            }
        }
    }

    fn poll_item_stream(&mut self) -> std::task::Poll<Option<Event>> {
        let waker = Arc::new(LoopWaker {
            waker_data: self.waker_data.clone().unwrap(),
            wake_from: WakeFrom::NewItem,
        })
        .into_waker();
        let mut cx = std::task::Context::from_waker(&waker);

        self.watcher_stream_register_notifier_item_registered
            .poll_next_unpin(&mut cx)
            .map(|item| {
                if let Some(item) = item {
                    self.handle_new_item(item)
                } else {
                    self.ternimated = true;
                    None
                }
            })
    }
}
