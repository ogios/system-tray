use crate::{
    item::{Status, StatusNotifierItem, Tooltip},
    menu::{MenuDiff, TrayMenu},
};

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
impl Event {
    pub fn destination(&self) -> &str {
        match self {
            Event::Add(d, _) | Event::Update(d, _) | Event::Remove(d) => d,
        }
    }
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
