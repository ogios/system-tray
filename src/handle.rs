use tracing::{debug, error};
use zbus::{
    fdo::{DBusProxy, NameOwnerChangedStream, PropertiesProxy},
    names::InterfaceName,
    proxy::SignalStream,
    Message,
};

use crate::{
    client::UpdateEvent,
    dbus::{
        dbus_menu_proxy::{DBusMenuProxy, LayoutUpdated, LayoutUpdatedStream},
        notifier_item_proxy::StatusNotifierItemProxy,
        notifier_watcher_proxy::StatusNotifierWatcherProxy,
        DBusProps,
    },
    error::Result,
    item::StatusNotifierItem,
    menu::TrayMenu,
    stream::{Item, LoopInner, LoopWaker, Token, WakeFrom},
};

#[derive(Debug, Clone)]
pub struct Event {
    pub destination: String,
    pub t: EventType,
}
impl Event {
    pub(crate) fn new(destination: String, t: EventType) -> Self {
        Self { destination, t }
    }
}

#[derive(Debug, Clone)]
pub enum EventType {
    /// A new `StatusNotifierItem` was added.
    Add(Box<StatusNotifierItem>),
    /// An update was received for an existing `StatusNotifierItem`.
    /// This could be either an update to the item itself,
    /// or an update to the associated menu.
    Update(UpdateEvent),
    /// A `StatusNotifierItem` was unregistered.
    Remove,
}

struct LoopEventAddInner {
    events: Vec<Event>,
    destination: String,
    disconnect_stream: NameOwnerChangedStream,
    property_change_stream: SignalStream<'static>,
    layout_updated_stream: Option<LayoutUpdatedStream>,
}

pub(crate) enum LoopEvent {
    Add(Box<LoopEventAddInner>),
    Remove(Event),
    Updata(Event),
}
impl LoopEvent {
    pub(crate) fn process_by_loop(self, lp: &mut LoopInner) -> Vec<Event> {
        match self {
            LoopEvent::Add(inner) => {
                let LoopEventAddInner {
                    events,
                    disconnect_stream,
                    property_change_stream,
                    layout_updated_stream,
                    destination,
                } = *inner;
                lp.new_item_added(
                    &destination,
                    disconnect_stream,
                    property_change_stream,
                    layout_updated_stream,
                );
                events
            }
            LoopEvent::Remove(event) => {
                lp.item_removed(&event.destination);
                vec![event]
            }
            LoopEvent::Updata(event) => vec![event],
        }
    }
}

pub(crate) fn to_update_item_event(m: Message) -> Vec<Event> {
    todo!()
}

pub(crate) fn to_layout_update_event(up: LayoutUpdated) -> Vec<Event> {
    todo!()
}

const PROPERTIES_INTERFACE: &str = "org.kde.StatusNotifierItem";

async fn get_item_properties(
    destination: &str,
    path: &str,
    properties_proxy: &PropertiesProxy<'_>,
) -> Result<StatusNotifierItem> {
    let properties = properties_proxy
        .get_all(
            InterfaceName::from_static_str(PROPERTIES_INTERFACE)
                .expect("to be valid interface name"),
        )
        .await;

    let properties = match properties {
        Ok(properties) => properties,
        Err(err) => {
            error!("Error fetching properties from {destination}{path}: {err:?}");
            return Err(err.into());
        }
    };

    StatusNotifierItem::try_from(DBusProps(properties))
}

impl LoopInner {
    pub(crate) fn handle_new_item(
        &mut self,
        item: crate::dbus::notifier_watcher_proxy::StatusNotifierItemRegistered,
    ) -> Vec<Event> {
        let connection = self.connection.clone();

        let fut = async move {
            let res: Result<LoopEvent> = async {
                let address = item.args().map(|args| args.service)?;

                let (destination, path) = parse_address(address);

                let properties_proxy = PropertiesProxy::builder(&connection)
                    .destination(destination.to_string())?
                    .path(path.clone())?
                    .build()
                    .await?;

                let properties = get_item_properties(destination, &path, &properties_proxy).await?;

                let mut events = vec![Event::new(
                    destination.to_string(),
                    EventType::Add(properties.clone().into()),
                )];

                let notifier_item_proxy = StatusNotifierItemProxy::builder(&connection)
                    .destination(destination)?
                    .path(path.clone())?
                    .build()
                    .await?;
                let dbus_proxy = DBusProxy::new(&connection).await?;
                let disconnect_stream = dbus_proxy.receive_name_owner_changed().await?;
                let property_change_stream =
                    notifier_item_proxy.inner().receive_all_signals().await?;

                let layout_updated_stream = if let Some(menu_path) = properties.menu {
                    let destination = destination.to_string();

                    let dbus_menu_proxy = DBusMenuProxy::builder(&connection)
                        .destination(destination.as_str())?
                        .path(menu_path)?
                        .build()
                        .await?;

                    let menu = dbus_menu_proxy.get_layout(0, -1, &[]).await?;
                    let menu = TrayMenu::try_from(menu)?;

                    events.push(Event::new(
                        destination.to_string(),
                        EventType::Update(UpdateEvent::Menu(menu)),
                    ));

                    Some(dbus_menu_proxy.receive_layout_updated().await?)
                } else {
                    None
                };

                Ok(LoopEvent::Add(Box::new(LoopEventAddInner {
                    events,
                    destination: destination.to_string(),
                    disconnect_stream,
                    property_change_stream,
                    layout_updated_stream,
                })))
            }
            .await
            .inspect_err(|e| tracing::error!("Fail to handle_new_item: {e}"));

            res.ok()
        };

        self.futures
            .try_put(fut, self.waker_data.clone().unwrap())
            .map(|e| e.process_by_loop(self))
            .unwrap_or_default()
    }

