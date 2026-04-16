load("@rules_rust//rust:defs.bzl", "rust_test")

CTX_HTTP_SUITE_ORDER = [
    "workspace-stream",
    "provider-auth",
    "provider-runtime",
    "repo-vcs",
    "lsp",
    "turns-terminal",
    "artifacts-updates",
    "sandbox-cloud",
]

CTX_HTTP_SUITE_TESTS = {
    "workspace-stream": [
        "cache_rehydration",
        "fault_matrix",
        "hot_endpoints_no_db",
        "replay_properties",
        "workspace_active_snapshot_http_workspace_active_hydration_returns_500_for_store_open_failures_and_404_for_missing_workspaces",
        "workspace_active_snapshot_http_workspace_active_snapshot_includes_sessions",
        "workspace_active_snapshot_http_create_session_rejects_initial_prompt_without_client_ids",
        "workspace_active_snapshot_http_workspace_active_snapshot_includes_worktree_vcs_for_active_tasks_only",
        "workspace_active_snapshot_http_workspace_active_heads_batch_strips_partials",
        "workspace_active_snapshot_http_session_snapshot_returns_summary_only",
        "workspace_active_snapshot_http_session_head_returns_head",
        "workspace_active_snapshot_http_workspace_stream_replays_from_after_seq",
        "workspace_active_snapshot_http_workspace_stream_reset_replay_waits_for_fresh_resume_cursor",
        "workspace_active_snapshot_http_workspace_stream_replays_tool_events",
        "workspace_active_snapshot_http_workspace_stream_under_load_no_gap_or_reset",
        "workspace_active_snapshot_http_workspace_stream_emits_git_status_snapshot_on_change",
        "workspace_active_snapshot_http_workspace_stream_emits_git_status_snapshot_for_new_subscriber",
        "workspace_active_snapshot_http_workspace_stream_delivers_snapshot_before_worktree_vcs_summary_refresh",
        "workspace_active_snapshot_http_workspace_stream_repeat_subscribe_preserves_ready_worktree_vcs_state",
        "workspace_active_snapshot_http_workspace_stream_replay_only_subscribe_reseeds_cached_worktree_vcs_snapshot",
        "workspace_active_snapshot_http_mobile_secure_workspace_stream_replay_only_subscribe_reseeds_cached_worktree_vcs_snapshot",
        "workspace_active_snapshot_http_workspace_stream_repeat_subscribe_rescans_fresh_unavailable_worktree_vcs",
        "workspace_active_snapshot_http_workspace_stream_initial_snapshot_includes_worktree_vcs_for_explicit_archived_session",
        "workspace_active_snapshot_http_workspace_stream_initial_snapshot_preserves_secondary_worktree_vcs_for_active_task",
        "workspace_active_snapshot_http_workspace_stream_active_subscribe_keeps_secondary_worktree_vcs_publishable",
        "workspace_active_snapshot_http_mobile_secure_workspace_stream_active_subscribe_keeps_secondary_worktree_vcs_publishable",
        "workspace_active_snapshot_http_workspace_stream_emits_worktree_vcs_snapshot_on_activation",
        "workspace_active_snapshot_http_workspace_stream_emits_gap_on_large_replay",
        "workspace_active_snapshot_http_workspace_active_snapshot_stream_pushes_updates",
        "workspace_active_snapshot_http_workspace_stream_archived_task_upsert_has_no_snapshot_payload",
        "workspace_active_snapshot_http_workspace_active_snapshot_stream_filters_session_head_deltas",
        "workspace_stream_context_window_metrics",
        "workspace_stream_no_gaps_under_activity",
        "workspace_stream_stress_active_heads_lag",
    ],
    "provider-auth": [
        "acp_target_scoped_status",
        "codex_host_import_api",
        "codex_login_callback_api",
        "install_start_contract",
        "provider_target_scoped_installs",
        "subscription_accounts_api",
    ],
    "provider-runtime": [
        "acp_crp_bridge_tokens_e2e",
        "gemini_live_model_catalog",
        "live_provider_canary",
        "provider_probe_runtime_env",
        "provider_worker_reaping_offline",
        "provider_scenarios_offline_crp_fixtures",
        "provider_scenarios_offline_interleaved_assistant_tools_do_not_fragment_messages",
        "provider_scenarios_offline_crp_fixtures_persist_context_window_metrics",
        "session_model_api",
        "workspace_provider_model_preferences_http",
    ],
    "repo-vcs": [
        "jj_merge_queue_basics",
        "merge_queue_isolation",
        "repo_clone_branch_and_safety",
        "repo_init_initial_commit",
        "repo_validate_destination",
        "session_diff_unavailable",
        "workspace_merge_queue_config_http",
        "worktree_archive_http",
        "worktree_vcs_snapshot",
    ],
    "lsp": [
        "buffers_http_e2e",
        "lsp_catalog_http_e2e",
        "lsp_edit_plans_http_e2e",
        "lsp_http_e2e",
    ],
    "turns-terminal": [
        "assistant_chunk_stream_only",
        "assistant_message_persistence_faults",
        "demo_seed_transcript_http",
        "message_idempotency_post_message_idempotent_same_payload",
        "message_idempotency_post_message_idempotent_conflict_on_change",
        "noisy_output_backpressure",
        "terminal_workspace_stream_separation",
        "terminal_ws_reconnect",
        "turn_lifecycle_events",
        "turn_terminal_reconciliation",
    ],
    "artifacts-updates": [
        "attachments_demo_react",
        "global_id_routing_http_artifact_route_is_session_scoped",
        "global_id_routing_http_quicktime_artifact_upload_is_accepted",
        "global_id_routing_http_message_delete_route_is_session_scoped",
        "global_id_routing_http_subagent_invocation_route_is_session_scoped",
        "image_attachments_http_e2e",
        "openai_responses_sse_stub",
        "oracle_mcp_http",
        "release_manifest_corpus",
        "storage_guard_api",
        "subagent_mcp_http",
        "system_prompt_append_http",
        "title_generation_local",
        "updates_failure_safety_checksum_mismatch",
        "updates_failure_safety_interrupted_transfer",
        "updates_failure_safety_manifest_parse",
        "updates_failure_safety_missing_artifact",
        "workspace_attachments_local_canonical",
    ],
    "sandbox-cloud": [
        "disk_isolated_sandbox_smoke",
        "disk_isolated_vcs_integrity",
        "harness_container_sandbox_e2e",
        "memory_leak_e2e",
        "resource_governance_systemd_e2e",
        "title_generation_local_e2e",
        "workspace_runtime_crash_recovery",
    ],
}

