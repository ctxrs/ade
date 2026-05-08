use super::*;

#[test]
fn collect_ctx_managed_sandbox_helper_pids_matches_only_ctx_scoped_helpers() {
    let temp = tempfile::tempdir().expect("tempdir");
    let machine_name = sandbox_machine_name(temp.path());
    let helper_dir = temp
        .path()
        .join("managed")
        .join("runtimes")
        .join("sandbox-cli")
        .join("macos")
        .join("aarch64")
        .join("sandbox-cli-5.8.0")
        .join("usr")
        .join("libexec")
        .join("sandbox-cli");
    let matches = collect_ctx_managed_sandbox_helper_pids(
        vec![
            (
                42,
                vec![
                    helper_dir.join("gvproxy").to_string_lossy().into_owned(),
                    machine_name.clone(),
                    sandbox_machine_temp_root(temp.path())
                        .join("sandbox-cli")
                        .join(format!("{machine_name}-api.sock"))
                        .to_string_lossy()
                        .into_owned(),
                ],
            ),
            (
                77,
                vec![
                    "/opt/homebrew/bin/vfkit".to_string(),
                    temp.path()
                        .join("sandbox-cli")
                        .join("xdg")
                        .join("data")
                        .join("containers")
                        .join("sandbox-cli")
                        .join("machine")
                        .join("applehv")
                        .join(format!("{machine_name}-arm64.raw"))
                        .to_string_lossy()
                        .into_owned(),
                    machine_name.clone(),
                ],
            ),
            (
                88,
                vec![
                    "/opt/homebrew/libexec/sandbox-cli/gvproxy".to_string(),
                    "/tmp/sandbox-cli/sandbox-machine-default-api.sock".to_string(),
                    "sandbox-machine-default".to_string(),
                ],
            ),
        ],
        temp.path(),
        &machine_name,
    );
    assert_eq!(matches, vec![42, 77]);
}

#[test]
fn ctx_managed_sandbox_helper_process_detection_matches_expected_shapes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let machine_name = sandbox_machine_name(temp.path());
    let matching_gvproxy = vec![
        temp.path()
            .join("managed")
            .join("runtimes")
            .join("sandbox-cli")
            .join("macos")
            .join("aarch64")
            .join("sandbox-cli-5.8.0")
            .join("usr")
            .join("libexec")
            .join("sandbox-cli")
            .join("gvproxy")
            .to_string_lossy()
            .into_owned(),
        sandbox_machine_temp_root(temp.path())
            .join("sandbox-cli")
            .join(format!("{machine_name}-api.sock"))
            .to_string_lossy()
            .into_owned(),
        machine_name.clone(),
    ];
    assert!(is_ctx_managed_sandbox_helper_process_command(
        &matching_gvproxy,
        temp.path(),
        &machine_name
    ));

    let matching_vfkit = vec![
        String::from("/opt/homebrew/bin/vfkit"),
        temp.path()
            .join("sandbox-cli")
            .join("xdg")
            .join("data")
            .join("containers")
            .join("sandbox-cli")
            .join("machine")
            .join("applehv")
            .join(format!("{machine_name}-arm64.raw"))
            .to_string_lossy()
            .into_owned(),
        machine_name.clone(),
    ];
    assert!(is_ctx_managed_sandbox_helper_process_command(
        &matching_vfkit,
        temp.path(),
        &machine_name
    ));

    let wrong_machine = vec![
        String::from("/opt/homebrew/bin/vfkit"),
        temp.path()
            .join("sandbox-cli")
            .join("xdg")
            .join("data")
            .join("containers")
            .join("sandbox-cli")
            .join("machine")
            .join("applehv")
            .join("ctx-someone-else-arm64.raw")
            .to_string_lossy()
            .into_owned(),
    ];
    assert!(!is_ctx_managed_sandbox_helper_process_command(
        &wrong_machine,
        temp.path(),
        &machine_name
    ));

    let host_helper = vec![
        String::from("/opt/homebrew/libexec/sandbox-cli/gvproxy"),
        String::from("/tmp/sandbox-cli/sandbox-machine-default-api.sock"),
        String::from("sandbox-machine-default"),
    ];
    assert!(!is_ctx_managed_sandbox_helper_process_command(
        &host_helper,
        temp.path(),
        &machine_name
    ));
}

#[test]
fn collect_ctx_managed_sandbox_helper_pids_from_ps_output_matches_real_macos_shapes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let machine_name = sandbox_machine_name(temp.path());
    let helper_dir = temp
        .path()
        .join("managed")
        .join("runtimes")
        .join("sandbox-cli")
        .join("macos")
        .join("aarch64")
        .join("sandbox-cli-5.8.0")
        .join("usr")
        .join("libexec")
        .join("sandbox-cli");
    let gvproxy_line = format!(
            " 6622 {} -mtu 1500 -listen-vfkit unixgram://{} -forward-sock {} -forward-identity {} -pid-file {}/gvproxy.pid",
            helper_dir.join("gvproxy").display(),
            sandbox_machine_temp_root(temp.path())
                .join("sandbox-cli")
                .join(format!("{machine_name}-gvproxy.sock"))
                .display(),
            sandbox_machine_temp_root(temp.path())
                .join("sandbox-cli")
                .join(format!("{machine_name}-api.sock"))
                .display(),
            temp.path()
                .join("sandbox-cli")
                .join("xdg")
                .join("data")
                .join("containers")
                .join("sandbox-cli")
                .join("machine")
                .join("machine")
                .display(),
            sandbox_machine_temp_root(temp.path()).join("sandbox-cli").display(),
        );
    let vfkit_line = format!(
            "12484 /Users/example-user/Library/Application Support/vfkit --device virtio-blk,path={} --device virtio-vsock,port=1025,socketURL={} --device virtio-net,unixSocketPath={}",
            temp.path()
                .join("sandbox-cli")
                .join("xdg")
                .join("data")
                .join("containers")
                .join("sandbox-cli")
                .join("machine")
                .join("applehv")
                .join(format!("{machine_name}-arm64.raw"))
                .display(),
            sandbox_machine_temp_root(temp.path())
                .join("sandbox-cli")
                .join(format!("{machine_name}.sock"))
                .display(),
            sandbox_machine_temp_root(temp.path())
                .join("sandbox-cli")
                .join(format!("{machine_name}-gvproxy.sock"))
                .display(),
        );
    let host_line = String::from(
            "88 /opt/homebrew/libexec/sandbox-cli/gvproxy -forward-sock /tmp/sandbox-cli/sandbox-machine-default-api.sock sandbox-machine-default",
        );
    let ps_output = format!("{gvproxy_line}\n{vfkit_line}\n{host_line}\n");

    let matches = collect_ctx_managed_sandbox_helper_pids_from_ps_output(
        &ps_output,
        temp.path(),
        &machine_name,
    );
    assert_eq!(matches, vec![6622, 12484]);
}
