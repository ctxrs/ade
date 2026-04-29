export interface Env {
  ENVIRONMENT?: string;
  GRANT_VERIFICATION_MODE?: "signed_chain" | "hmac" | "test_unsigned";
  CONTROL_PLANE_JWKS?: string;
  GRANT_HMAC_SECRET?: string;
  AUTHORITY_BASE_URL?: string;
  AUTHORITY_BEARER_TOKEN?: string;
  OPENAI_RESPONSES_URL?: string;
  OPENAI_API_KEY?: string;
}

export interface ExecutionContextLike {
  waitUntil(promise: Promise<unknown>): void;
}

export interface RelayDelegation {
  jti: string;
  access_context_kind: "personal" | "org";
  billing_subject_id: string;
  ctx_user_id: string;
  ctx_account_id?: string;
  ctx_org_id?: string;
  ctx_membership_id?: string;
  daemon_id: string;
  daemon_public_key_thumbprint: string;
  daemon_public_key_jwk?: JsonWebKey;
  allowed_route_ids: string[];
  allowed_provider_model_pairs: Array<{ provider_id: string; model_id: string }>;
  allowed_auth_methods: string[];
  policy_version: string;
  pricing_version: string;
  max_per_request_cents: number;
  max_input_tokens: number;
  max_output_tokens: number;
  issued_at: string;
  expires_at: string;
  issuer: string;
  audience: string;
}

export interface RunGrant {
  jti: string;
  request_id: string;
  access_context_kind: "personal" | "org";
  billing_subject_id: string;
  ctx_user_id: string;
  ctx_account_id?: string;
  ctx_org_id?: string;
  ctx_membership_id?: string;
  daemon_id: string;
  route_id: string;
  route_type: "ctx_managed";
  provider_id: string;
  model_id: string;
  policy_version: string;
  pricing_version: string;
  max_estimated_cents: number;
  max_input_tokens?: number;
  max_output_tokens?: number;
  delegation_jti: string;
  delegation_hash: string;
  issued_at: string;
  expires_at: string;
  issuer: string;
  audience: string;
}

export interface ValidatedResponsesRequest {
  model: string;
  maxOutputTokens: number;
  estimatedInputBytes: number;
}

export interface AuthorityReserveResponse {
  request_id: string;
  reservation_id: string;
  state: string;
  reserved_cents: number;
  idempotent: boolean;
}
