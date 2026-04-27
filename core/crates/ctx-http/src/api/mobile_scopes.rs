#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MobileScope {
    DeviceRegistration,
    WorkspaceRead,
    WorkspaceStream,
}

impl MobileScope {
    fn bit(self) -> u8 {
        match self {
            Self::DeviceRegistration => 1 << 0,
            Self::WorkspaceRead => 1 << 1,
            Self::WorkspaceStream => 1 << 2,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::DeviceRegistration => "device_registration",
            Self::WorkspaceRead => "workspace_read",
            Self::WorkspaceStream => "workspace_stream",
        }
    }

    pub(super) fn missing_error(self) -> &'static str {
        match self {
            Self::DeviceRegistration => "mobile profile lacks device_registration scope",
            Self::WorkspaceRead => "mobile profile lacks workspace_read scope",
            Self::WorkspaceStream => "mobile profile lacks workspace_stream scope",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MobileScopeSet(u8);

impl MobileScopeSet {
    fn empty() -> Self {
        Self(0)
    }

    fn insert(&mut self, scope: MobileScope) {
        self.0 |= scope.bit();
    }

    pub(super) fn managed_default() -> Self {
        let mut set = Self::empty();
        set.insert(MobileScope::DeviceRegistration);
        set.insert(MobileScope::WorkspaceRead);
        set.insert(MobileScope::WorkspaceStream);
        set
    }

    pub(super) fn allows(self, scope: MobileScope) -> bool {
        self.0 & scope.bit() != 0
    }

    fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub(super) fn to_strings(self) -> Vec<String> {
        [
            MobileScope::DeviceRegistration,
            MobileScope::WorkspaceRead,
            MobileScope::WorkspaceStream,
        ]
        .into_iter()
        .filter(|scope| self.allows(*scope))
        .map(|scope| scope.as_str().to_string())
        .collect()
    }
}

pub(super) fn default_mobile_profile_scopes() -> Vec<String> {
    MobileScopeSet::managed_default().to_strings()
}

pub(super) fn mobile_scope_set_from_strings(scopes: &[String]) -> Result<MobileScopeSet, String> {
    let mut set = MobileScopeSet::empty();
    for raw_scope in scopes {
        let scope = raw_scope.trim();
        if scope.is_empty() {
            continue;
        }
        let parsed = match scope {
            "device_registration" => MobileScope::DeviceRegistration,
            "workspace_read" => MobileScope::WorkspaceRead,
            "workspace_stream" => MobileScope::WorkspaceStream,
            _ => return Err(format!("unknown mobile scope: {scope}")),
        };
        set.insert(parsed);
    }
    if set.is_empty() {
        return Err("at least one mobile scope is required".to_string());
    }
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_mobile_default_scope_bundle_is_explicit() {
        assert_eq!(
            default_mobile_profile_scopes(),
            vec![
                "device_registration".to_string(),
                "workspace_read".to_string(),
                "workspace_stream".to_string(),
            ]
        );
    }

    #[test]
    fn mobile_scope_parser_rejects_empty_scope_sets() {
        assert_eq!(
            mobile_scope_set_from_strings(&[]),
            Err("at least one mobile scope is required".to_string())
        );
    }

    #[test]
    fn mobile_scope_parser_rejects_unknown_scopes() {
        assert_eq!(
            mobile_scope_set_from_strings(&["unknown".to_string()]),
            Err("unknown mobile scope: unknown".to_string())
        );
    }
}
