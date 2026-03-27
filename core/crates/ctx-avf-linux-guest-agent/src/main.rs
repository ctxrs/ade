mod protocol;

#[cfg(target_os = "linux")]
use std::fs::{self, File};
#[cfg(target_os = "linux")]
use std::io::{Read, Write};
#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::process::{Command, Stdio};
#[cfg(target_os = "linux")]
use std::sync::{Arc, Mutex};

#[cfg(target_os = "linux")]
use anyhow::Context;
use anyhow::{bail, Result};
#[cfg(target_os = "linux")]
use portable_pty::{CommandBuilder as PtyCommandBuilder, NativePtySystem, PtySize, PtySystem};

#[cfg(any(target_os = "linux", test))]
use crate::protocol::AvfLinuxExecFrame;
#[cfg(any(target_os = "linux", test))]
use crate::protocol::{read_exec_frame, write_exec_frame};
#[cfg(target_os = "linux")]
use crate::protocol::{AvfLinuxExecError, AvfLinuxExecExit};
use crate::protocol::{AvfLinuxExecRequest, AVF_LINUX_EXEC_PROTOCOL_VERSION};

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const GUEST_VSOCK_PORT: u32 = 47001;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const DEFAULT_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";
#[cfg(target_os = "linux")]
const VSOCK_LISTENER_RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);
#[cfg(target_os = "linux")]
const GUEST_CONTROL_READY_MARKER_ENV: &str = "CTX_AVF_GUEST_CONTROL_READY_MARKER";
#[cfg(target_os = "linux")]
const DEFAULT_PTY_COLS: u16 = 80;
#[cfg(target_os = "linux")]
const DEFAULT_PTY_ROWS: u16 = 24;
// Keep guest-agent stream chunks aligned with the host helper's empirically safe
// shared-VM transport budget so streamed stdin is not truncated mid-import.
#[cfg(any(target_os = "linux", test))]
const AVF_EXEC_STREAM_FRAME_MAX_PAYLOAD: usize = 1024;

fn main() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        return serve();
    }

    #[cfg(not(target_os = "linux"))]
    {
        bail!("ctx-avf-linux-guest-agent only runs on Linux guests");
    }
}

