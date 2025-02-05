use crate::event::Event;

use super::stream_loop::{Item, LoopInner, Token};

#[derive(Debug)]
pub struct LoopEventAddInner {
    pub events: Vec<Event>,
    pub token: Token,
    pub item: Item,
}

#[derive(Debug)]
pub enum LoopEvent {
    Add(Box<LoopEventAddInner>),
    Remove(Event),
    Updata(Event),
}
impl LoopEvent {
    pub fn process_by_loop(self, lp: &mut LoopInner) -> Vec<Event> {
        match self {
            LoopEvent::Add(inner) => {
                let LoopEventAddInner {
                    events,
                    token,
                    item,
                } = *inner;
                lp.wrap_new_item_added(events, token, item)
            }
            LoopEvent::Remove(event) => {
                lp.item_removed(&event.destination);
                vec![event]
            }
            LoopEvent::Updata(event) => vec![event],
        }
    }
}
