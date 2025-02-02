use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    task::Waker,
};

use cooked_waker::{IntoWaker, WakeRef};
use futures::{Stream, StreamExt};
use zbus::{fdo::NameOwnerChangedStream, proxy::SignalStream};

use crate::{
    client::Event,
    dbus::{
        dbus_menu_proxy::LayoutUpdatedStream,
        notifier_watcher_proxy::StatusNotifierItemRegisteredStream,
    },
    handle::{to_layout_update_event, to_update_item_event},
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Token {
    destination: Arc<String>,
}

#[derive(Debug, Clone)]
enum ItemWakeFrom {
    Disconnect,
    PropertyChange,
    LayoutUpdate,
}

#[derive(Debug, Clone)]
enum WakeFrom {
    NewItem,
    ItemUpdate {
        token: Token,
        item_wake_from: ItemWakeFrom,
    },
}

#[derive(Debug)]
struct WakerData {
    ready_tokens: Vec<WakeFrom>,
    root_waker: Waker,
}

#[derive(Debug, Clone)]
struct LoopWaker {
    waker_data: Arc<Mutex<WakerData>>,
    wake_from: WakeFrom,
}

impl WakeRef for LoopWaker {
    fn wake_by_ref(&self) {
        let mut data = self.waker_data.lock().unwrap();
        data.ready_tokens.push(self.wake_from.clone());
        data.root_waker.wake_by_ref();
    }
}

struct Item {
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

struct LoopInner {
    waker_data: Arc<Mutex<WakerData>>,
    ternimated: bool,

    /// watcher_proxy.receive_status_notifier_item_registered
    watcher_stream_register_notifier_item_registered: StatusNotifierItemRegisteredStream,
    /// key: destination (":1.52")
    items: HashMap<Token, Item>,
    // NOTE: dbus_proxy.receive_name_acquired will not be added currently,
    // cosmic applet didn't do it.
}

impl Stream for LoopInner {
    type Item = Vec<Event>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let data = self.waker_data.clone();
        let mut waker_data = data.lock().unwrap();
        waker_data.root_waker = cx.waker().clone();
        let ready_events = waker_data
            .ready_tokens
            .drain(..)
            .filter_map(|f| {
                if let std::task::Poll::Ready(Some(e)) = self.wake_from(f) {
                    Some(e)
                } else {
                    None
                }
            })
            .collect::<Vec<Event>>();

        if self.ternimated {
            return std::task::Poll::Ready(None);
        }
        todo!()
    }
}

impl LoopInner {
    fn wake_from(&mut self, wake_from: WakeFrom) -> std::task::Poll<Option<Event>> {
        match wake_from {
            WakeFrom::NewItem => self.poll_item_stream(),
            WakeFrom::ItemUpdate {
                token,
                item_wake_from,
            } => {
                let item = self.items.get_mut(&token).unwrap();
                let waker_data = self.waker_data.clone();

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
            waker_data: self.waker_data.clone(),
            wake_from: WakeFrom::NewItem,
        })
        .into_waker();
        let mut cx = std::task::Context::from_waker(&waker);

        self.watcher_stream_register_notifier_item_registered
            .poll_next_unpin(&mut cx)
            .map(|item| {
                if let Some(item) = item {
                    Some(self.handle_new_item(item))
                } else {
                    self.ternimated = true;
                    None
                }
            })
    }
}

impl LoopInner {
    fn handle_new_item(
        &mut self,
        item: crate::dbus::notifier_watcher_proxy::StatusNotifierItemRegistered,
    ) -> Event {
        todo!()
    }

    fn handle_remove_item(&mut self, token: &Token, n: zbus::fdo::NameOwnerChanged) -> Event {
        todo!()
    }
}