#[cfg(target_os = "linux")]
fn serve() -> Result<()> {
    let ready_marker = guest_control_ready_marker_path();
    clear_guest_control_ready_marker(ready_marker.as_deref());
    loop {
        clear_guest_control_ready_marker(ready_marker.as_deref());
        let listener = match wait_for_vsock_listener(GUEST_VSOCK_PORT) {
            Ok(listener) => listener,
            Err(err) => {
                eprintln!("guest-agent listener setup failed permanently: {err:#}");
                std::thread::sleep(VSOCK_LISTENER_RETRY_INTERVAL);
                continue;
            }
        };
        announce_guest_control_ready(ready_marker.as_deref(), GUEST_VSOCK_PORT);
        loop {
            match accept_vsock_connection(&listener) {
                Ok(conn) => {
                    std::thread::spawn(move || {
                        if let Err(err) = handle_connection(conn) {
                            eprintln!("guest-agent connection failed: {err:#}");
                        }
                    });
                }
                Err(err) if is_transient_vsock_accept_error(&err) => {
                    eprintln!("guest-agent transient accept error: {err:#}");
                    continue;
                }
                Err(err) => {
                    clear_guest_control_ready_marker(ready_marker.as_deref());
                    eprintln!("guest-agent accept failed, recreating listener: {err:#}");
                    break;
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn guest_control_ready_marker_path() -> Option<PathBuf> {
    std::env::var_os(GUEST_CONTROL_READY_MARKER_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(target_os = "linux")]
fn clear_guest_control_ready_marker(path: Option<&Path>) {
    if let Some(path) = path {
        let _ = fs::remove_file(path);
    }
}

#[cfg(target_os = "linux")]
fn announce_guest_control_ready(path: Option<&Path>, port: u32) {
    if let Some(path) = path {
        if let Some(parent) = path.parent() {
            if let Err(err) = fs::create_dir_all(parent) {
                eprintln!(
                    "guest-agent failed to create ready-marker parent {}: {err}",
                    parent.display()
                );
            }
        }
        if let Err(err) = fs::write(path, format!("listening:{port}\n")) {
            eprintln!(
                "guest-agent failed to publish ready marker {}: {err}",
                path.display()
            );
        }
    }
    eprintln!("guest-agent listening on AF_VSOCK port {port}");
}

#[cfg(target_os = "linux")]
fn bind_vsock_listener(port: u32) -> Result<OwnedFd> {
    let fd = unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        bail!(
            "creating AF_VSOCK listener failed: {}",
            std::io::Error::last_os_error()
        );
    }
    let listener = unsafe { OwnedFd::from_raw_fd(fd) };
    let addr = libc::sockaddr_vm {
        svm_family: libc::AF_VSOCK as libc::sa_family_t,
        svm_reserved1: 0,
        svm_port: port,
        svm_cid: libc::VMADDR_CID_ANY,
        svm_zero: [0; 4],
    };
    let bind_rc = unsafe {
        libc::bind(
            listener.as_raw_fd(),
            (&addr as *const libc::sockaddr_vm).cast(),
            std::mem::size_of::<libc::sockaddr_vm>() as libc::socklen_t,
        )
    };
    if bind_rc != 0 {
        bail!(
            "binding AF_VSOCK listener on port {port} failed: {}",
            std::io::Error::last_os_error()
        );
    }
    let listen_rc = unsafe { libc::listen(listener.as_raw_fd(), 128) };
    if listen_rc != 0 {
        bail!(
            "listening on AF_VSOCK port {port} failed: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(listener)
}

#[cfg(target_os = "linux")]
fn wait_for_vsock_listener(port: u32) -> Result<OwnedFd> {
    loop {
        match bind_vsock_listener(port) {
            Ok(listener) => return Ok(listener),
            Err(err) => {
                eprintln!("guest-agent waiting for AF_VSOCK port {port}: {err:#}");
                std::thread::sleep(VSOCK_LISTENER_RETRY_INTERVAL);
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn accept_vsock_connection(listener: &OwnedFd) -> Result<OwnedFd> {
    let fd = unsafe {
        libc::accept(
            listener.as_raw_fd(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if fd < 0 {
        bail!(
            "accepting AF_VSOCK connection failed: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

#[cfg(target_os = "linux")]
fn is_transient_vsock_accept_error(err: &anyhow::Error) -> bool {
    err.chain()
        .find_map(|cause| cause.downcast_ref::<std::io::Error>())
        .and_then(std::io::Error::raw_os_error)
        .is_some_and(|code| {
            matches!(
                code,
                libc::EINTR | libc::EAGAIN | libc::ECONNABORTED | libc::ECONNRESET
            )
        })
}

#[cfg(target_os = "linux")]
fn handle_connection(conn: OwnedFd) -> Result<()> {
    let mut reader = File::from(conn);
    let request = match read_exec_frame(&mut reader).context("reading exec request")? {
        Some(AvfLinuxExecFrame::Request(request)) => request,
        Some(other) => {
            let writer = Arc::new(Mutex::new(
                reader
                    .try_clone()
                    .context("cloning connection for error reply")?,
            ));
            let _ = write_error_frame(
                &writer,
                "invalid_request",
                &format!("expected request frame first, received {other:?}"),
            );
            return Ok(());
        }
        None => return Ok(()),
    };
    let writer = Arc::new(Mutex::new(
        reader
            .try_clone()
            .context("cloning connection for response stream")?,
    ));

    let prepared = match prepare_exec_request(&request) {
        Ok(prepared) => prepared,
        Err(err) => {
            let _ = write_error_frame(&writer, "prepare_failed", &err.to_string());
            return Ok(());
        }
    };

    if prepared.pty {
        if let Err(err) = handle_pty_connection(reader, prepared) {
            let _ = write_error_frame(&writer, "pty_failed", &err.to_string());
        }
        return Ok(());
    }

    let mut command = Command::new(&prepared.command);
    command
        .args(&prepared.args)
        .current_dir(&prepared.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Err(err) = configure_command_process_group(&mut command) {
        let _ = write_error_frame(&writer, "spawn_setup_failed", &err.to_string());
        return Ok(());
    }
    command.env_clear();
    for (key, value) in &prepared.env {
        command.env(key, value);
    }
    if !prepared.env.contains_key("PATH") {
        command.env("PATH", DEFAULT_PATH);
    }
    if let Some(user) = prepared.user.as_deref() {
        let account = match lookup_user(user) {
            Ok(account) => account,
            Err(err) => {
                let _ = write_error_frame(&writer, "user_lookup_failed", &err.to_string());
                return Ok(());
            }
        };
        if !prepared.env.contains_key("HOME") {
            command.env("HOME", &account.home);
        }
        if !prepared.env.contains_key("USER") {
            command.env("USER", &account.user);
        }
        if !prepared.env.contains_key("LOGNAME") {
            command.env("LOGNAME", &account.user);
        }
        if let Err(err) = configure_command_user(&mut command, &account) {
            let _ = write_error_frame(&writer, "spawn_setup_failed", &err.to_string());
            return Ok(());
        }
    }

    let mut child = match command.spawn().with_context(|| {
        format!(
            "spawning guest command `{}` in {}",
            prepared.command,
            prepared.cwd.display()
        )
    }) {
        Ok(child) => child,
        Err(err) => {
            let _ = write_error_frame(&writer, "spawn_failed", &err.to_string());
            return Ok(());
        }
    };
    let child_pid = child.id();
    let Some(mut child_stdin) = child.stdin.take() else {
        let _ = write_error_frame(&writer, "spawn_failed", "guest command stdin unavailable");
        return Ok(());
    };
    let Some(mut child_stdout) = child.stdout.take() else {
        let _ = write_error_frame(&writer, "spawn_failed", "guest command stdout unavailable");
        return Ok(());
    };
    let Some(mut child_stderr) = child.stderr.take() else {
        let _ = write_error_frame(&writer, "spawn_failed", "guest command stderr unavailable");
        return Ok(());
    };

    let stdout_writer = Arc::clone(&writer);
    let stdout_thread = std::thread::spawn(move || -> Result<()> {
        relay_stream_output(&mut child_stdout, &stdout_writer, true)
    });
    let stderr_writer = Arc::clone(&writer);
    let stderr_thread = std::thread::spawn(move || -> Result<()> {
        relay_stream_output(&mut child_stderr, &stderr_writer, false)
    });

    let input_thread = std::thread::spawn(move || -> Result<()> {
        loop {
            match read_exec_frame(&mut reader) {
                Ok(Some(AvfLinuxExecFrame::Stdin(bytes))) => child_stdin
                    .write_all(&bytes)
                    .and_then(|_| child_stdin.flush())
                    .context("writing guest stdin")?,
                Ok(Some(AvfLinuxExecFrame::CloseStdin)) => {
                    drop(child_stdin);
                    return Ok(());
                }
                Ok(Some(AvfLinuxExecFrame::Resize(_))) => continue,
                Ok(None) => {
                    drop(child_stdin);
                    let _ = terminate_exec_process_group(child_pid);
                    return Ok(());
                }
                Ok(Some(other)) => {
                    let _ = terminate_exec_process_group(child_pid);
                    bail!("unexpected exec frame after request: {other:?}");
                }
                Err(err) => {
                    drop(child_stdin);
                    let _ = terminate_exec_process_group(child_pid);
                    return Err(err).context("reading exec input frame");
                }
            }
        }
    });

    let status = child.wait().context("waiting for guest command")?;
    let exit_code = status.code().unwrap_or(1);
    let input_result = input_thread
        .join()
        .map_err(|_| anyhow::anyhow!("guest exec stdin relay thread panicked"))?;
    let stdout_result = stdout_thread
        .join()
        .map_err(|_| anyhow::anyhow!("guest exec stdout relay thread panicked"))?;
    let stderr_result = stderr_thread
        .join()
        .map_err(|_| anyhow::anyhow!("guest exec stderr relay thread panicked"))?;
    if let Err(err) = &input_result {
        eprintln!(
            "guest-agent stdin relay failed for {:?} in {} as {:?}: {err:#}",
            prepared.command,
            prepared.cwd.display(),
            prepared.user
        );
    }
    if let Err(err) = &stdout_result {
        eprintln!(
            "guest-agent stdout relay failed for {:?} in {} as {:?}: {err:#}",
            prepared.command,
            prepared.cwd.display(),
            prepared.user
        );
    }
    if let Err(err) = &stderr_result {
        eprintln!(
            "guest-agent stderr relay failed for {:?} in {} as {:?}: {err:#}",
            prepared.command,
            prepared.cwd.display(),
            prepared.user
        );
    }
    if exit_code != 0 {
        eprintln!(
            "guest-agent command {:?} in {} as {:?} exited with {}",
            prepared.command,
            prepared.cwd.display(),
            prepared.user,
            exit_code
        );
    }

    write_stream_frame(
        &writer,
        AvfLinuxExecFrame::Exit(AvfLinuxExecExit { exit_code }),
    )
}

#[cfg(target_os = "linux")]
fn relay_stream_output(
    reader: &mut impl Read,
    writer: &Arc<Mutex<File>>,
    stdout: bool,
) -> Result<()> {
    let mut buf = [0u8; AVF_EXEC_STREAM_FRAME_MAX_PAYLOAD];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => emit_exec_stream_frames(&buf[..n], stdout, |frame| {
                write_stream_frame(writer, frame)
            })?,
            Err(err) => return Err(err).context("reading child output"),
        }
    }
}

#[cfg(target_os = "linux")]
fn relay_pty_output(reader: &mut impl Read, writer: &Arc<Mutex<File>>) -> Result<()> {
    let mut buf = [0u8; AVF_EXEC_STREAM_FRAME_MAX_PAYLOAD];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => {
                emit_exec_stream_frames(&buf[..n], true, |frame| write_stream_frame(writer, frame))?
            }
            Err(err) => return Err(err).context("reading PTY output"),
        }
    }
}

#[cfg(any(target_os = "linux", test))]
fn emit_exec_stream_frames<F>(bytes: &[u8], stdout: bool, mut emit: F) -> Result<()>
where
    F: FnMut(AvfLinuxExecFrame) -> Result<()>,
{
    for chunk in bytes.chunks(AVF_EXEC_STREAM_FRAME_MAX_PAYLOAD) {
        let frame = if stdout {
            AvfLinuxExecFrame::Stdout(chunk.to_vec())
        } else {
            AvfLinuxExecFrame::Stderr(chunk.to_vec())
        };
        emit(frame)?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn handle_pty_connection(stream: File, prepared: PreparedExec) -> Result<()> {
    let pty_system = NativePtySystem::default();
    let pair = pty_system
        .openpty(PtySize {
            rows: DEFAULT_PTY_ROWS,
            cols: DEFAULT_PTY_COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("opening guest PTY")?;
    let cmd = build_pty_command(&prepared)?;
    let mut child = pair.slave.spawn_command(cmd).with_context(|| {
        format!(
            "spawning guest PTY command `{}` in {}",
            prepared.command,
            prepared.cwd.display()
        )
    })?;
    drop(pair.slave);

    let mut pty_reader = pair
        .master
        .try_clone_reader()
        .context("cloning guest PTY reader")?;
    let mut pty_writer = pair
        .master
        .take_writer()
        .context("taking guest PTY writer")?;
    let resize_master = Arc::new(Mutex::new(pair.master));
    let writer = Arc::new(Mutex::new(
        stream
            .try_clone()
            .context("cloning connection for PTY output")?,
    ));
    let output_writer = Arc::clone(&writer);
    let output_thread = std::thread::spawn(move || -> Result<()> {
        relay_pty_output(&mut pty_reader, &output_writer)
    });
    let mut stdin_reader = stream;
    let input_thread = std::thread::spawn(move || -> Result<()> {
        loop {
            match read_exec_frame(&mut stdin_reader).context("reading guest PTY input frame")? {
                Some(AvfLinuxExecFrame::Stdin(bytes)) => pty_writer
                    .write_all(&bytes)
                    .and_then(|_| pty_writer.flush())
                    .context("writing guest PTY stdin")?,
                Some(AvfLinuxExecFrame::Resize(resize)) => {
                    let master = resize_master
                        .lock()
                        .map_err(|_| anyhow::anyhow!("guest PTY master mutex poisoned"))?;
                    master
                        .resize(PtySize {
                            rows: resize.rows,
                            cols: resize.cols,
                            pixel_width: 0,
                            pixel_height: 0,
                        })
                        .context("resizing guest PTY")?;
                }
                Some(AvfLinuxExecFrame::CloseStdin) | None => return Ok(()),
                Some(other) => bail!("unexpected PTY frame after request: {other:?}"),
            }
        }
    });

    let exit_code = i32::try_from(
        child
            .wait()
            .context("waiting for guest PTY command")?
            .exit_code(),
    )
    .unwrap_or(1);
    let _ = input_thread.join();
    let _ = output_thread.join();
    write_stream_frame(
        &writer,
        AvfLinuxExecFrame::Exit(AvfLinuxExecExit { exit_code }),
    )
}

#[cfg(target_os = "linux")]
fn write_error_frame(writer: &Arc<Mutex<File>>, code: &str, message: &str) -> Result<()> {
    write_stream_frame(
        writer,
        AvfLinuxExecFrame::Error(AvfLinuxExecError {
            code: code.to_string(),
            message: message.to_string(),
        }),
    )
}

#[cfg(target_os = "linux")]
fn write_stream_frame(writer: &Arc<Mutex<File>>, frame: AvfLinuxExecFrame) -> Result<()> {
    let mut guard = writer
        .lock()
        .map_err(|_| anyhow::anyhow!("guest-agent writer mutex poisoned"))?;
    write_exec_frame(&mut *guard, &frame).context("writing exec frame")
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Debug)]
struct PreparedExec {
    command: String,
    args: Vec<String>,
    cwd: std::path::PathBuf,
    user: Option<String>,
    env: std::collections::HashMap<String, String>,
    pty: bool,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn prepare_exec_request(request: &AvfLinuxExecRequest) -> Result<PreparedExec> {
    if request.protocol_version != AVF_LINUX_EXEC_PROTOCOL_VERSION {
        bail!(
            "unsupported exec protocol version {}, expected {}",
            request.protocol_version,
            AVF_LINUX_EXEC_PROTOCOL_VERSION
        );
    }
    if request.command.trim().is_empty() {
        bail!("guest exec command must not be empty");
    }
    if request.cwd.trim().is_empty() {
        bail!("guest exec cwd must not be empty");
    }

    let user = request
        .user
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    Ok(PreparedExec {
        command: request.command.clone(),
        args: request.args.clone(),
        cwd: request.cwd.clone().into(),
        user,
        env: request.env.clone(),
        pty: request.pty,
    })
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
struct ResolvedUserAccount {
    uid: libc::uid_t,
    gid: libc::gid_t,
    home: String,
    user: String,
}

#[cfg(target_os = "linux")]
fn lookup_user(user: &str) -> Result<ResolvedUserAccount> {
    let c_user =
        std::ffi::CString::new(user).with_context(|| format!("invalid username `{user}`"))?;
    let pwd = unsafe { libc::getpwnam(c_user.as_ptr()) };
    if pwd.is_null() {
        bail!("guest user `{user}` does not exist");
    }
    let pwd = unsafe { &*pwd };
    let home = unsafe { std::ffi::CStr::from_ptr(pwd.pw_dir) }
        .to_string_lossy()
        .to_string();
    Ok(ResolvedUserAccount {
        uid: pwd.pw_uid,
        gid: pwd.pw_gid,
        home,
        user: user.to_string(),
    })
}

#[cfg(target_os = "linux")]
fn build_pty_command(prepared: &PreparedExec) -> Result<PtyCommandBuilder> {
    if let Some(user) = prepared.user.as_deref() {
        let account = lookup_user(user)?;
        let mut env_pairs = prepared.env.clone();
        if !env_pairs.contains_key("PATH") {
            env_pairs.insert("PATH".to_string(), DEFAULT_PATH.to_string());
        }
        if !env_pairs.contains_key("HOME") {
            env_pairs.insert("HOME".to_string(), account.home.clone());
        }
        if !env_pairs.contains_key("USER") {
            env_pairs.insert("USER".to_string(), account.user.clone());
        }
        if !env_pairs.contains_key("LOGNAME") {
            env_pairs.insert("LOGNAME".to_string(), account.user.clone());
        }

        let mut script = format!(
            "cd {} && exec /usr/bin/env -i",
            shell_words::quote(prepared.cwd.to_string_lossy().as_ref())
        );
        let mut env_entries = env_pairs.into_iter().collect::<Vec<_>>();
        env_entries.sort_by(|(left, _), (right, _)| left.cmp(right));
        for (key, value) in env_entries {
            script.push(' ');
            script.push_str(&shell_words::quote(&format!("{key}={value}")));
        }
        script.push(' ');
        script.push_str(&shell_words::quote(&prepared.command));
        for arg in &prepared.args {
            script.push(' ');
            script.push_str(&shell_words::quote(arg));
        }

        let su_path = if std::path::Path::new("/usr/bin/su").exists() {
            "/usr/bin/su"
        } else {
            "/bin/su"
        };
        let mut cmd = PtyCommandBuilder::new(su_path);
        cmd.arg("-s");
        cmd.arg("/bin/sh");
        cmd.arg("-c");
        cmd.arg(script);
        cmd.arg(account.user);
        return Ok(cmd);
    }

    let mut cmd = PtyCommandBuilder::new(prepared.command.clone());
    for arg in &prepared.args {
        cmd.arg(arg);
    }
    cmd.cwd(prepared.cwd.clone());
    for (key, value) in &prepared.env {
        cmd.env(key, value);
    }
    if !prepared.env.contains_key("PATH") {
        cmd.env("PATH", DEFAULT_PATH);
    }
    Ok(cmd)
}

#[cfg(target_os = "linux")]
fn configure_command_process_group(command: &mut Command) -> Result<()> {
    use std::os::unix::process::CommandExt;

    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn configure_command_user(command: &mut Command, account: &ResolvedUserAccount) -> Result<()> {
    use std::os::unix::process::CommandExt;

    let uid = account.uid;
    let gid = account.gid;
    unsafe {
        command.pre_exec(move || {
            if libc::setgroups(0, std::ptr::null()) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::setgid(gid) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::setuid(uid) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn terminate_exec_process_group(pid: u32) -> Result<()> {
    let pgid = -(pid as i32);
    let signal_result = unsafe { libc::kill(pgid, libc::SIGTERM) };
    if signal_result != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::ESRCH) {
            return Err(err).with_context(|| format!("stopping guest command pid {pid}"));
        }
        return Ok(());
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        let probe = unsafe { libc::kill(pgid, 0) };
        if probe != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let kill_result = unsafe { libc::kill(pgid, libc::SIGKILL) };
    if kill_result != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::ESRCH) {
            return Err(err).with_context(|| format!("force-stopping guest command pid {pid}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::Cursor;

    use super::*;

    #[test]
    fn exec_stream_payload_budget_stays_within_shared_vm_safe_limit() {
        const {
            assert!(
                AVF_EXEC_STREAM_FRAME_MAX_PAYLOAD <= 1024,
                "shared-VM exec transport truncated larger stdin frames in live tar-import repros",
            );
        }
    }

    #[test]
    fn prepare_exec_request_rejects_empty_command() {
        let err = prepare_exec_request(&AvfLinuxExecRequest {
            protocol_version: AVF_LINUX_EXEC_PROTOCOL_VERSION,
            command: " ".to_string(),
            args: Vec::new(),
            cwd: "/tmp".to_string(),
            user: None,
            env: HashMap::new(),
            pty: false,
        })
        .expect_err("empty command should fail");
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn emit_exec_stream_frames_splits_large_stdout_payloads() {
        let payload = vec![b'x'; AVF_EXEC_STREAM_FRAME_MAX_PAYLOAD + 33];
        let mut bytes = Vec::new();

        emit_exec_stream_frames(&payload, true, |frame| {
            write_exec_frame(&mut bytes, &frame).map_err(anyhow::Error::from)
        })
        .expect("split stdout frames");

        let mut cursor = Cursor::new(bytes);
        let first = read_exec_frame(&mut cursor)
            .expect("read first")
            .expect("first frame");
        let second = read_exec_frame(&mut cursor)
            .expect("read second")
            .expect("second frame");
        let eof = read_exec_frame(&mut cursor).expect("read eof");

        let AvfLinuxExecFrame::Stdout(first) = first else {
            panic!("expected stdout frame");
        };
        let AvfLinuxExecFrame::Stdout(second) = second else {
            panic!("expected stdout frame");
        };

        assert_eq!(first.len(), AVF_EXEC_STREAM_FRAME_MAX_PAYLOAD);
        assert_eq!(second.len(), 33);
        assert!(eof.is_none());
    }
}
