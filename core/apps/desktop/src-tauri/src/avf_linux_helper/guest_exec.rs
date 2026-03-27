use super::*;

pub(super) struct GuestExecCaptureResult {
    pub(super) exit_code: i32,
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
}

enum GuestExecTerminalFrame {
    Exit(AvfLinuxExecExit),
    Error(AvfLinuxExecError),
}

pub(super) fn guest_directory_exists(data_root: &Path, guest_path: &Path) -> Result<bool> {
    let result = run_guest_exec_capture(
        &shared_vm_control_socket_path(data_root),
        Path::new("/"),
        "/usr/bin/test",
        &[String::from("-d"), guest_path.display().to_string()],
        Some("root"),
        HashMap::new(),
        None,
    )
    .with_context(|| {
        format!(
            "checking whether guest path {} exists",
            guest_path.display()
        )
    })?;
    Ok(result.exit_code == 0)
}

pub(super) fn guest_exec(
    data_root: &Path,
    workspace_id: &str,
    worktree_id: &str,
    cwd: &Path,
    command: &str,
    env: &[String],
    user: Option<&str>,
    pty: bool,
    args: &[String],
) -> Result<i32> {
    let shared_vm = shared_vm_state(data_root)?;
    if !matches!(shared_vm.state, AvfLinuxSharedVmLifecycleState::Running) {
        bail!(
            "shared AVF Linux VM must be running before guest exec (state={:?})",
            shared_vm.state
        );
    }

    let metadata_path = shared_vm_worktree_metadata_path(data_root, workspace_id, worktree_id);
    let Some(worktree) = load_guest_worktree_state(&metadata_path)? else {
        bail!(
            "guest worktree metadata is missing for workspace {} worktree {}",
            workspace_id,
            worktree_id
        );
    };
    if !cwd.starts_with(&worktree.guest_root) {
        bail!(
            "guest exec cwd {} must stay under guest worktree root {}",
            cwd.display(),
            worktree.guest_root.display()
        );
    }

    let control_socket = shared_vm_control_socket_path(data_root);
    let guest_env = parse_guest_exec_env(env)?;
    let guest_user = user
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            if worktree.guest_user.trim().is_empty() {
                Some(guest_workspace_user(workspace_id))
            } else {
                Some(worktree.guest_user.clone())
            }
        });
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    let mut stdout = stdout.lock();
    let mut stderr = stderr.lock();
    run_guest_exec_cli(
        &control_socket,
        cwd,
        command,
        args,
        guest_user.as_deref(),
        guest_env,
        pty,
        &mut stdout,
        &mut stderr,
    )
    .with_context(|| {
        format!(
            "running AVF Linux guest exec for workspace {} worktree {}",
            workspace_id, worktree_id
        )
    })
}

pub(super) fn shared_vm_exec(
    data_root: &Path,
    cwd: &Path,
    command: &str,
    env: &[String],
    user: Option<&str>,
    pty: bool,
    args: &[String],
) -> Result<i32> {
    let shared_vm = shared_vm_state(data_root)?;
    if !matches!(shared_vm.state, AvfLinuxSharedVmLifecycleState::Running) {
        bail!(
            "shared AVF Linux VM must be running before shared-vm-exec (state={:?})",
            shared_vm.state
        );
    }

    let control_socket = shared_vm_control_socket_path(data_root);
    let guest_env = parse_guest_exec_env(env)?;
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    let mut stdout = stdout.lock();
    let mut stderr = stderr.lock();
    run_guest_exec_cli(
        &control_socket,
        cwd,
        command,
        args,
        user,
        guest_env,
        pty,
        &mut stdout,
        &mut stderr,
    )
    .with_context(|| format!("running shared AVF Linux guest exec `{command}`"))
}

pub(super) fn parse_guest_exec_env(env: &[String]) -> Result<HashMap<String, String>> {
    let mut parsed = HashMap::new();
    for entry in env {
        let Some((key, value)) = entry.split_once('=') else {
            bail!("guest exec env entry must be KEY=VALUE, got `{entry}`");
        };
        let key = key.trim();
        if key.is_empty() {
            bail!("guest exec env key must not be empty");
        }
        if key.starts_with("CTX_AVF_") {
            bail!("guest exec env key `{key}` is reserved for helper control state");
        }
        parsed.insert(key.to_string(), value.to_string());
    }
    Ok(parsed)
}