CTX_HTTP_MANUAL_ONLY_TESTS = [
    "cloud_gateway_azure_e2e",
    "cloud_gateway_gcp_e2e",
]

CTX_HTTP_CUSTOM_INTEGRATION_TARGETS = {
    "workspace_active_snapshot_http_workspace_active_hydration_returns_500_for_store_open_failures_and_404_for_missing_workspaces": {
        "source": "workspace_active_snapshot_http",
        "args": ["--exact", "workspace_active_hydration_returns_500_for_store_open_failures_and_404_for_missing_workspaces"],
        "timeout": "long",
    },
    "workspace_active_snapshot_http_workspace_active_snapshot_includes_sessions": {
        "source": "workspace_active_snapshot_http",
        "args": ["--exact", "workspace_active_snapshot_includes_sessions"],
        "timeout": "long",
    },
    "workspace_active_snapshot_http_create_session_rejects_initial_prompt_without_client_ids": {
        "source": "workspace_active_snapshot_http",
        "args": ["--exact", "create_session_rejects_initial_prompt_without_client_ids"],
        "timeout": "long",
    },
    "workspace_active_snapshot_http_workspace_active_snapshot_includes_worktree_vcs_for_active_tasks_only": {
        "source": "workspace_active_snapshot_http",
        "args": ["--exact", "workspace_active_snapshot_includes_worktree_vcs_for_active_tasks_only"],
        "timeout": "long",
    },
    "workspace_active_snapshot_http_workspace_active_heads_batch_strips_partials": {
        "source": "workspace_active_snapshot_http",
        "args": ["--exact", "workspace_active_heads_batch_strips_partials"],
        "timeout": "long",
    },
    "workspace_active_snapshot_http_session_snapshot_returns_summary_only": {
        "source": "workspace_active_snapshot_http",
        "args": ["--exact", "session_snapshot_returns_summary_only"],
        "timeout": "long",
    },
    "workspace_active_snapshot_http_session_head_returns_head": {
        "source": "workspace_active_snapshot_http",
        "args": ["--exact", "session_head_returns_head"],
        "timeout": "long",
    },
    "workspace_active_snapshot_http_workspace_stream_replays_from_after_seq": {
        "source": "workspace_active_snapshot_http",
        "args": ["--exact", "workspace_stream_replays_from_after_seq"],
        "timeout": "long",
    },
    "workspace_active_snapshot_http_workspace_stream_reset_replay_waits_for_fresh_resume_cursor": {
        "source": "workspace_active_snapshot_http",
        "args": ["--exact", "workspace_stream_reset_replay_waits_for_fresh_resume_cursor"],
        "timeout": "long",
    },
    "workspace_active_snapshot_http_workspace_stream_replays_tool_events": {
        "source": "workspace_active_snapshot_http",
        "args": ["--exact", "workspace_stream_replays_tool_events"],
        "timeout": "long",
    },
    "workspace_active_snapshot_http_workspace_stream_under_load_no_gap_or_reset": {
        "source": "workspace_active_snapshot_http",
        "args": ["--exact", "workspace_stream_under_load_no_gap_or_reset"],
        "timeout": "long",
    },
    "workspace_active_snapshot_http_workspace_stream_emits_git_status_snapshot_on_change": {
        "source": "workspace_active_snapshot_http",
        "args": ["--exact", "workspace_stream_emits_git_status_snapshot_on_change"],
        "timeout": "long",
    },
    "workspace_active_snapshot_http_workspace_stream_emits_git_status_snapshot_for_new_subscriber": {
        "source": "workspace_active_snapshot_http",
        "args": ["--exact", "workspace_stream_emits_git_status_snapshot_for_new_subscriber"],
        "timeout": "long",
    },
    "workspace_active_snapshot_http_workspace_stream_delivers_snapshot_before_worktree_vcs_summary_refresh": {
        "source": "workspace_active_snapshot_http",
        "args": ["--exact", "workspace_stream_delivers_snapshot_before_worktree_vcs_summary_refresh"],
        "timeout": "long",
    },
    "workspace_active_snapshot_http_workspace_stream_repeat_subscribe_preserves_ready_worktree_vcs_state": {
        "source": "workspace_active_snapshot_http",
        "args": ["--exact", "workspace_stream_repeat_subscribe_preserves_ready_worktree_vcs_state"],
        "timeout": "long",
    },
    "workspace_active_snapshot_http_workspace_stream_replay_only_subscribe_reseeds_cached_worktree_vcs_snapshot": {
        "source": "workspace_active_snapshot_http",
        "args": ["--exact", "workspace_stream_replay_only_subscribe_reseeds_cached_worktree_vcs_snapshot"],
        "timeout": "long",
    },
    "workspace_active_snapshot_http_mobile_secure_workspace_stream_replay_only_subscribe_reseeds_cached_worktree_vcs_snapshot": {
        "source": "workspace_active_snapshot_http",
        "args": ["--exact", "mobile_secure_workspace_stream_replay_only_subscribe_reseeds_cached_worktree_vcs_snapshot"],
        "timeout": "long",
    },
    "workspace_active_snapshot_http_workspace_stream_repeat_subscribe_rescans_fresh_unavailable_worktree_vcs": {
        "source": "workspace_active_snapshot_http",
        "args": ["--exact", "workspace_stream_repeat_subscribe_rescans_fresh_unavailable_worktree_vcs"],
        "timeout": "long",
    },
    "workspace_active_snapshot_http_workspace_stream_initial_snapshot_includes_worktree_vcs_for_explicit_archived_session": {
        "source": "workspace_active_snapshot_http",
        "args": ["--exact", "workspace_stream_initial_snapshot_includes_worktree_vcs_for_explicit_archived_session"],
        "timeout": "long",
    },
    "workspace_active_snapshot_http_workspace_stream_initial_snapshot_preserves_secondary_worktree_vcs_for_active_task": {
        "source": "workspace_active_snapshot_http",
        "args": ["--exact", "workspace_stream_initial_snapshot_preserves_secondary_worktree_vcs_for_active_task"],
        "timeout": "long",
    },
    "workspace_active_snapshot_http_workspace_stream_active_subscribe_keeps_secondary_worktree_vcs_publishable": {
        "source": "workspace_active_snapshot_http",
        "args": ["--exact", "workspace_stream_active_subscribe_keeps_secondary_worktree_vcs_publishable"],
        "timeout": "long",
    },
    "workspace_active_snapshot_http_mobile_secure_workspace_stream_active_subscribe_keeps_secondary_worktree_vcs_publishable": {
        "source": "workspace_active_snapshot_http",
        "args": ["--exact", "mobile_secure_workspace_stream_active_subscribe_keeps_secondary_worktree_vcs_publishable"],
        "timeout": "long",
    },
    "workspace_active_snapshot_http_workspace_stream_emits_worktree_vcs_snapshot_on_activation": {
        "source": "workspace_active_snapshot_http",
        "args": ["--exact", "workspace_stream_emits_worktree_vcs_snapshot_on_activation"],
        "timeout": "long",
    },
    "workspace_active_snapshot_http_workspace_stream_emits_gap_on_large_replay": {
        "source": "workspace_active_snapshot_http",
        "args": ["--exact", "workspace_stream_emits_gap_on_large_replay"],
        "timeout": "long",
    },
    "workspace_active_snapshot_http_workspace_active_snapshot_stream_pushes_updates": {
        "source": "workspace_active_snapshot_http",
        "args": ["--exact", "workspace_active_snapshot_stream_pushes_updates"],
        "timeout": "long",
    },
    "workspace_active_snapshot_http_workspace_stream_archived_task_upsert_has_no_snapshot_payload": {
        "source": "workspace_active_snapshot_http",
        "args": ["--exact", "workspace_stream_archived_task_upsert_has_no_snapshot_payload"],
        "timeout": "long",
    },
    "workspace_active_snapshot_http_workspace_active_snapshot_stream_filters_session_head_deltas": {
        "source": "workspace_active_snapshot_http",
        "args": ["--exact", "workspace_active_snapshot_stream_filters_session_head_deltas"],
        "timeout": "long",
    },
    "message_idempotency_post_message_idempotent_same_payload": {
        "source": "message_idempotency",
        "args": ["--exact", "post_message_idempotent_same_payload"],
        "timeout": "long",
    },
    "message_idempotency_post_message_idempotent_conflict_on_change": {
        "source": "message_idempotency",
        "args": ["--exact", "post_message_idempotent_conflict_on_change"],
        "timeout": "long",
    },
    "global_id_routing_http_artifact_route_is_session_scoped": {
        "source": "global_id_routing_http",
        "args": ["--exact", "artifact_route_is_session_scoped"],
        "timeout": "long",
    },
    "global_id_routing_http_quicktime_artifact_upload_is_accepted": {
        "source": "global_id_routing_http",
        "args": ["--exact", "quicktime_artifact_upload_is_accepted"],
        "timeout": "long",
    },
    "global_id_routing_http_message_delete_route_is_session_scoped": {
        "source": "global_id_routing_http",
        "args": ["--exact", "message_delete_route_is_session_scoped"],
        "timeout": "long",
    },
    "global_id_routing_http_subagent_invocation_route_is_session_scoped": {
        "source": "global_id_routing_http",
        "args": ["--exact", "subagent_invocation_route_is_session_scoped"],
        "timeout": "long",
    },
    "workspace_stream_no_gaps_under_activity": {
        "source": "workspace_stream_no_gaps_under_activity",
        "args": [],
        "timeout": "long",
    },
    "provider_scenarios_offline_crp_fixtures": {
        "source": "provider_scenarios_offline",
        "args": ["--exact", "provider_scenarios_offline_crp_fixtures"],
    },
    "provider_scenarios_offline_interleaved_assistant_tools_do_not_fragment_messages": {
        "source": "provider_scenarios_offline",
        "args": ["--exact", "provider_scenarios_offline_interleaved_assistant_tools_do_not_fragment_messages"],
    },
    "provider_scenarios_offline_crp_fixtures_persist_context_window_metrics": {
        "source": "provider_scenarios_offline",
        "args": ["--exact", "provider_scenarios_offline_crp_fixtures_persist_context_window_metrics"],
    },
}

