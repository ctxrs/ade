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
        "workspace_active_snapshot_http",
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
        "provider_scenarios_offline",
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
        "message_idempotency",
        "noisy_output_backpressure",
        "terminal_workspace_stream_separation",
        "terminal_ws_reconnect",
        "turn_lifecycle_events",
        "turn_terminal_reconciliation",
    ],
    "artifacts-updates": [
        "attachments_demo_react",
        "global_id_routing_http",
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

CTX_HTTP_BAZEL_MANUAL_ONLY_TARGET = "manual-only"

def _as_label(name):
    return ":" + name

def _declare_ctx_http_test(name, common_srcs, compile_data, data, deps, proc_macro_deps, rustc_env, test_args):
    tags = ["manual"] if name in CTX_HTTP_MANUAL_ONLY_TESTS else []
    rust_test(
        name = name,
        crate_name = name,
        crate_root = "tests/{}.rs".format(name),
        srcs = ["tests/{}.rs".format(name)] + common_srcs,
        args = test_args,
        compile_data = compile_data,
        data = data,
        edition = "2021",
        rustc_env = rustc_env,
        deps = deps,
        proc_macro_deps = proc_macro_deps,
        tags = tags,
    )

def declare_ctx_http_integration_tests(common_srcs, compile_data, data, deps, proc_macro_deps, rustc_env, test_args):
    declared = {}
    for suite_name in CTX_HTTP_SUITE_ORDER:
        test_labels = []
        for test_name in CTX_HTTP_SUITE_TESTS[suite_name]:
            if test_name not in declared:
                _declare_ctx_http_test(
                    name = test_name,
                    common_srcs = common_srcs,
                    compile_data = compile_data,
                    data = data,
                    deps = deps,
                    proc_macro_deps = proc_macro_deps,
                    rustc_env = rustc_env,
                    test_args = test_args,
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
                common_srcs = common_srcs,
                compile_data = compile_data,
                data = data,
                deps = deps,
                proc_macro_deps = proc_macro_deps,
                rustc_env = rustc_env,
                test_args = test_args,
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
