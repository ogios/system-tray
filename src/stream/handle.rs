use tracing::{debug, error};
use zbus::{
    fdo::{DBusProxy, PropertiesProxy},
    Connection, Message,
};

use crate::{
    dbus::{
        dbus_menu_proxy::DBusMenuProxy, notifier_item_proxy::StatusNotifierItemProxy,
        notifier_watcher_proxy::StatusNotifierWatcherProxy,
    },
    error::Result,
    event::Event,
    handle::{get_item_properties, get_new_layout, get_update_event, parse_address},
};

use super::{
    future::{LoopEvent, LoopEventAddInner},
    stream_loop::{Item, LoopInner, Token},
};

pub async fn to_update_item_event(
    destination: String,
    m: Message,
    proxy: PropertiesProxy<'static>,
) -> Option<LoopEvent> {
    let e = get_update_event(m, &proxy).await?;
    Some(LoopEvent::Updata(Event::Update(destination, e)))
}

pub async fn to_layout_update_event(
    destination: String,
    proxy: DBusMenuProxy<'static>,
) -> Option<LoopEvent> {
    let e = get_new_layout(destination, &proxy)
        .await
        .inspect_err(|e| error!("error get layout: {e}"))
        .ok()?;

    Some(LoopEvent::Updata(e))
}

pub async fn to_new_item_event(address: &str, connection: &Connection) -> Option<LoopEvent> {
    let res: Result<LoopEvent> = async {
        let (destination, path) = parse_address(address);

        let properties_proxy = PropertiesProxy::builder(connection)
            .destination(destination.to_string())?
            .path(path.clone())?
            .build()
            .await?;

        let properties = get_item_properties(destination, &path, &properties_proxy).await?;

        let mut events = vec![Event::Add(
            destination.to_string(),
            properties.clone().into(),
        )];

        let notifier_item_proxy = StatusNotifierItemProxy::builder(connection)
            .destination(destination)?
            .path(path.clone())?
            .build()
            .await?;
        let dbus_proxy = DBusProxy::new(connection).await?;
        let disconnect_stream = dbus_proxy.receive_name_owner_changed().await?;
        let property_change_stream = notifier_item_proxy.inner().receive_all_signals().await?;

        let (layout_updated_stream, dbus_menu_proxy) = if let Some(menu_path) = properties.menu {
            let destination = destination.to_string();

            let dbus_menu_proxy = DBusMenuProxy::builder(connection)
                .destination(destination.clone())?
                .path(menu_path)?
                .build()
                .await?;

            events.push(get_new_layout(destination, &dbus_menu_proxy).await?);

            (
                Some(dbus_menu_proxy.receive_layout_updated().await?),
                Some(dbus_menu_proxy),
            )
        } else {
            (None, None)
        };

        Ok(LoopEvent::Add(Box::new(LoopEventAddInner {
            events,
            token: Token::new(destination.to_string()),
            item: Item {
                properties_proxy,
                dbus_menu_proxy,
                disconnect_stream,
                property_change_stream,
                layout_updated_stream,
            },
        })))
    }
    .await
    .inspect_err(|e| tracing::error!("Fail to handle_new_item: {e}"));

    res.ok()
}

impl LoopInner {
    pub fn handle_new_item(
        &mut self,
        item: crate::dbus::notifier_watcher_proxy::StatusNotifierItemRegistered,
    ) -> Vec<Event> {
        let connection = self.connection.clone();

        let fut = async move {
            let address = item
                .args()
                .map(|args| args.service)
                .inspect_err(|e| error!("error getting service: {e}"))
                .ok()?;
            to_new_item_event(address, &connection).await
        };

        self.futures
            .try_put(fut, self.waker_data.clone().unwrap())
            .map(|e| e.process_by_loop(self))
            .unwrap_or_default()
    }

    pub fn wrap_new_item_added(
        &mut self,
        mut events: Vec<Event>,
        token: Token,
        item: Item,
    ) -> Vec<Event> {
        let mut e = self.new_item_added(token, item);
        events.append(&mut e);
        events
    }

    pub fn new_item_added(&mut self, token: Token, mut item: Item) -> Vec<Event> {
        let mut es = vec![];

        // watch property
        let property_changed = item.poll_property_change(
            token.clone(),
            self.waker_data.clone().unwrap(),
            &mut self.futures,
        );
        es.append(
            &mut property_changed
                .into_iter()
                .flat_map(|e| e.process_by_loop(self))
                .collect(),
        );

        // watch layout
        let layout_changed = item.poll_layout_change(
            token.clone(),
            self.waker_data.clone().unwrap(),
            &mut self.futures,
        );
        es.append(
            &mut layout_changed
                .into_iter()
                .flat_map(|e| e.process_by_loop(self))
                .collect(),
        );

        // watch disconnect
        let disconnect_stream =
            item.poll_disconnect(token.clone(), self.waker_data.clone().unwrap());

        self.items.insert(token.clone(), item);

        es.append(
            &mut disconnect_stream
                .into_iter()
                .flat_map(|d| self.handle_remove_item(&token, d))
                .collect(),
        );

        es
    }

    pub fn handle_remove_item(
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
                .ok()
                .unwrap();
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

                    return Some(LoopEvent::Remove(Event::Remove(destination.to_string())));
                };
            }

            None
        };

        self.futures
            .try_put(fut, self.waker_data.clone().unwrap())
            .map(|e| e.process_by_loop(self))
            .unwrap_or_default()
    }

    pub fn item_removed(&mut self, destination: &str) {
        self.items.remove(&Token::new(destination.to_string()));
    }
}
