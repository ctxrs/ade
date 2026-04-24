pub const LEGACY_CODEX_PROVIDER_ID: &str = "codex";
pub const CODEX_CRP_PROVIDER_ID: &str = "codex-crp";

pub fn canonical_provider_id(provider_id: &str) -> &str {
    if provider_id == LEGACY_CODEX_PROVIDER_ID {
        CODEX_CRP_PROVIDER_ID
    } else {
        provider_id
    }
}

pub fn legacy_provider_id_alias(provider_id: &str) -> Option<&'static str> {
    if provider_id == CODEX_CRP_PROVIDER_ID {
        Some(LEGACY_CODEX_PROVIDER_ID)
    } else {
        None
    }
}

pub fn provider_id_matches(provider_id: &str, canonical_provider_id_value: &str) -> bool {
    canonical_provider_id(provider_id) == canonical_provider_id_value
}

#[cfg(test)]
mod tests {
    use super::{
        canonical_provider_id, legacy_provider_id_alias, provider_id_matches,
        CODEX_CRP_PROVIDER_ID, LEGACY_CODEX_PROVIDER_ID,
    };

    #[test]
    fn legacy_codex_alias_canonicalizes_to_codex_crp() {
        assert_eq!(
            canonical_provider_id(LEGACY_CODEX_PROVIDER_ID),
            CODEX_CRP_PROVIDER_ID
        );
        assert_eq!(
            canonical_provider_id(CODEX_CRP_PROVIDER_ID),
            CODEX_CRP_PROVIDER_ID
        );
    }

    #[test]
    fn canonical_codex_crp_projects_legacy_alias() {
        assert_eq!(
            legacy_provider_id_alias(CODEX_CRP_PROVIDER_ID),
            Some(LEGACY_CODEX_PROVIDER_ID)
        );
        assert_eq!(legacy_provider_id_alias("claude-crp"), None);
    }

    #[test]
    fn provider_id_match_uses_canonical_identity() {
        assert!(provider_id_matches(
            LEGACY_CODEX_PROVIDER_ID,
            CODEX_CRP_PROVIDER_ID
        ));
        assert!(provider_id_matches(
            CODEX_CRP_PROVIDER_ID,
            CODEX_CRP_PROVIDER_ID
        ));
        assert!(!provider_id_matches("claude-crp", CODEX_CRP_PROVIDER_ID));
    }
}
