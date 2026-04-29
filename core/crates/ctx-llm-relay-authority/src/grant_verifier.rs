use base64::Engine as _;
use ctx_llm_relay_contract::{RelayDelegationClaims, RunGrantClaims};
use jsonwebtoken::jwk::{Jwk, JwkSet};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use ring::digest::{digest, SHA256};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::api::ReserveRequest;
use crate::store::AuthorityError;

#[derive(Debug, Clone)]
pub struct GrantVerifier {
    control_plane_jwks: JwkSet,
}

impl GrantVerifier {
    pub fn from_control_plane_jwks_json(value: &str) -> Result<Self, AuthorityError> {
        let control_plane_jwks = serde_json::from_str::<JwkSet>(value)
            .map_err(|err| AuthorityError::Store(format!("invalid control-plane JWKS: {err}")))?;
        Ok(Self { control_plane_jwks })
    }

    pub fn verify_reserve_request(
        &self,
        mut request: ReserveRequest,
    ) -> Result<ReserveRequest, AuthorityError> {
        let delegation_jws = request
            .delegation_jws
            .as_deref()
            .ok_or_else(|| AuthorityError::Unauthorized("missing delegation JWS".to_string()))?;
        let grant_jws = request
            .grant_jws
            .as_deref()
            .ok_or_else(|| AuthorityError::Unauthorized("missing grant JWS".to_string()))?;
        let delegation =
            verify_jws::<RelayDelegationClaims>(delegation_jws, &self.control_plane_jwks)?;
        let daemon_jwk_value = delegation.daemon_public_key_jwk.as_ref().ok_or_else(|| {
            AuthorityError::Unauthorized("delegation is missing daemon public key".to_string())
        })?;
        reject_private_jwk_material(daemon_jwk_value)?;
        let actual_thumbprint = jwk_thumbprint(daemon_jwk_value)?;
        if actual_thumbprint != delegation.daemon_public_key_thumbprint {
            return Err(AuthorityError::Unauthorized(
                "daemon public key thumbprint mismatch".to_string(),
            ));
        }
        let daemon_jwk = serde_json::from_value::<Jwk>(daemon_jwk_value.clone())
            .map_err(|err| AuthorityError::Unauthorized(format!("invalid daemon JWK: {err}")))?;
        let grant = verify_jws::<RunGrantClaims>(
            grant_jws,
            &JwkSet {
                keys: vec![daemon_jwk],
            },
        )?;
        request.delegation = delegation;
        request.grant = grant;
        Ok(request)
    }
}

fn verify_jws<T>(compact: &str, keys: &JwkSet) -> Result<T, AuthorityError>
where
    T: DeserializeOwned,
{
    let header = decode_header(compact)
        .map_err(|err| AuthorityError::Unauthorized(format!("invalid JWS header: {err}")))?;
    if !matches!(header.alg, Algorithm::ES256 | Algorithm::EdDSA) {
        return Err(AuthorityError::Unauthorized(
            "JWS alg must be ES256 or EdDSA".to_string(),
        ));
    }
    let key = select_jwk(keys, header.kid.as_deref())?;
    let decoding_key = DecodingKey::from_jwk(key)
        .map_err(|err| AuthorityError::Unauthorized(format!("invalid JWK: {err}")))?;
    let mut validation = Validation::new(header.alg);
    validation.required_spec_claims.clear();
    validation.validate_exp = false;
    validation.validate_aud = false;
    decode::<T>(compact, &decoding_key, &validation)
        .map(|data| data.claims)
        .map_err(|err| AuthorityError::Unauthorized(format!("invalid JWS signature: {err}")))
}

fn select_jwk<'a>(keys: &'a JwkSet, kid: Option<&str>) -> Result<&'a Jwk, AuthorityError> {
    if let Some(kid) = kid {
        return keys
            .find(kid)
            .ok_or_else(|| AuthorityError::Unauthorized("JWS signing key is missing".to_string()));
    }
    if keys.keys.len() == 1 {
        return Ok(&keys.keys[0]);
    }
    Err(AuthorityError::Unauthorized(
        "JWS signing key is ambiguous".to_string(),
    ))
}