pub(super) fn guest_workspace_user(workspace_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(workspace_id.as_bytes());
    let digest = hex::encode(hasher.finalize());
    format!("{GUEST_WORKSPACE_USER_PREFIX}{}", &digest[..12])
}

pub(super) fn guest_workspace_home(guest_user: &str) -> PathBuf {
    PathBuf::from(GUEST_WORKSPACE_HOMES_ROOT).join(guest_user)
}

pub(super) fn ensure_guest_workspace_user(data_root: &Path, guest_user: &str) -> Result<()> {
    let control_socket = shared_vm_control_socket_path(data_root);
    ensure_guest_exec_success(
        "creating guest workspace account roots",
        run_guest_exec_capture(
            &control_socket,
            Path::new("/"),
            "/bin/mkdir",
            &[
                String::from("-p"),
                GUEST_WORKSPACE_HOMES_ROOT.to_string(),
                GUEST_WORKSPACE_CACHE_ROOT.to_string(),
                GUEST_WORKSPACE_TMP_ROOT.to_string(),
            ],
            None,
            HashMap::new(),
            None,
        )?,
    )?;

    let user_exists = run_guest_exec_capture(
        &control_socket,
        Path::new("/"),
        "/usr/bin/id",
        &[String::from("-u"), guest_user.to_string()],
        None,
        HashMap::new(),
        None,
    )?
    .exit_code
        == 0;
    if !user_exists {
        ensure_guest_exec_success(
            &format!("creating guest workspace user {guest_user}"),
            run_guest_exec_capture(
                &control_socket,
                Path::new("/"),
                "/usr/sbin/useradd",
                &[
                    String::from("--create-home"),
                    String::from("--home-dir"),
                    guest_workspace_home(guest_user).display().to_string(),
                    String::from("--shell"),
                    String::from("/bin/bash"),
                    guest_user.to_string(),
                ],
                None,
                HashMap::new(),
                None,
            )?,
        )?;
    }

    ensure_guest_exec_success(
        &format!("ensuring guest workspace directories for {guest_user}"),
        run_guest_exec_capture(
            &control_socket,
            Path::new("/"),
            "/usr/bin/install",
            &[
                String::from("-d"),
                String::from("-o"),
                guest_user.to_string(),
                String::from("-g"),
                guest_user.to_string(),
                String::from("-m"),
                String::from("700"),
                guest_workspace_home(guest_user).display().to_string(),
                PathBuf::from(GUEST_WORKSPACE_CACHE_ROOT)
                    .join(guest_user)
                    .display()
                    .to_string(),
                PathBuf::from(GUEST_WORKSPACE_TMP_ROOT)
                    .join(guest_user)
                    .display()
                    .to_string(),
            ],
            None,
            HashMap::new(),
            None,
        )?,
    )
}

pub(super) fn finalize_guest_worktree_permissions(
    data_root: &Path,
    guest_root: &Path,
    guest_user: &str,
) -> Result<()> {
    let control_socket = shared_vm_control_socket_path(data_root);
    ensure_guest_exec_success(
        &format!(
            "setting guest worktree ownership on {}",
            guest_root.display()
        ),
        run_guest_exec_capture(
            &control_socket,
            Path::new("/"),
            "/bin/chown",
            &[
                String::from("-R"),
                format!("{guest_user}:{guest_user}"),
                guest_root.display().to_string(),
            ],
            None,
            HashMap::new(),
            None,
        )?,
    )?;
    ensure_guest_exec_success(
        &format!("restricting guest worktree root {}", guest_root.display()),
        run_guest_exec_capture(
            &control_socket,
            Path::new("/"),
            "/bin/chmod",
            &[String::from("700"), guest_root.display().to_string()],
            None,
            HashMap::new(),
            None,
        )?,
    )
}

