use super::update::{
    UpdateDictationSettingsReq, UpdateLiveKitDictationSettingsReq,
    UpdateTitleGenerationRemoteSettingsReq, UpdateTitleGenerationSettingsReq,
};
use super::*;

#[test]
fn network_profiles_defaults_are_safe_for_system_tasks() {
    let settings = NetworkProfilesSettings::default();
    assert_eq!(settings.agent_default.mode, ContainerNetworkMode::LlmOnly);
    assert_eq!(settings.merge_queue.mode, ContainerNetworkMode::All);
    assert_eq!(settings.worktree_setup.mode, ContainerNetworkMode::All);
    assert_eq!(settings.user_shell.mode, ContainerNetworkMode::All);
    assert!(settings.merge_queue.allowlist.is_empty());
    assert!(settings.worktree_setup.allowlist.is_empty());
}

#[test]
fn to_public_redacts_secret_values() {
    let settings = Settings {
        dictation: Some(DictationSettings {
            enabled: true,
            provider: DictationProvider::LiveKitInference,
            livekit: Some(LiveKitDictationSettings {
                base_url: "https://livekit.example".to_string(),
                api_key: "lk-key".to_string(),
                api_secret: Some("lk-secret".to_string()),
                model: "auto".to_string(),
                language: "en".to_string(),
            }),
        }),
        title_generation: Some(TitleGenerationSettings {
            mode: TitleGenerationMode::Remote,
            remote: TitleGenerationRemoteSettings {
                base_url: "https://titles.example".to_string(),
                api_key: "title-key".to_string(),
                model: "gpt-test".to_string(),
                use_json: true,
            },
            local: TitleGenerationLocalSettings::default(),
        }),
        oracle: Some(OracleSettings {
            api_key: "oracle-key".to_string(),
            ..OracleSettings::default()
        }),
        ..Settings::default()
    };

    let public = to_public(&settings);
    let livekit = public
        .dictation
        .as_ref()
        .and_then(|dictation| dictation.livekit.as_ref())
        .expect("dictation livekit");
    assert!(livekit.api_key_set);
    assert!(livekit.api_secret_set);

    let title_remote = public
        .title_generation
        .as_ref()
        .map(|titling| &titling.remote)
        .expect("title generation");
    assert!(title_remote.api_key_set);
    assert_eq!(title_remote.base_url, "https://titles.example");
    assert_eq!(title_remote.model, "gpt-test");

    let oracle = public.oracle.as_ref().expect("oracle");
    assert!(oracle.api_key_set);
}

#[test]
fn apply_update_preserves_existing_secret_values_when_omitted() {
    let current = Settings {
        dictation: Some(DictationSettings {
            enabled: true,
            provider: DictationProvider::LiveKitInference,
            livekit: Some(LiveKitDictationSettings {
                base_url: "https://livekit.example".to_string(),
                api_key: "lk-key".to_string(),
                api_secret: Some("lk-secret".to_string()),
                model: "auto".to_string(),
                language: "en".to_string(),
            }),
        }),
        title_generation: Some(TitleGenerationSettings {
            mode: TitleGenerationMode::Remote,
            remote: TitleGenerationRemoteSettings {
                base_url: "https://titles.example".to_string(),
                api_key: "title-key".to_string(),
                model: "gpt-test".to_string(),
                use_json: false,
            },
            local: TitleGenerationLocalSettings::default(),
        }),
        ..Settings::default()
    };

    let next = apply_update(
        current,
        UpdateSettingsReq {
            dictation: Some(UpdateDictationSettingsReq {
                enabled: true,
                provider: DictationProvider::LiveKitInference,
                livekit: Some(UpdateLiveKitDictationSettingsReq {
                    base_url: "https://livekit.next".to_string(),
                    api_key: None,
                    api_secret: None,
                    model: "new-model".to_string(),
                    language: "es".to_string(),
                }),
            }),
            title_generation: Some(UpdateTitleGenerationSettingsReq {
                mode: TitleGenerationMode::Remote,
                remote: UpdateTitleGenerationRemoteSettingsReq {
                    base_url: "https://titles.next".to_string(),
                    api_key: None,
                    model: "gpt-next".to_string(),
                    use_json: true,
                },
                local: TitleGenerationLocalSettings::default(),
            }),
            oracle: None,
            telemetry: None,
            resource_governance: None,
            provider_guard: None,
            tool_limits: None,
            provider_restart: None,
            subagents: None,
            sandboxing: None,
            execution: None,
            network_profiles: None,
        },
    );

    let livekit = next
        .dictation
        .as_ref()
        .and_then(|dictation| dictation.livekit.as_ref())
        .expect("dictation livekit");
    assert_eq!(livekit.api_key, "lk-key");
    assert_eq!(livekit.api_secret.as_deref(), Some("lk-secret"));
    assert_eq!(livekit.base_url, "https://livekit.next");
    assert_eq!(livekit.model, "new-model");
    assert_eq!(livekit.language, "es");

    let title_remote = &next
        .title_generation
        .as_ref()
        .expect("title generation")
        .remote;
    assert_eq!(title_remote.api_key, "title-key");
    assert_eq!(title_remote.base_url, "https://titles.next");
    assert_eq!(title_remote.model, "gpt-next");
    assert!(title_remote.use_json);
}
