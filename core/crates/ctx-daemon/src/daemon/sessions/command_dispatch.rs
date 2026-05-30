#[derive(Debug)]
pub enum SessionSchedulerCommandError {
    BadRequest,
    NotFound,
    StoreUnavailable,
}
