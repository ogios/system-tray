use std::sync::Arc;

use futures::FutureExt;
use tracing::error;
use zbus::{fdo::PropertiesProxy, names::InterfaceName, Message};

use crate::{
    client::Event,
    dbus::{dbus_menu_proxy::LayoutUpdated, DBusProps},
    error::Result,
    item::StatusNotifierItem,
    stream::{LoopInner, LoopWaker, Token, WakeFrom},
};

pub(crate) fn to_update_item_event(m: Message) -> Event {
    todo!()
}

pub(crate) fn to_layout_update_event(up: LayoutUpdated) -> Event {
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
    ) -> Option<Event> {
        let connection = self.connection.clone();

        let mut fut = Box::pin(async move {
            let res: Result<(&str, StatusNotifierItem)> = async {
                let address = item.args().map(|args| args.service)?;

                let (destination, path) = parse_address(address);

                let properties_proxy = PropertiesProxy::builder(&connection)
                    .destination(destination.to_string())?
                    .path(path.clone())?
                    .build()
                    .await?;

                let properties = get_item_properties(destination, &path, &properties_proxy).await?;

                Ok((destination, properties))
            }
            .await
            .inspect_err(|e| tracing::error!("Fail to handle_new_item: {e}"));

            let (destination, properties) = res.ok()?;

            Some(Event::Add(
                destination.to_string(),
                properties.clone().into(),
            ))
        });

        let index = self.futures.preserve_space();
        let waker = LoopWaker::new(
            self.waker_data.clone().unwrap(),
            WakeFrom::FutureEvent(index),
        );
        let res = fut.poll_unpin(&mut std::task::Context::from_waker(&waker));

        if let std::task::Poll::Ready(e) = res {
            if let Some(event) = e {
                return Some(event);
            }
        } else {
            self.futures.get(index).replace(fut);
        }

        None
    }

    pub(crate) fn handle_remove_item(
        &mut self,
        token: &Token,
        n: zbus::fdo::NameOwnerChanged,
    ) -> Event {
        todo!()
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