CTX_HTTP_BAZEL_MANUAL_ONLY_TARGET = "manual-only"

def _as_label(name):
    return ":" + name

def _integration_rustc_env(rustc_env):
    merged = {}
    for key, value in rustc_env.items():
        merged[key] = value
    merged["CARGO_BIN_EXE_ctx"] = "$(rootpath :ctx)"
    merged["CARGO_BIN_EXE_ctx-http-lsp-test-server"] = "$(rootpath :ctx-http-lsp-test-server)"
    merged["CARGO_BIN_EXE_llama_server_mock"] = "$(rootpath :llama_server_mock)"
    return merged

def _declare_ctx_http_test(name, source_name, common_srcs, compile_data, data, deps, proc_macro_deps, rustc_env, test_args, timeout = None):
    tags = ["manual"] if name in CTX_HTTP_MANUAL_ONLY_TESTS else []
    kwargs = {}
    if timeout != None:
        kwargs["timeout"] = timeout
    rust_test(
        name = name,
        crate_name = name,
        crate_root = "tests/{}.rs".format(source_name),
        srcs = ["tests/{}.rs".format(source_name)] + common_srcs,
        args = test_args,
        compile_data = compile_data,
        data = data,
        edition = "2021",
        rustc_env = _integration_rustc_env(rustc_env),
        deps = deps,
        proc_macro_deps = proc_macro_deps,
        tags = tags,
        **kwargs
    )

