use zbus::Message;

use crate::{client::Event, dbus::dbus_menu_proxy::LayoutUpdated};

pub(crate) fn to_update_item_event(m: Message) -> Event {
    todo!()
}

pub(crate) fn to_layout_update_event(up: LayoutUpdated) -> Event {
    todo!()
}
