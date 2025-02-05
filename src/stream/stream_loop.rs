use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    task::Context,
};

use cooked_waker::IntoWaker;
use futures::{Stream, StreamExt};
use tracing::error;
use zbus::{
    fdo::{NameOwnerChangedStream, PropertiesProxy},
    proxy::SignalStream,
    Connection,
};

use crate::{
    dbus::{
        dbus_menu_proxy::{DBusMenuProxy, LayoutUpdatedStream},
        notifier_watcher_proxy::{StatusNotifierItemRegisteredStream, StatusNotifierWatcherProxy},
    },
    error::Result,
    event::Event,
};

use super::{
    future::{FutureMap, LoopEvent},
    handle::{to_layout_update_event, to_update_item_event},
    waker::{ItemWakeFrom, LoopWaker, WakeFrom, WakerData},
};

/// Token is used to identify an item.
/// destination example: ":1.52"
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Token {
    pub destination: Arc<String>,
}
impl Token {
    pub fn new(destination: String) -> Self {
        Self {
            destination: Arc::new(destination),
        }
    }
}

#[derive(Debug)]
pub struct Item {
    pub dbus_menu_proxy: Option<DBusMenuProxy<'static>>,
    pub properties_proxy: PropertiesProxy<'static>,
    pub disconnect_stream: NameOwnerChangedStream,
    pub property_change_stream: SignalStream<'static>,
    pub layout_updated_stream: Option<LayoutUpdatedStream>,
    // NOTE: dbus_menu_proxy.receive_items_properties_updated is not added,
    // as i find it useless or has bugs, everything it provides is None except for `visible` been
    // removed
}
impl Item {
    pub fn poll_disconnect(
        &mut self,
        token: Token,
        waker_data: Arc<Mutex<WakerData>>,
    ) -> Vec<zbus::fdo::NameOwnerChanged> {
        let waker = Arc::new(LoopWaker {
            waker_data: waker_data.clone(),
            wake_from: WakeFrom::ItemUpdate {
                token,
                item_wake_from: ItemWakeFrom::Disconnect,
            },
        })
        .into_waker();
        let mut cx = std::task::Context::from_waker(&waker);

        loop_until_pending(&mut self.disconnect_stream, &mut cx)
            .into_iter()
            .flatten()
            .collect()
    }
    pub fn poll_property_change(
        &mut self,
        token: Token,
        waker_data: Arc<Mutex<WakerData>>,
        future_map: &mut FutureMap,
    ) -> Vec<LoopEvent> {
        let waker = LoopWaker::new_waker(
            waker_data.clone(),
            WakeFrom::ItemUpdate {
                token: token.clone(),
                item_wake_from: ItemWakeFrom::PropertyChange,
            },
        );
        let mut cx = std::task::Context::from_waker(&waker);

        loop_until_pending(&mut self.property_change_stream, &mut cx)
            .into_iter()
            .flatten()
            .filter_map(|m| {
                future_map.try_put(
                    to_update_item_event(
                        token.destination.as_str().to_string(),
                        m,
                        self.properties_proxy.clone(),
                    ),
                    waker_data.clone(),
                )
            })
            .collect()
    }
    pub fn poll_layout_change(
        &mut self,
        token: Token,
        waker_data: Arc<Mutex<WakerData>>,
        future_map: &mut FutureMap,
    ) -> Vec<LoopEvent> {
        let waker = LoopWaker::new_waker(
            waker_data.clone(),
            WakeFrom::ItemUpdate {
                token: token.clone(),
                item_wake_from: ItemWakeFrom::LayoutUpdate,
            },
        );
        let mut cx = std::task::Context::from_waker(&waker);

        self.layout_updated_stream
            .as_mut()
            .map(|st| {
                loop_until_pending(st, &mut cx)
                    .into_iter()
                    .flatten()
                    .filter_map(|_| {
                        future_map.try_put(
                            to_layout_update_event(
                                token.destination.as_str().to_string(),
                                self.dbus_menu_proxy.clone().unwrap(),
                            ),
                            waker_data.clone(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

pub struct LoopInner {
    pub waker_data: Option<Arc<Mutex<WakerData>>>,
    pub ternimated: bool,
    pub polled: bool,
    pub futures: FutureMap,

    pub connection: Connection,
    pub watcher_stream_register_notifier_item_registered: StatusNotifierItemRegisteredStream,
    pub items: HashMap<Token, Item>,
    // NOTE: dbus_proxy.receive_name_acquired will not be added currently,
    // cosmic applet didn't do it.
}
impl LoopInner {
    pub(super) async fn new(
        connection: Connection,
        watcher_proxy: StatusNotifierWatcherProxy<'static>,
    ) -> Result<Self> {
        let watcher_stream_register_notifier_item_registered = watcher_proxy
            .receive_status_notifier_item_registered()
            .await?;

        let mut futures = FutureMap::new();
        futures.put(async move {
            watcher_proxy
                .registered_status_notifier_items()
                .await
                .inspect_err(|e| error!("error getting initial items: {e}"))
                .ok()
                .map(LoopEvent::Init)
        });

        Ok(Self {
            waker_data: None,
            ternimated: false,
            polled: false,
            futures,

            connection,
            watcher_stream_register_notifier_item_registered,
            items: HashMap::new(),
        })
    }
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
            self.waker_data = Some(Arc::new(Mutex::new(WakerData {
                ready_tokens: Vec::new(),
                root_waker: cx.waker().clone(),
            })));

            self.first_poll()
        } else {
            let data = self.waker_data.clone().unwrap();
            let mut waker_data = data.lock().unwrap();
            waker_data.root_waker = cx.waker().clone();
            let sources: Vec<WakeFrom> = waker_data.ready_tokens.drain(..).collect();
            drop(waker_data);
            sources
                .into_iter()
                .flat_map(|f| self.wake_from(f))
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
    fn first_poll(&mut self) -> Vec<Event> {
        let waker_data = self.waker_data.clone().unwrap();
        let mut polls = vec![];

        if let Some(le) = self.futures.try_poll(0, waker_data) {
            polls.append(&mut le.process_by_loop(self));
        }

        polls.append(&mut self.poll_item_stream());

        polls
    }
    fn wake_from(&mut self, wake_from: WakeFrom) -> Vec<Event> {
        println!("waker from: {wake_from:?}");
        match wake_from {
            WakeFrom::NewItem => self.poll_item_stream(),
            WakeFrom::FutureEvent(index) => self
                .futures
                .try_poll(index, self.waker_data.clone().unwrap())
                .map(|le| le.process_by_loop(self))
                .unwrap_or_default(),
            WakeFrom::ItemUpdate {
                token,
                item_wake_from,
            } => {
                let item = self.items.get_mut(&token).unwrap();
                let waker_data = self.waker_data.clone().unwrap();

                match item_wake_from {
                    ItemWakeFrom::Disconnect => item
                        .poll_disconnect(token.clone(), waker_data)
                        .into_iter()
                        .flat_map(|d| self.handle_remove_item(&token, d))
                        .collect(),
                    ItemWakeFrom::PropertyChange => item
                        .poll_property_change(token, waker_data, &mut self.futures)
                        .into_iter()
                        .flat_map(|e| e.process_by_loop(self))
                        .collect(),
                    ItemWakeFrom::LayoutUpdate => item
                        .poll_layout_change(token, waker_data, &mut self.futures)
                        .into_iter()
                        .flat_map(|e| e.process_by_loop(self))
                        .collect(),
                }
            }
        }
    }

    fn poll_item_stream(&mut self) -> Vec<Event> {
        let waker = LoopWaker::new_waker(self.waker_data.clone().unwrap(), WakeFrom::NewItem);
        let mut cx = std::task::Context::from_waker(&waker);

        loop_until_pending(
            &mut self.watcher_stream_register_notifier_item_registered,
            &mut cx,
        )
        .into_iter()
        .flatten()
        .flat_map(|item| self.handle_new_item(item))
        .collect()
    }
}

fn loop_until_pending<T, St: Stream<Item = T> + Unpin>(
    st: &mut St,
    cx: &mut Context,
) -> Vec<Option<T>> {
    let mut outputs = vec![];
    while let std::task::Poll::Ready(item) = st.poll_next_unpin(cx) {
        outputs.push(item);
    }

    outputs
}
