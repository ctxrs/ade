#[path = "post/delivery.rs"]
mod delivery;
#[path = "post/events.rs"]
mod events;
#[path = "post/handler.rs"]
mod handler;
#[path = "post/persistence.rs"]
mod persistence;
#[path = "post/request.rs"]
mod request;
#[path = "post/scheduler.rs"]
mod scheduler;

pub(crate) use handler::post_message;
