use super::*;

pub(crate) async fn get_codex_accounts_usage(
    State(providers): State<ProvidersHandle>,
    Query(query): Query<ProviderUsageQuery>,
) -> Result<Json<CodexAccountsUsageResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let refresh = query.refresh.unwrap_or(false);
    let entries = providers
        .load_codex_accounts_usage(refresh)
        .await
        .map_err(internal_error)?
        .into_iter()
        .map(|entry| CodexAccountUsageEntry {
            account_id: entry.account_id,
            label: entry.label,
            email: entry.email,
            plan_type: entry.plan_type,
            last_used_at: entry.last_used_at,
            usage: entry.usage,
        })
        .collect();
    Ok(Json(CodexAccountsUsageResponse { entries }))
}