#[cfg(unix)]
pub(super) fn run_guest_exec_cli(
    control_socket: &Path,
    cwd: &Path,
    command: &str,
    args: &[String],
    user: Option<&str>,
    env: HashMap<String, String>,
    pty: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<i32> {
    if pty {
        return run_guest_exec_process(control_socket, cwd, command, args, user, env, true);
    }

    let stdin = std::io::stdin();
    let stdin_reader = if unsafe { libc::isatty(stdin.as_raw_fd()) } == 1 {
        None
    } else {
        Some(Box::new(stdin) as Box<dyn Read + Send>)
    };
    run_guest_exec_cli_with_streaming_stdin(
        control_socket,
        cwd,
        command,
        args,
        user,
        env,
        stdin_reader,
        stdout,
        stderr,
    )
}

#[cfg(unix)]
pub(super) fn run_guest_exec_cli_with_streaming_stdin(
    control_socket: &Path,
    cwd: &Path,
    command: &str,
    args: &[String],
    user: Option<&str>,
    env: HashMap<String, String>,
    stdin_reader: Option<Box<dyn Read + Send>>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<i32> {
    if command.trim().is_empty() {
        bail!("guest exec command must not be empty");
    }
    if cwd.as_os_str().is_empty() {
        bail!("guest exec cwd must not be empty");
    }

    let mut stream = connect_shared_vm_control_socket(control_socket)?;
    let request = AvfLinuxExecRequest::new(
        command,
        args.to_vec(),
        cwd.display().to_string(),
        user.map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        env,
        false,
    );
    write_exec_frame(&mut stream, &AvfLinuxExecFrame::Request(request))
        .context("writing AVF Linux guest exec request")?;

    let writer =
        Arc::new(Mutex::new(stream.try_clone().context(
            "cloning shared VM control stream for stdin forwarding",
        )?));
    let _stdin_forwarder = if let Some(mut reader) = stdin_reader {
        let writer = Arc::clone(&writer);
        Some(std::thread::spawn(move || {
            let mut buf = [0u8; AVF_EXEC_STREAM_FRAME_MAX_PAYLOAD];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        let Ok(mut guard) = writer.lock() else {
                            return;
                        };
                        let _ = write_exec_frame(&mut *guard, &AvfLinuxExecFrame::CloseStdin);
                        return;
                    }
                    Ok(n) => {
                        let Ok(mut guard) = writer.lock() else {
                            return;
                        };
                        if write_exec_frame(
                            &mut *guard,
                            &AvfLinuxExecFrame::Stdin(buf[..n].to_vec()),
                        )
                        .is_err()
                        {
                            return;
                        }
                    }
                    Err(_) => {
                        let Ok(mut guard) = writer.lock() else {
                            return;
                        };
                        let _ = write_exec_frame(&mut *guard, &AvfLinuxExecFrame::CloseStdin);
                        return;
                    }
                }
            }
        }))
    } else {
        write_exec_frame(
            &mut *writer
                .lock()
                .map_err(|_| anyhow::anyhow!("guest exec writer mutex poisoned"))?,
            &AvfLinuxExecFrame::CloseStdin,
        )
        .context("closing AVF Linux guest exec stdin")?;
        None
    };

    loop {
        match read_exec_frame(&mut stream).context("reading AVF Linux guest exec response")? {
            Some(AvfLinuxExecFrame::Stdout(bytes)) => {
                stdout
                    .write_all(&bytes)
                    .and_then(|_| stdout.flush())
                    .context("writing guest stdout")?;
            }
            Some(AvfLinuxExecFrame::Stderr(bytes)) => {
                stderr
                    .write_all(&bytes)
                    .and_then(|_| stderr.flush())
                    .context("writing guest stderr")?;
            }
            Some(AvfLinuxExecFrame::Exit(AvfLinuxExecExit { exit_code })) => return Ok(exit_code),
            Some(AvfLinuxExecFrame::Error(AvfLinuxExecError { code, message })) => {
                bail!("{code}: {message}");
            }
            Some(other) => {
                bail!("received unexpected AVF Linux exec frame: {other:?}");
            }
            None => {
                bail!(
                    "shared VM control socket {} closed before sending an exit frame",
                    control_socket.display()
                );
            }
        }
    }
}