    pub(crate) fn wrap_new_item_added(
        &mut self,
        mut events: Vec<Event>,
        destination: String,
        disconnect_stream: NameOwnerChangedStream,
        property_change_stream: SignalStream<'static>,
        layout_updated_stream: Option<LayoutUpdatedStream>,
    ) -> Vec<Event> {
        let mut e = self.new_item_added(
            &destination,
            disconnect_stream,
            property_change_stream,
            layout_updated_stream,
        );
        events.append(&mut e);
        events
    }

    pub(crate) fn new_item_added(
        &mut self,
        destination: &str,
        disconnect_stream: NameOwnerChangedStream,
        property_change_stream: SignalStream<'static>,
        layout_updated_stream: Option<LayoutUpdatedStream>,
    ) -> Vec<Event> {
        let token = Token::new(destination.to_string());
        let mut item = Item {
            disconnect_stream,
            property_change_stream,
            layout_updated_stream,
        };
        // watch disconnect
        let disconnect_stream = item
            .poll_disconnect(token.clone(), self.waker_data.clone().unwrap())
            .map(|f| {
                f.map(|f| self.handle_remove_item(&token, f))
                    .unwrap_or_default()
            });
        // this means the connection is disconnected before we insert
        if let std::task::Poll::Ready(e) = disconnect_stream {
            return e;
        }

        let mut es = vec![];

        // watch property
        let property_changed =
            item.poll_property_change(token.clone(), self.waker_data.clone().unwrap());
        if let std::task::Poll::Ready(mut e) = property_changed {
            es.append(&mut e);
        }

        // watch layout
        let layout_changed =
            item.poll_layout_change(token.clone(), self.waker_data.clone().unwrap());
        if let std::task::Poll::Ready(mut e) = layout_changed {
            es.append(&mut e);
        }

        self.items.insert(token, item);

        es
    }

    pub(crate) fn handle_remove_item(
        &mut self,
        token: &Token,
        n: zbus::fdo::NameOwnerChanged,
    ) -> Vec<Event> {
        let destination = token.destination.clone();
        let connection = self.connection.clone();

        let fut = async move {
            let args = n
                .args()
                .inspect_err(|e| error!("Failed to parse NameOwnerChanged: {e:?}"))
                .ok()?;
            let old = args.old_owner();
            let new = args.new_owner();

            if let (Some(old), None) = (old.as_ref(), new.as_ref()) {
                if old == destination.as_str() {
                    debug!("[{destination}] disconnected");

                    let watcher_proxy = StatusNotifierWatcherProxy::new(&connection)
                        .await
                        .expect("Failed to open StatusNotifierWatcherProxy");

                    if let Err(error) = watcher_proxy.unregister_status_notifier_item(old).await {
                        error!("{error:?}");
                    }

                    return Some(LoopEvent::Remove(Event::new(
                        destination.to_string(),
                        EventType::Remove,
                    )));
                };
            }

            None
        };

        self.futures
            .try_put(fut, self.waker_data.clone().unwrap())
            .map(|e| e.process_by_loop(self))
            .unwrap_or_default()
    }

    pub(crate) fn item_removed(&mut self, destination: &str) {
        self.items.remove(&Token::new(destination.to_string()));
    }
}

fn parse_address(address: &str) -> (&str, String) {
    address
        .split_once('/')
        .map_or((address, String::from("/StatusNotifierItem")), |(d, p)| {
            (d, format!("/{p}"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_unnamed() {
        let address = ":1.58/StatusNotifierItem";
        let (destination, path) = parse_address(address);

        assert_eq!(":1.58", destination);
        assert_eq!("/StatusNotifierItem", path);
    }

    #[test]
    fn parse_named() {
        let address = ":1.72/org/ayatana/NotificationItem/dropbox_client_1398";
        let (destination, path) = parse_address(address);

        assert_eq!(":1.72", destination);
        assert_eq!("/org/ayatana/NotificationItem/dropbox_client_1398", path);
    }
}
