use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccessContextKind {
    Personal,
    Org,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BillingSubject {
    PersonalAccount {
        billing_subject_id: String,
        ctx_account_id: String,
    },
    Org {
        billing_subject_id: String,
        ctx_org_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccessContext {
    pub kind: AccessContextKind,
    pub billing_subject_id: String,
    pub ctx_user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ctx_account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ctx_org_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ctx_membership_id: Option<String>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AccessContextError {
    #[error("personal access context requires ctx_account_id")]
    MissingAccountId,
    #[error("personal access context must not include ctx_org_id")]
    UnexpectedOrgId,
    #[error("personal access context must not include ctx_membership_id")]
    UnexpectedMembershipId,
    #[error("org access context requires ctx_org_id")]
    MissingOrgId,
    #[error("org access context requires ctx_membership_id")]
    MissingMembershipId,
}

impl AccessContext {
    pub fn validate(&self) -> Result<(), AccessContextError> {
        match self.kind {
            AccessContextKind::Personal => {
                if self.ctx_account_id.is_none() {
                    return Err(AccessContextError::MissingAccountId);
                }
                if self.ctx_org_id.is_some() {
                    return Err(AccessContextError::UnexpectedOrgId);
                }
                if self.ctx_membership_id.is_some() {
                    return Err(AccessContextError::UnexpectedMembershipId);
                }
            }
            AccessContextKind::Org => {
                if self.ctx_org_id.is_none() {
                    return Err(AccessContextError::MissingOrgId);
                }
                if self.ctx_membership_id.is_none() {
                    return Err(AccessContextError::MissingMembershipId);
                }
            }
        }

        Ok(())
    }

    pub fn billing_subject(&self) -> Result<BillingSubject, AccessContextError> {
        self.validate()?;

        match self.kind {
            AccessContextKind::Personal => Ok(BillingSubject::PersonalAccount {
                billing_subject_id: self.billing_subject_id.clone(),
                ctx_account_id: self
                    .ctx_account_id
                    .clone()
                    .ok_or(AccessContextError::MissingAccountId)?,
            }),
            AccessContextKind::Org => Ok(BillingSubject::Org {
                billing_subject_id: self.billing_subject_id.clone(),
                ctx_org_id: self
                    .ctx_org_id
                    .clone()
                    .ok_or(AccessContextError::MissingOrgId)?,
            }),
        }
    }
}