#[cfg(all(unix, test))]
pub(super) fn run_guest_exec_cli_with_capture_stdin(
    control_socket: &Path,
    cwd: &Path,
    command: &str,
    args: &[String],
    user: Option<&str>,
    env: HashMap<String, String>,
    stdin_reader: Option<&mut dyn Read>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<i32> {
    let result =
        run_guest_exec_capture(control_socket, cwd, command, args, user, env, stdin_reader)?;
    stdout
        .write_all(&result.stdout)
        .and_then(|_| stdout.flush())
        .context("writing captured guest stdout")?;
    stderr
        .write_all(&result.stderr)
        .and_then(|_| stderr.flush())
        .context("writing captured guest stderr")?;
    Ok(result.exit_code)
}

#[cfg(not(unix))]
pub(super) fn run_guest_exec_cli(
    _control_socket: &Path,
    _cwd: &Path,
    _command: &str,
    _args: &[String],
    _user: Option<&str>,
    _env: HashMap<String, String>,
    _pty: bool,
    _stdout: &mut dyn Write,
    _stderr: &mut dyn Write,
) -> Result<i32> {
    bail!("AVF Linux guest exec relay requires unix domain sockets")
}

#[cfg(unix)]
fn configure_shared_vm_control_stream_timeout(
    stream: &UnixStream,
    timeout: Option<Duration>,
) -> Result<()> {
    stream
        .set_read_timeout(timeout)
        .context("configuring shared VM control stream read timeout")?;
    stream
        .set_write_timeout(timeout)
        .context("configuring shared VM control stream write timeout")?;
    Ok(())
}

#[cfg(unix)]
fn collect_guest_exec_capture_response(
    response_stream: &mut impl Read,
) -> Result<(GuestExecTerminalFrame, Vec<u8>, Vec<u8>)> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    loop {
        match read_exec_frame(response_stream)
            .context("reading AVF Linux guest exec capture frame")?
        {
            Some(AvfLinuxExecFrame::Stdout(bytes)) => stdout.extend_from_slice(&bytes),
            Some(AvfLinuxExecFrame::Stderr(bytes)) => stderr.extend_from_slice(&bytes),
            Some(AvfLinuxExecFrame::Exit(exit)) => {
                return Ok((GuestExecTerminalFrame::Exit(exit), stdout, stderr));
            }
            Some(AvfLinuxExecFrame::Error(error)) => {
                return Ok((GuestExecTerminalFrame::Error(error), stdout, stderr));
            }
            Some(
                AvfLinuxExecFrame::Request(_)
                | AvfLinuxExecFrame::Stdin(_)
                | AvfLinuxExecFrame::CloseStdin
                | AvfLinuxExecFrame::Resize(_),
            ) => bail!("received unexpected frame while waiting for guest exec result"),
            None => bail!("shared VM control socket closed before guest exec exit"),
        }
    }
}

#[cfg(unix)]
fn finish_guest_exec_capture_result(
    terminal: GuestExecTerminalFrame,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
) -> Result<GuestExecCaptureResult> {
    match terminal {
        GuestExecTerminalFrame::Exit(exit) => Ok(GuestExecCaptureResult {
            exit_code: exit.exit_code,
            stdout,
            stderr,
        }),
        GuestExecTerminalFrame::Error(error) => {
            let stderr_text = String::from_utf8_lossy(&stderr).trim().to_string();
            let stdout_text = String::from_utf8_lossy(&stdout).trim().to_string();
            let extra = [stderr_text, stdout_text]
                .into_iter()
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            if extra.is_empty() {
                bail!("guest exec failed: {} ({})", error.message, error.code);
            }
            bail!(
                "guest exec failed: {} ({})\n{}",
                error.message,
                error.code,
                extra
            );
        }
    }
}

