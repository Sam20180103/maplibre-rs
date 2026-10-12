use thiserror::Error;

use crate::{
    context::MapContext,
    environment::Environment,
    map::Map,
    window::{HeadedMapWindow, MapWindowConfig},
};

pub trait EventLoopConfig {
    type EventType: 'static;
    type EventLoopProxy: EventLoopProxy<Self::EventType>;

    fn create_proxy() -> Self::EventLoopProxy;
}

/// When sending events to an event loop errors can occur.
#[derive(Error, Debug)]
pub enum SendEventError {
    /// The event loop was already closed
    #[error("event loop is closed")]
    Closed,
}

/// When sending events to an event loop errors can occur.
#[derive(Error, Debug)]
#[error("event loop creation failed")]
pub struct EventLoopError;

pub trait EventLoopProxy<T: 'static> {
    fn send_event(&self, event: T) -> Result<(), SendEventError>;
}

pub trait EventLoop<ET: 'static + PartialEq> {
    type EventLoopProxy: EventLoopProxy<ET>;

    fn run<E>(self, map: Map<E>, max_frames: Option<u64>) -> Result<(), EventLoopError>
    where
        E: Environment,
        <E::MapWindowConfig as MapWindowConfig>::MapWindow: HeadedMapWindow;

    fn create_proxy(&self) -> Self::EventLoopProxy;
}

/// A user event sent to the map through an [`EventLoopProxy`]. The event loop applies it to the
/// map context before the next frame, which lets embedders control the map from outside the loop.
pub trait MapEvent: 'static {
    fn apply(self, context: &mut MapContext);
}

impl MapEvent for () {
    fn apply(self, _context: &mut MapContext) {}
}