fn reject_private_jwk_material(value: &Value) -> Result<(), AuthorityError> {
    let object = value
        .as_object()
        .ok_or_else(|| AuthorityError::Unauthorized("daemon JWK must be an object".to_string()))?;
    if object.contains_key("d") {
        return Err(AuthorityError::Unauthorized(
            "daemon JWK must not include private key material".to_string(),
        ));
    }
    Ok(())
}

fn jwk_thumbprint(value: &Value) -> Result<String, AuthorityError> {
    let object = value
        .as_object()
        .ok_or_else(|| AuthorityError::Unauthorized("daemon JWK must be an object".to_string()))?;
    let kty = required_string(object.get("kty"), "kty")?;
    let crv = required_string(object.get("crv"), "crv")?;
    let canonical = match (kty, crv) {
        ("EC", "P-256") => {
            let x = required_string(object.get("x"), "x")?;
            let y = required_string(object.get("y"), "y")?;
            format!(r#"{{"crv":"{crv}","kty":"{kty}","x":"{x}","y":"{y}"}}"#)
        }
        ("OKP", "Ed25519") => {
            let x = required_string(object.get("x"), "x")?;
            format!(r#"{{"crv":"{crv}","kty":"{kty}","x":"{x}"}}"#)
        }
        _ => {
            return Err(AuthorityError::Unauthorized(
                "unsupported daemon public key JWK".to_string(),
            ));
        }
    };
    let digest = digest(&SHA256, canonical.as_bytes());
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest.as_ref()))
}

