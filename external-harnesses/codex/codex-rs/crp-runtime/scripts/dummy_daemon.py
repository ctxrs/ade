#!/usr/bin/env python3
import json
import subprocess
import sys
import time


def main() -> int:
    crp_bin = sys.argv[1] if len(sys.argv) > 1 else "codex-crp"
    proc = subprocess.Popen(
        [crp_bin],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    if proc.stdin is None or proc.stdout is None:
        print("failed to start codex-crp", file=sys.stderr)
        return 1

    def send(obj: dict) -> None:
        proc.stdin.write(json.dumps(obj) + "\n")
        proc.stdin.flush()

    send(
        {
            "type": "session.open",
            "session_id": "dummy-session",
            "config": {"reasoning_trace_enabled": True},
        }
    )
    time.sleep(0.5)
    send(
        {
            "type": "session.prompt",
            "session_id": "dummy-session",
            "turn_id": "dummy-turn-1",
            "prompt": "Say hello, then run `pwd`.",
        }
    )

    for line in proc.stdout:
        line = line.strip()
        if not line:
            continue
        print(line)
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get("type") == "tool.request":
            tool_call_id = event.get("tool_call_id")
            send(
                {
                    "type": "tool.result",
                    "session_id": event.get("session_id"),
                    "turn_id": event.get("turn_id"),
                    "tool_call_id": tool_call_id,
                    "status": "success",
                    "output": "dummy tool output",
                }
            )

    return proc.wait()


if __name__ == "__main__":
    raise SystemExit(main())
