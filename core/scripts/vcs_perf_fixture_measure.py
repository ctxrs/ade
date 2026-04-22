#!/usr/bin/env python3

import argparse
import json
import os
import socket
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path

try:
    from websockets.sync.client import connect as ws_connect
except Exception:
    ws_connect = None


def pick_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def parse_ps_cputime(raw: str) -> float:
    value = raw.strip()
    if not value:
        return 0.0
    days = 0
    if "-" in value:
        day_part, value = value.split("-", 1)
        days = int(day_part)
    parts = value.split(":")
    seconds = 0.0
    for part in parts:
        seconds = seconds * 60.0 + float(part)
    return seconds + days * 86400.0


def read_pid_cpu_seconds(pid: int) -> float:
    proc_stat = Path(f"/proc/{pid}/stat")
    if proc_stat.exists():
        stat = proc_stat.read_text(encoding="utf-8")
        fields = stat.split()
        clk_tck = os.sysconf(os.sysconf_names["SC_CLK_TCK"])
        return (int(fields[13]) + int(fields[14])) / clk_tck
    completed = subprocess.run(
        ["ps", "-p", str(pid), "-o", "cputime="],
        capture_output=True,
        text=True,
        check=True,
    )
    return parse_ps_cputime(completed.stdout)


def sample_cpu(pid: int, duration_s: float, interval_s: float) -> list[float]:
    points: list[float] = []
    start = time.monotonic()
    prev_time = start
    prev_cpu_seconds = read_pid_cpu_seconds(pid)
    while True:
        time.sleep(interval_s)
        now = time.monotonic()
        cpu_seconds = read_pid_cpu_seconds(pid)
        cpu_percent = ((cpu_seconds - prev_cpu_seconds) / (now - prev_time)) * 100.0
        points.append(cpu_percent)
        prev_time = now
        prev_cpu_seconds = cpu_seconds
        if now - start >= duration_s:
            break
    return points


def wait_for_health(base_url: str, timeout_s: float) -> float:
    start = time.monotonic()
    deadline = start + timeout_s
    while time.monotonic() < deadline:
        try:
            with urllib.request.urlopen(f"{base_url}/api/health", timeout=2) as response:
                if response.status == 200:
                    return time.monotonic() - start
        except Exception:
            time.sleep(0.5)
    raise TimeoutError("daemon failed to become healthy in time")


def load_auth_token(data_root: Path, timeout_s: float) -> str:
    auth_path = data_root / "daemon_auth.json"
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        if auth_path.exists():
            payload = json.loads(auth_path.read_text(encoding="utf-8"))
            token = payload.get("token", "")
            if token:
                return token
        time.sleep(0.25)
    raise TimeoutError(f"failed to load auth token from {auth_path}")


def request_json(url: str, token: str, timeout_s: float = 30.0) -> tuple[float, object]:
    request = urllib.request.Request(url, headers={"Authorization": f"Bearer {token}"})
    start = time.monotonic()
    with urllib.request.urlopen(request, timeout=timeout_s) as response:
        payload = json.loads(response.read().decode("utf-8"))
    return time.monotonic() - start, payload


def collect_primary_session_ids(manifest: dict) -> list[str]:
    session_ids: list[str] = []
    for task in manifest["tasks"]:
        session_id = task.get("primary_session_id")
        if session_id:
            session_ids.append(session_id)
    return session_ids


class WorkspaceStreamSubscription:
    def __init__(self, url: str, message: dict):
        self.url = url
        self.message = message
        self.connection = None
        self._stop = threading.Event()
        self._thread = None

    def start(self) -> bool:
        if ws_connect is None:
            return False
        self.connection = ws_connect(self.url, max_size=None)
        self.connection.send(json.dumps(self.message))
        self._thread = threading.Thread(target=self._drain, daemon=True)
        self._thread.start()
        return True

    def _drain(self) -> None:
        while not self._stop.is_set():
            try:
                self.connection.recv(timeout=1)
            except TimeoutError:
                continue
            except Exception:
                break

    def close(self) -> None:
        self._stop.set()
        if self.connection is not None:
            try:
                self.connection.close()
            except Exception:
                pass
        if self._thread is not None:
            self._thread.join(timeout=2)


def choose_hot_worktrees(manifest: dict, limit: int) -> list[dict]:
    return sorted(
        manifest["worktrees"],
        key=lambda entry: entry["dirty"]["total_entries"],
        reverse=True,
    )[:limit]


def churn_worktrees(fixture_root: Path, manifest: dict, worktree_limit: int, waves: int) -> dict:
    touched = 0
    created = 0
    hot_worktrees = choose_hot_worktrees(manifest, worktree_limit)
    for wave in range(waves):
        for worktree in hot_worktrees:
            root = fixture_root / "worktrees" / worktree["id"]
            git_cmd = ["git", "-C", str(root)]
            modified = subprocess.run(
                git_cmd + ["ls-files", "-m", "-o", "--exclude-standard", "-z"],
                capture_output=True,
                text=False,
                check=True,
            ).stdout
            relpaths = [entry for entry in modified.decode("utf-8", "replace").split("\0") if entry]
            for relpath in relpaths[:400]:
                candidate = root / relpath
                if not candidate.is_file():
                    continue
                os.utime(candidate, None)
                touched += 1
            temp_path = root / f".ctx-vcs-churn-wave-{wave}"
            temp_path.write_text(f"wave={wave}\n", encoding="utf-8")
            temp_path.unlink()
            created += 1
        time.sleep(1.0)
    return {"touched_files": touched, "temp_file_events": created}


def summarize(points: list[float]) -> dict[str, float]:
    if not points:
        return {"avg_cpu": 0.0, "max_cpu": 0.0}
    return {
        "avg_cpu": sum(points) / len(points),
        "max_cpu": max(points),
    }


