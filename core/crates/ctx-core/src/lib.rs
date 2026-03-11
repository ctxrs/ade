pub mod boolish;
pub mod ids;
pub mod models;

#[cfg(test)]
mod tests {
    use super::models::*;

    #[test]
    fn task_status_serializes_snake_case() {
        let v = serde_json::to_string(&TaskStatus::Pending).unwrap();
        assert_eq!(v, "\"pending\"");
    }
}