#[cfg(unix)]
pub(super) fn run_guest_exec_capture_over_connected_stream(
    stream: &mut (impl Read + Write),
    cwd: &Path,
    command: &str,
    args: &[String],
    user: Option<&str>,
    env: HashMap<String, String>,
) -> Result<GuestExecCaptureResult> {
    if command.trim().is_empty() {
        bail!("guest exec command must not be empty");
    }
    if cwd.as_os_str().is_empty() {
        bail!("guest exec cwd must not be empty");
    }

    let request = AvfLinuxExecRequest::new(
        command,
        args.to_vec(),
        cwd.display().to_string(),
        user.map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        env,
        false,
    );
    write_exec_frame(stream, &AvfLinuxExecFrame::Request(request))
        .context("writing AVF Linux guest exec capture request")?;
    write_exec_frame(stream, &AvfLinuxExecFrame::CloseStdin)
        .context("closing AVF Linux guest exec stdin")?;

    let (terminal, stdout, stderr) = collect_guest_exec_capture_response(stream)?;
    finish_guest_exec_capture_result(terminal, stdout, stderr)
}

#[cfg(unix)]
pub(super) fn run_guest_exec_capture(
    control_socket: &Path,
    cwd: &Path,
    command: &str,
    args: &[String],
    user: Option<&str>,
    env: HashMap<String, String>,
    stdin_reader: Option<&mut dyn Read>,
) -> Result<GuestExecCaptureResult> {
    run_guest_exec_capture_with_socket_timeout(
        control_socket,
        cwd,
        command,
        args,
        user,
        env,
        stdin_reader,
        None,
    )
}

#[cfg(unix)]
pub(super) fn run_guest_exec_capture_with_socket_timeout(
    control_socket: &Path,
    cwd: &Path,
    command: &str,
    args: &[String],
    user: Option<&str>,
    env: HashMap<String, String>,
    stdin_reader: Option<&mut dyn Read>,
    socket_timeout: Option<Duration>,
) -> Result<GuestExecCaptureResult> {
    if command.trim().is_empty() {
        bail!("guest exec command must not be empty");
    }
    if cwd.as_os_str().is_empty() {
        bail!("guest exec cwd must not be empty");
    }

    let mut stream = connect_shared_vm_control_socket(control_socket)?;
    configure_shared_vm_control_stream_timeout(&stream, socket_timeout)?;
    let request = AvfLinuxExecRequest::new(
        command,
        args.to_vec(),
        cwd.display().to_string(),
        user.map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        env,
        false,
    );
    write_exec_frame(&mut stream, &AvfLinuxExecFrame::Request(request))
        .context("writing AVF Linux guest exec capture request")?;

    let mut response_stream = stream
        .try_clone()
        .context("cloning shared VM control stream for capture response")?;
    configure_shared_vm_control_stream_timeout(&response_stream, socket_timeout)?;
    let response_thread = std::thread::spawn(move || -> Result<_> {
        collect_guest_exec_capture_response(&mut response_stream)
    });

    let mut stdin_error = None;
    if let Some(reader) = stdin_reader {
        let mut buf = [0u8; AVF_EXEC_STREAM_FRAME_MAX_PAYLOAD];
        loop {
            let read = reader
                .read(&mut buf)
                .context("reading staged guest exec stdin")?;
            if read == 0 {
                break;
            }
            if let Err(err) =
                write_exec_frame(&mut stream, &AvfLinuxExecFrame::Stdin(buf[..read].to_vec()))
                    .context("writing AVF Linux guest exec stdin frame")
            {
                stdin_error = Some(err);
                break;
            }
        }
    }
    if stdin_error.is_none() {
        if let Err(err) = write_exec_frame(&mut stream, &AvfLinuxExecFrame::CloseStdin)
            .context("closing AVF Linux guest exec stdin")
        {
            stdin_error = Some(err);
        }
    }

    let (terminal, stdout, stderr) = response_thread
        .join()
        .map_err(|_| anyhow::anyhow!("guest exec response reader thread panicked"))??;

    if let Some(err) = stdin_error {
        if !is_ignorable_guest_exec_stdin_write_error(&err) {
            return Err(err);
        }
    }

    finish_guest_exec_capture_result(terminal, stdout, stderr)
}