def declare_ctx_http_rust_unit_test(name, args, compile_data, data, deps, proc_macro_deps, timeout = None):
    kwargs = {}
    if timeout != None:
        kwargs["timeout"] = timeout
    rust_test(
        name = name,
        crate = ":lib_test_support",
        args = args,
        compile_data = compile_data,
        data = data,
        deps = deps,
        proc_macro_deps = proc_macro_deps,
        **kwargs
    )

def declare_ctx_http_integration_tests(common_srcs, compile_data, data, deps, proc_macro_deps, rustc_env, test_args):
    declared = {}
    for suite_name in CTX_HTTP_SUITE_ORDER:
        test_labels = []
        for test_name in CTX_HTTP_SUITE_TESTS[suite_name]:
            if test_name not in declared:
                custom = CTX_HTTP_CUSTOM_INTEGRATION_TARGETS.get(test_name)
                _declare_ctx_http_test(
                    name = test_name,
                    source_name = custom["source"] if custom else test_name,
                    common_srcs = common_srcs,
                    compile_data = compile_data,
                    data = data,
                    deps = deps,
                    proc_macro_deps = proc_macro_deps,
                    rustc_env = rustc_env,
                    test_args = test_args + custom["args"] if custom else test_args,
                    timeout = custom.get("timeout") if custom else None,
                )
                declared[test_name] = True
            test_labels.append(_as_label(test_name))
        native.test_suite(
            name = suite_name,
            tests = test_labels,
        )

    manual_test_labels = []
    for test_name in CTX_HTTP_MANUAL_ONLY_TESTS:
        if test_name not in declared:
            _declare_ctx_http_test(
                name = test_name,
                source_name = test_name,
                common_srcs = common_srcs,
                compile_data = compile_data,
                data = data,
                deps = deps,
                proc_macro_deps = proc_macro_deps,
                rustc_env = rustc_env,
                test_args = test_args,
                timeout = None,
            )
            declared[test_name] = True
        manual_test_labels.append(_as_label(test_name))

    native.test_suite(
        name = CTX_HTTP_BAZEL_MANUAL_ONLY_TARGET,
        tags = ["manual"],
        tests = manual_test_labels,
    )

    native.test_suite(
        name = "all",
        tests = [_as_label("base")] + [_as_label(suite_name) for suite_name in CTX_HTTP_SUITE_ORDER],
    )
