use crate::{
    item::{Status, StatusNotifierItem, Tooltip},
    menu::{MenuDiff, TrayMenu},
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
