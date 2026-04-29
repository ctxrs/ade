use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RouteType {
    CtxManaged,
    UserManaged,
    CustomerGateway,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CredentialOwner {
    Ctx,
    User,
    Customer,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RouteAuthMethod {
    CtxProviderKey,
    #[serde(rename = "oauth")]
    OAuth,
    ApiKey,
    GatewayToken,
    Mtls,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GovernanceLevel {
    HardEnforced,
    PolicyEnforcedLocal,
    ReceiptEnforced,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RouteStatus {
    Enabled,
    Disabled,
    Planned,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RouteModel {
    pub id: String,
    #[serde(rename = "type")]
    pub route_type: RouteType,
    pub credential_owner: CredentialOwner,
    pub auth_method: RouteAuthMethod,
    pub governance_level: GovernanceLevel,
    pub status: RouteStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UserManagedAuthMethod {
    #[serde(rename = "oauth")]
    OAuth,
    ApiKey,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserManagedAuth {
    pub method: UserManagedAuthMethod,
}

impl UserManagedAuth {
    pub fn route_auth_method(&self) -> RouteAuthMethod {
        match self.method {
            UserManagedAuthMethod::OAuth => RouteAuthMethod::OAuth,
            UserManagedAuthMethod::ApiKey => RouteAuthMethod::ApiKey,
        }
    }
}