#[cfg(not(unix))]
pub(super) fn run_guest_exec_capture(
    _control_socket: &Path,
    _cwd: &Path,
    _command: &str,
    _args: &[String],
    _user: Option<&str>,
    _env: HashMap<String, String>,
    _stdin_reader: Option<&mut dyn Read>,
) -> Result<GuestExecCaptureResult> {
    bail!("programmatic AVF guest exec capture requires unix domain sockets")
}

pub(super) fn format_guest_exec_output(output: &[u8]) -> String {
    String::from_utf8_lossy(output).trim().to_string()
}

fn is_ignorable_guest_exec_stdin_write_error(err: &anyhow::Error) -> bool {
    err.chain()
        .find_map(|cause| cause.downcast_ref::<std::io::Error>())
        .is_some_and(|io_err| {
            matches!(
                io_err.kind(),
                std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
            )
        })
}

pub(super) fn ensure_guest_exec_success(
    context: &str,
    result: GuestExecCaptureResult,
) -> Result<()> {
    if result.exit_code == 0 {
        return Ok(());
    }
    let stdout = format_guest_exec_output(&result.stdout);
    let stderr = format_guest_exec_output(&result.stderr);
    let joined = [stderr, stdout]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if joined.is_empty() {
        bail!("{context} failed with exit code {}", result.exit_code);
    }
    bail!(
        "{context} failed with exit code {}:\n{}",
        result.exit_code,
        joined
    );
}

#[cfg(unix)]
pub(super) fn connect_shared_vm_control_socket(socket_path: &Path) -> Result<UnixStream> {
    let deadline = std::time::Instant::now() + GUEST_EXEC_CONNECT_TIMEOUT;
    loop {
        match UnixStream::connect(socket_path) {
            Ok(stream) => return Ok(stream),
            Err(err)
                if err.kind() == std::io::ErrorKind::NotFound
                    || err.kind() == std::io::ErrorKind::ConnectionRefused =>
            {
                if std::time::Instant::now() >= deadline {
                    return Err(err).with_context(|| {
                        format!(
                            "connecting to shared VM control socket {}",
                            socket_path.display()
                        )
                    });
                }
                std::thread::sleep(GUEST_EXEC_CONNECT_RETRY_INTERVAL);
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "connecting to shared VM control socket {}",
                        socket_path.display()
                    )
                });
            }
        }
    }
}

#[cfg(unix)]
pub(super) fn connect_guest_agent_control_socket(socket_path: &Path) -> Result<UnixStream> {
    let deadline = std::time::Instant::now() + GUEST_EXEC_CONNECT_TIMEOUT;
    loop {
        match UnixStream::connect(socket_path) {
            Ok(stream) => return Ok(stream),
            Err(err)
                if err.kind() == std::io::ErrorKind::NotFound
                    || err.kind() == std::io::ErrorKind::ConnectionRefused =>
            {
                if std::time::Instant::now() >= deadline {
                    return Err(err).with_context(|| {
                        format!(
                            "connecting to guest-agent control socket {}",
                            socket_path.display()
                        )
                    });
                }
                std::thread::sleep(GUEST_EXEC_CONNECT_RETRY_INTERVAL);
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "connecting to guest-agent control socket {}",
                        socket_path.display()
                    )
                });
            }
        }
    }
}

