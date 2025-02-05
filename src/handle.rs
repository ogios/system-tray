use tracing::{debug, error, warn};
use zbus::{fdo::PropertiesProxy, names::InterfaceName, zvariant::Structure, Message};

use crate::{
    dbus::{dbus_menu_proxy::DBusMenuProxy, DBusProps, OwnedValueExt},
    error::Result,
    event::{Event, UpdateEvent},
    item::{self, StatusNotifierItem},
    menu::TrayMenu,
};

pub async fn get_new_layout(destination: String, proxy: &DBusMenuProxy<'static>) -> Result<Event> {
    let menu = proxy.get_layout(0, -1, &[]).await?;
    let menu = TrayMenu::try_from(menu)?;
    Ok(Event::Update(destination.clone(), UpdateEvent::Menu(menu)))
}

const PROPERTIES_INTERFACE: &str = "org.kde.StatusNotifierItem";

pub async fn get_update_event(
    change: Message,
    properties_proxy: &PropertiesProxy<'_>,
) -> Option<UpdateEvent> {
    let header = change.header();
    let member = header.member()?;

    let property_name = match member.as_str() {
        "NewAttentionIcon" => "AttentionIconName",
        "NewIcon" => "IconName",
        "NewOverlayIcon" => "OverlayIconName",
        "NewStatus" => "Status",
        "NewTitle" => "Title",
        "NewToolTip" => "ToolTip",
        _ => &member.as_str()["New".len()..],
    };

    let res = properties_proxy
        .get(
            InterfaceName::from_static_str(PROPERTIES_INTERFACE)
                .expect("to be valid interface name"),
            property_name,
        )
        .await;

    let property = match res {
        Ok(property) => property,
        Err(err) => {
            error!("error fetching property '{property_name}': {err:?}");
            return None;
        }
    };

    debug!("received tray item update: {member} -> {property:?}");

    use UpdateEvent::*;
    match member.as_str() {
        "NewAttentionIcon" => Some(AttentionIcon(property.to_string().ok())),
        "NewIcon" => Some(Icon(property.to_string().ok())),
        "NewOverlayIcon" => Some(OverlayIcon(property.to_string().ok())),
        "NewStatus" => Some(Status(
            property
                .downcast_ref::<&str>()
                .ok()
                .map(item::Status::from)
                .unwrap_or_default(),
        )),
        "NewTitle" => Some(Title(property.to_string().ok())),
        "NewToolTip" => Some(Tooltip({
            property
                .downcast_ref::<&Structure>()
                .ok()
                .map(crate::item::Tooltip::try_from)?
                .ok()
        })),
        _ => {
            warn!("received unhandled update event: {member}");
            None
        }
    }
}

pub async fn get_item_properties(
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

pub fn parse_address(address: &str) -> (&str, String) {
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