def measure(args: argparse.Namespace) -> int:
    code_dir = args.code_dir.expanduser().resolve()
    data_root = args.data_root.expanduser().resolve()
    fixture_root = args.fixture_root.expanduser().resolve()
    out_json = args.out_json.expanduser().resolve()
    manifest = json.loads((fixture_root / "manifest.json").read_text(encoding="utf-8"))
    workspace_id = manifest["workspace"]["id"]
    port = args.port or pick_port()
    base_url = f"http://127.0.0.1:{port}"
    log_path = out_json.with_suffix(".daemon.log")
    log_path.parent.mkdir(parents=True, exist_ok=True)
    target_dir = code_dir / "target" / "vcs-perf"
    build_env = os.environ.copy()
    build_env["CARGO_TARGET_DIR"] = str(target_dir)

    build_command = [
        "cargo",
        "build",
        "-p",
        "ctx-http",
        "--bin",
        "ctx",
    ]
    subprocess.run(build_command, cwd=code_dir, env=build_env, check=True)

    binary_path = target_dir / "debug" / "ctx"
    command = [
        str(binary_path),
        "serve",
        "--bind",
        f"127.0.0.1:{port}",
        "--data-dir",
        str(data_root),
    ]
    with log_path.open("wb") as log_file:
        process = subprocess.Popen(
            command,
            cwd=code_dir,
            env=build_env,
            stdout=log_file,
            stderr=subprocess.STDOUT,
        )
        stream = None
        stream_started = False
        try:
            health_latency = wait_for_health(base_url, args.health_timeout_s)
            auth_token = load_auth_token(data_root, 10.0)
            primary_session_ids = collect_primary_session_ids(manifest)[: args.primary_sessions_to_probe]
            subscribe_message = {
                "type": "subscribe",
                "scope": "active",
                "include_active_heads": True,
            }
            if primary_session_ids:
                subscribe_message["foreground_session_id"] = primary_session_ids[0]
                subscribe_message["session_ids"] = primary_session_ids
                subscribe_message["sessions"] = [
                    {
                        "session_id": session_id,
                        "replay": {"mode": "auto"},
                    }
                    for session_id in primary_session_ids
                ]
            stream = WorkspaceStreamSubscription(
                f"ws://127.0.0.1:{port}/api/workspaces/{workspace_id}/active_snapshot/stream?token={auth_token}",
                subscribe_message,
            )
            stream_started = stream.start()
            startup_cpu = sample_cpu(process.pid, args.startup_sample_s, args.cpu_interval_s)
            active_snapshot_latency, active_snapshot_payload = request_json(
                f"{base_url}/api/workspaces/{workspace_id}/active_snapshot?limit={args.active_snapshot_limit}",
                auth_token,
            )
            active_heads_latency, _ = request_json(
                f"{base_url}/api/workspaces/{workspace_id}/active_heads",
                auth_token,
            )
            primary_head_latencies: list[dict] = []
            for session_id in primary_session_ids:
                latency, _ = request_json(
                    f"{base_url}/api/sessions/{session_id}/head?limit=60&include_events=false",
                    auth_token,
                )
                primary_head_latencies.append({"session_id": session_id, "latency_s": latency})
            churn_summary = churn_worktrees(
                fixture_root,
                manifest,
                args.churn_worktree_limit,
                args.churn_waves,
            )
            churn_cpu = sample_cpu(process.pid, args.churn_sample_s, args.cpu_interval_s)
            post_churn_snapshot_latency, _ = request_json(
                f"{base_url}/api/workspaces/{workspace_id}/active_snapshot?limit={args.active_snapshot_limit}",
                auth_token,
            )
        finally:
            if stream is not None:
                stream.close()
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)

    result = {
        "workspace_id": workspace_id,
        "port": port,
        "health_latency_s": health_latency,
        "startup": {
            **summarize(startup_cpu),
            "samples": startup_cpu,
            "active_snapshot_latency_s": active_snapshot_latency,
            "active_snapshot_task_count": len(
                active_snapshot_payload.get("active", {}).get("tasks", [])
            )
            if isinstance(active_snapshot_payload, dict)
            else None,
            "active_heads_latency_s": active_heads_latency,
            "primary_head_latencies": primary_head_latencies,
            "workspace_stream_connected": stream_started,
        },
        "churn": {
            **summarize(churn_cpu),
            "samples": churn_cpu,
            "active_snapshot_latency_s": post_churn_snapshot_latency,
            **churn_summary,
        },
        "log_path": str(log_path),
        "code_dir": str(code_dir),
        "data_root": str(data_root),
        "fixture_root": str(fixture_root),
    }
    out_json.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result, indent=2))
    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Measure daemon CPU and latency against a VCS perf fixture.")
    parser.add_argument("--code-dir", type=Path, required=True)
    parser.add_argument("--data-root", type=Path, required=True)
    parser.add_argument("--fixture-root", type=Path, required=True)
    parser.add_argument("--out-json", type=Path, required=True)
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--health-timeout-s", type=float, default=180.0)
    parser.add_argument("--startup-sample-s", type=float, default=20.0)
    parser.add_argument("--churn-sample-s", type=float, default=20.0)
    parser.add_argument("--cpu-interval-s", type=float, default=1.0)
    parser.add_argument("--active-snapshot-limit", type=int, default=25)
    parser.add_argument("--primary-sessions-to-probe", type=int, default=6)
    parser.add_argument("--churn-worktree-limit", type=int, default=3)
    parser.add_argument("--churn-waves", type=int, default=3)
    return parser.parse_args()


def main() -> int:
    return measure(parse_args())


if __name__ == "__main__":
    sys.exit(main())