#[cfg(unix)]
pub(super) fn run_guest_exec_process(
    control_socket: &Path,
    cwd: &Path,
    command: &str,
    args: &[String],
    user: Option<&str>,
    env: HashMap<String, String>,
    pty: bool,
) -> Result<i32> {
    if command.trim().is_empty() {
        bail!("guest exec command must not be empty");
    }
    if cwd.as_os_str().is_empty() {
        bail!("guest exec cwd must not be empty");
    }

    let mut stream = connect_shared_vm_control_socket(control_socket)?;
    let request = AvfLinuxExecRequest::new(
        command,
        args.to_vec(),
        cwd.display().to_string(),
        user.map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        env,
        pty,
    );
    write_exec_frame(&mut stream, &AvfLinuxExecFrame::Request(request))
        .context("writing AVF Linux guest exec request")?;

    let writer =
        Arc::new(Mutex::new(stream.try_clone().context(
            "cloning shared VM control stream for stdin forwarding",
        )?));
    if pty {
        spawn_terminal_resize_forwarder(Arc::clone(&writer));
    }
    let stdin_writer = Arc::clone(&writer);
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let mut buf = [0u8; AVF_EXEC_STREAM_FRAME_MAX_PAYLOAD];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => {
                    let Ok(mut guard) = stdin_writer.lock() else {
                        return;
                    };
                    let _ = write_exec_frame(&mut *guard, &AvfLinuxExecFrame::CloseStdin);
                    return;
                }
                Ok(n) => {
                    let Ok(mut guard) = stdin_writer.lock() else {
                        return;
                    };
                    if write_exec_frame(&mut *guard, &AvfLinuxExecFrame::Stdin(buf[..n].to_vec()))
                        .is_err()
                    {
                        return;
                    }
                }
                Err(_) => {
                    let Ok(mut guard) = stdin_writer.lock() else {
                        return;
                    };
                    let _ = write_exec_frame(&mut *guard, &AvfLinuxExecFrame::CloseStdin);
                    return;
                }
            }
        }
    });

    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    loop {
        match read_exec_frame(&mut stream).context("reading AVF Linux guest exec response")? {
            Some(AvfLinuxExecFrame::Stdout(bytes)) => {
                stdout
                    .write_all(&bytes)
                    .and_then(|_| stdout.flush())
                    .context("writing guest stdout")?;
            }
            Some(AvfLinuxExecFrame::Stderr(bytes)) => {
                stderr
                    .write_all(&bytes)
                    .and_then(|_| stderr.flush())
                    .context("writing guest stderr")?;
            }
            Some(AvfLinuxExecFrame::Exit(AvfLinuxExecExit { exit_code })) => return Ok(exit_code),
            Some(AvfLinuxExecFrame::Error(AvfLinuxExecError { code, message })) => {
                bail!("{code}: {message}");
            }
            Some(other) => {
                bail!("received unexpected AVF Linux exec frame: {other:?}");
            }
            None => {
                bail!(
                    "shared VM control socket {} closed before sending an exit frame",
                    control_socket.display()
                );
            }
        }
    }
}

#[cfg(unix)]
pub(super) fn spawn_terminal_resize_forwarder(writer: Arc<Mutex<UnixStream>>) {
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let fd = stdin.as_raw_fd();
        let mut last_size = None;
        loop {
            let Some((cols, rows)) = current_terminal_size(fd) else {
                return;
            };
            if last_size != Some((cols, rows)) {
                let Ok(mut guard) = writer.lock() else {
                    return;
                };
                if write_exec_frame(
                    &mut *guard,
                    &AvfLinuxExecFrame::Resize(AvfLinuxExecResize { cols, rows }),
                )
                .is_err()
                {
                    return;
                }
                last_size = Some((cols, rows));
            }
            std::thread::sleep(GUEST_EXEC_TTY_RESIZE_POLL_INTERVAL);
        }
    });
}

#[cfg(unix)]
pub(super) fn current_terminal_size(fd: std::os::fd::RawFd) -> Option<(u16, u16)> {
    unsafe {
        if libc::isatty(fd) != 1 {
            return None;
        }
        let mut winsize = libc::winsize {
            ws_row: 0,
            ws_col: 0,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        if libc::ioctl(fd, libc::TIOCGWINSZ, &mut winsize) != 0 {
            return None;
        }
        if winsize.ws_col == 0 || winsize.ws_row == 0 {
            return None;
        }
        Some((winsize.ws_col, winsize.ws_row))
    }
}

#[cfg(not(unix))]
pub(super) fn run_guest_exec_process(
    _control_socket: &Path,
    _cwd: &Path,
    _command: &str,
    _args: &[String],
    _user: Option<&str>,
    _env: HashMap<String, String>,
    _pty: bool,
) -> Result<i32> {
    bail!("AVF Linux guest exec relay requires unix domain sockets")
}
