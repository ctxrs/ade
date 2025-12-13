pub mod acp;
pub mod adapters;
pub mod events;
pub mod fake;
pub mod tier1;

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc;

    use crate::adapters::ProviderAdapter;
    use crate::fake::FakeProviderAdapter;

    #[tokio::test]
    async fn fake_provider_emits_events() {
        let adapter = FakeProviderAdapter::new();
        let (tx, mut rx) = mpsc::channel(16);
        let _handle = adapter
            .run(
                crate::adapters::TurnInput {
                    content: "hi".into(),
                    attachments: vec![],
                },
                std::env::current_dir().unwrap(),
                Default::default(),
                tx,
            )
            .await
            .unwrap();

        let mut types = Vec::new();
        while let Some(ev) = rx.recv().await {
            types.push(ev.event_type);
            if types.len() >= 5 {
                break;
            }
        }
        assert!(types.len() >= 4);
    }
}