fn required_string<'a>(value: Option<&'a Value>, field: &str) -> Result<&'a str, AuthorityError> {
    value.and_then(Value::as_str).ok_or_else(|| {
        AuthorityError::Unauthorized(format!("daemon JWK field {field} must be a string"))
    })
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use chrono::{Duration, Utc};
    use ctx_llm_relay_contract::{
        AccessContextKind, ProviderModelRef, RouteAuthMethod, RouteType, CONTROL_PLANE_ISSUER,
        RELAY_AUDIENCE,
    };
    use ring::rand::SystemRandom;
    use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
    use serde::Serialize;
    use serde_json::json;

    use super::*;

    #[test]
    fn verifies_signed_delegation_and_daemon_grant() {
        let rng = SystemRandom::new();
        let control = test_key_pair("control_key_1", &rng);
        let daemon = test_key_pair("daemon_key_1", &rng);
        let mut delegation = base_delegation();
        delegation.daemon_public_key_jwk = Some(daemon.public_jwk.clone());
        delegation.daemon_public_key_thumbprint =
            jwk_thumbprint(&daemon.public_jwk).expect("thumbprint");
        let grant = base_grant();
        let request = ReserveRequest {
            delegation: base_delegation(),
            grant: base_grant(),
            delegation_jws: Some(sign_es256(
                &delegation,
                &control.key_pair,
                "control_key_1",
                &rng,
            )),
            grant_jws: Some(sign_es256(&grant, &daemon.key_pair, "daemon_key_1", &rng)),
            estimated_input_tokens: Some(128),
            estimated_output_tokens: Some(64),
            idempotency_key: None,
        };
        let verifier = GrantVerifier::from_control_plane_jwks_json(
            &json!({ "keys": [control.public_jwk] }).to_string(),
        )
        .expect("verifier");

        let verified = verifier
            .verify_reserve_request(request)
            .expect("verified request");

        assert_eq!(
            verified.delegation.daemon_public_key_jwk,
            Some(daemon.public_jwk)
        );
        assert_eq!(verified.grant.jti, "grant_1");
    }

    #[test]
    fn rejects_missing_signed_chain() {
        let verifier =
            GrantVerifier::from_control_plane_jwks_json(r#"{"keys":[]}"#).expect("verifier");
        let request = ReserveRequest {
            delegation: base_delegation(),
            grant: base_grant(),
            delegation_jws: None,
            grant_jws: None,
            estimated_input_tokens: None,
            estimated_output_tokens: None,
            idempotency_key: None,
        };

        let err = verifier
            .verify_reserve_request(request)
            .expect_err("missing JWS should reject");

        assert!(matches!(err, AuthorityError::Unauthorized(_)));
    }

    struct TestKeyPair {
        key_pair: EcdsaKeyPair,
        public_jwk: Value,
    }

    fn test_key_pair(kid: &str, rng: &SystemRandom) -> TestKeyPair {
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, rng)
            .expect("generate key");
        let key_pair =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), rng)
                .expect("parse key");
        let public_key = key_pair.public_key().as_ref();
        assert_eq!(public_key[0], 4);
        let x = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&public_key[1..33]);
        let y = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&public_key[33..65]);
        TestKeyPair {
            key_pair,
            public_jwk: json!({
                "kty": "EC",
                "crv": "P-256",
                "x": x,
                "y": y,
                "alg": "ES256",
                "use": "sig",
                "kid": kid,
            }),
        }
    }

    fn sign_es256<T: Serialize>(
        claims: &T,
        key_pair: &EcdsaKeyPair,
        kid: &str,
        rng: &SystemRandom,
    ) -> String {
        let header = base64_json(&json!({ "alg": "ES256", "typ": "JWT", "kid": kid }));
        let payload = base64_json(claims);
        let signing_input = format!("{header}.{payload}");
        let signature = key_pair
            .sign(rng, signing_input.as_bytes())
            .expect("sign JWS");
        format!(
            "{signing_input}.{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature.as_ref())
        )
    }

    fn base64_json<T: Serialize>(value: &T) -> String {
        let bytes = serde_json::to_vec(value).expect("serialize JSON");
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    }

    fn base_delegation() -> RelayDelegationClaims {
        let issued_at = Utc::now() - Duration::seconds(10);
        RelayDelegationClaims {
            jti: "delegation_1".to_string(),
            access_context_kind: AccessContextKind::Org,
            billing_subject_id: "bill_org_1".to_string(),
            ctx_user_id: "user_1".to_string(),
            ctx_account_id: Some("account_1".to_string()),
            ctx_org_id: Some("org_1".to_string()),
            ctx_membership_id: Some("membership_1".to_string()),
            daemon_id: "daemon_1".to_string(),
            daemon_public_key_thumbprint: "thumb_1".to_string(),
            daemon_public_key_jwk: None,
            allowed_route_ids: vec!["route_ctx".to_string()],
            allowed_provider_model_pairs: vec![ProviderModelRef {
                provider_id: "openai".to_string(),
                model_id: "gpt-5".to_string(),
            }],
            allowed_auth_methods: vec![RouteAuthMethod::CtxProviderKey],
            policy_version: "policy_v1".to_string(),
            pricing_version: "pricing_v1".to_string(),
            max_per_request_cents: 500,
            max_input_tokens: 32_000,
            max_output_tokens: 4_096,
            max_concurrent_requests: Some(4),
            issued_at,
            expires_at: issued_at + Duration::minutes(5),
            issuer: CONTROL_PLANE_ISSUER.to_string(),
            audience: RELAY_AUDIENCE.to_string(),
        }
    }

    fn base_grant() -> RunGrantClaims {
        let issued_at = Utc::now() - Duration::seconds(5);
        RunGrantClaims {
            jti: "grant_1".to_string(),
            request_id: "request_1".to_string(),
            access_context_kind: AccessContextKind::Org,
            billing_subject_id: "bill_org_1".to_string(),
            ctx_user_id: "user_1".to_string(),
            ctx_account_id: Some("account_1".to_string()),
            ctx_org_id: Some("org_1".to_string()),
            ctx_membership_id: Some("membership_1".to_string()),
            daemon_id: "daemon_1".to_string(),
            workspace_id: Some("workspace_1".to_string()),
            task_id: Some("task_1".to_string()),
            session_id: Some("session_1".to_string()),
            run_id: Some("run_1".to_string()),
            turn_id: Some("turn_1".to_string()),
            route_id: "route_ctx".to_string(),
            route_type: RouteType::CtxManaged,
            provider_id: "openai".to_string(),
            model_id: "gpt-5".to_string(),
            policy_version: "policy_v1".to_string(),
            pricing_version: "pricing_v1".to_string(),
            max_estimated_cents: 100,
            max_input_tokens: Some(4_000),
            max_output_tokens: Some(1_000),
            delegation_jti: "delegation_1".to_string(),
            delegation_hash: "hash".to_string(),
            issued_at,
            expires_at: issued_at + Duration::minutes(2),
            issuer: "daemon_1".to_string(),
            audience: RELAY_AUDIENCE.to_string(),
        }
    }
}
