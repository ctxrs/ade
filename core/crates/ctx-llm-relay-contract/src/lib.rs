pub mod claims;
pub mod credits;
pub mod pricing;
pub mod request;
pub mod route;
pub mod state;
pub mod subject;

pub use claims::{GrantValidationError, ProviderModelRef, RelayDelegationClaims, RunGrantClaims};
pub use credits::{
    allocate_credit_reservation, finalize_credit_reservation, release_credit_reservation,
    CreditAllocation, CreditAllocationError, CreditAllocationSettlement, CreditGrant,
    CreditGrantSource, CreditReservation,
};
pub use pricing::{ModelPrice, PricingCatalog};
pub use request::{
    validate_openai_responses_request_v1, OpenAiResponsesRequestV1Envelope,
    OpenAiResponsesRequestValidationError, ResponsesContentPart, ResponsesInput, ResponsesMessage,
    ResponsesMessageContent, ResponsesRole, ValidatedFunctionTool,
    ValidatedOpenAiResponsesRequestV1,
};
pub use route::{
    CredentialOwner, GovernanceLevel, RouteAuthMethod, RouteModel, RouteStatus, RouteType,
    UserManagedAuth, UserManagedAuthMethod,
};
pub use state::RelayRequestState;
pub use subject::{AccessContext, AccessContextError, AccessContextKind, BillingSubject};

pub const RELAY_AUDIENCE: &str = "ctx-llm-relay";
pub const CONTROL_PLANE_ISSUER: &str = "ctx-control-plane";
pub const V1_MAX_DELEGATION_TTL_SECONDS: i64 = 300;

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{BillingSubject, ResponsesContentPart};

    #[test]
    fn serde_enum_tags_match_contract() {
        let billing = BillingSubject::Org {
            billing_subject_id: "bill_org".to_string(),
            ctx_org_id: "org_123".to_string(),
        };
        assert_eq!(
            serde_json::to_value(&billing).ok(),
            Some(json!({
                "kind": "org",
                "billing_subject_id": "bill_org",
                "ctx_org_id": "org_123"
            }))
        );

        let part = ResponsesContentPart::InputText {
            text: "hello".to_string(),
        };
        assert_eq!(
            serde_json::to_value(&part).ok(),
            Some(json!({
                "type": "input_text",
                "text": "hello"
            }))
        );
    }
}
