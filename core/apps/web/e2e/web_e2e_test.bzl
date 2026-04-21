load("@rules_shell//shell:sh_test.bzl", "sh_test")

def web_e2e_test(name, config, runtime_profile, suite = "", specs = None, timeout = "long"):
    resolved_specs = specs or []
    args = [
        "--config",
        config,
        "--runtime-profile",
        runtime_profile,
        "--ctx-http-bin",
        "$(location //core/crates/ctx-http:ctx)",
    ]
    data = [
        "//core/apps/web:e2e_runtime_data",
        "//core/crates/ctx-http:ctx",
    ]
    if runtime_profile == "agent-full":
        args.extend([
            "--ctx-mcp-bin",
            "$(location //core/crates/ctx-mcp:ctx-mcp)",
        ])
        data.append("//core/crates/ctx-mcp:ctx-mcp")
    if suite:
        args.extend(["--suite", suite])
    for spec in resolved_specs:
        args.extend(["--spec", spec])
    sh_test(
        name = name,
        srcs = ["//core/apps/web:scripts/run-e2e-bazel-runtime.sh"],
        args = args,
        data = data,
        tags = [
            "local",
            "no-remote",
        ],
        timeout = timeout,
    )
