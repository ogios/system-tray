#[cfg(feature = "calloop-channel")]
#[cfg(not(feature = "broadcast"))]
pub mod calloop_channel;
#[cfg(feature = "calloop-channel")]
#[cfg(not(feature = "broadcast"))]
pub use calloop_channel::*;

#[cfg(feature = "broadcast")]
#[cfg(not(feature = "calloop-channel"))]
pub mod tokio_broadcast;
#[cfg(feature = "broadcast")]
#[cfg(not(feature = "calloop-channel"))]
pub use tokio_broadcast::*;

use crate::{item::StatusNotifierItem, menu::TrayMenu};
use std::collections::HashMap;

type State = HashMap<String, (StatusNotifierItem, Option<TrayMenu>)>;

const PROPERTIES_INTERFACE: &str = "org.kde.StatusNotifierItem";

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
