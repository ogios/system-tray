use crate::item::{Status, StatusNotifierItem, Tooltip};
use crate::menu::{MenuDiff, TrayMenu};

/// An event emitted by the client
/// representing a change from either the `StatusNotifierItem`
/// or `DBusMenu` protocols.
#[derive(Debug, Clone)]
pub enum Event {
    /// A new `StatusNotifierItem` was added.
    Add(String, Box<StatusNotifierItem>),
    /// An update was received for an existing `StatusNotifierItem`.
    /// This could be either an update to the item itself,
    /// or an update to the associated menu.
    Update(String, UpdateEvent),
    /// A `StatusNotifierItem` was unregistered.
    Remove(String),
}

/// The specific change associated with an update event.
#[derive(Debug, Clone)]
pub enum UpdateEvent {
    AttentionIcon(Option<String>),
    Icon(Option<String>),
    OverlayIcon(Option<String>),
    Status(Status),
    Title(Option<String>),
    Tooltip(Option<Tooltip>),
    /// A menu layout has changed.
    /// The entire layout is sent.
    Menu(TrayMenu),
    /// One or more menu properties have changed.
    /// Only the updated properties are sent.
    MenuDiff(Vec<MenuDiff>),
    /// A new menu has connected to the item.
    /// Its name on bus is sent.
    MenuConnect(String),
}

/// A request to 'activate' one of the menu items,
/// typically sent when it is clicked.
#[derive(Debug, Clone)]
pub enum ActivateRequest {
    /// Submenu ID
    MenuItem {
        address: String,
        menu_path: String,
        submenu_id: i32,
    },
    /// Default activation for the tray.
    /// The parameter(x and y) represents screen coordinates and is to be considered an hint to the item where to show eventual windows (if any).
    Default { address: String, x: i32, y: i32 },
    /// Secondary activation(less important) for the tray.
    /// The parameter(x and y) represents screen coordinates and is to be considered an hint to the item where to show eventual windows (if any).
    Secondary { address: String, x: i32, y: i32 },
}
