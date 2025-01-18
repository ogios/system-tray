use thiserror::Error;

#[cfg(feature = "broadcast")]
#[cfg(not(feature = "calloop-channel"))]
use tokio::sync::broadcast::error::SendError;

#[cfg(feature = "calloop-channel")]
#[cfg(not(feature = "broadcast"))]
use std::sync::mpsc::SendError;

use crate::event::Event;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("dbus properties missing one or more required fields")]
    MissingProperty(&'static str),
    #[cfg(feature = "broadcast")]
    #[cfg(not(feature = "calloop-channel"))]
    #[error("failed to send event through tokio broadcast channel")]
    EventSend(#[from] SendError<Event>),
    #[cfg(feature = "calloop-channel")]
    #[cfg(not(feature = "broadcast"))]
    #[error("failed to send event through tokio broadcast channel")]
    EventSend(#[from] SendError<Event>),
    #[error("zbus error")]
    ZBus(#[from] zbus::Error),
    #[error("zbus fdo error")]
    ZBusFdo(#[from] zbus::fdo::Error),
    #[error("zbus variant error")]
    ZBusVariant(#[from] zbus::zvariant::Error),
    #[error("invalid data error")]
    InvalidData(&'static str),
}
