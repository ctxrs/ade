#[path = "post/delivery.rs"]
mod delivery;
#[path = "post/handler.rs"]
mod handler;
#[path = "post/request.rs"]
mod request;

pub(crate) use handler::post_message;
