#!/usr/bin/env python3
import argparse
import json
import os
import sqlite3
from collections import OrderedDict, defaultdict
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Dict, Iterable, List, Optional, Tuple


CHUNK_SIZE_BUCKETS = [
    8,
    16,
    32,
    64,
    128,
    256,
    512,
    1024,
    2048,
    4096,
    8192,
    16384,
    32768,
    65536,
]

DELAY_MS_BUCKETS = [
    5,
    10,
    20,
    50,
    100,
    200,
    500,
    1000,
    2000,
    5000,
    10000,
    20000,
    60000,
]

ARTIFACT_SIZE_BUCKETS = [
    1024,
    4096,
    16384,
    65536,
    262144,
    1048576,
    4194304,
    16777216,
    67108864,
    268435456,
]

TOOL_CALL_BUCKETS = [
    1,
    2,
    3,
    5,
    8,
    13,
    21,
    34,
]


@dataclass
class Distribution:
    count: int
    min: Optional[int]
    max: Optional[int]
    p50: Optional[int]
    p95: Optional[int]
    p99: Optional[int]
    histogram: List[Dict[str, int]]
    overflow: int


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Summarize recent high-activity daemon traces for load-test calibration."
    )
    parser.add_argument(
        "--data-dir",
        type=Path,
        help="ctx data directory (defaults to CTX_DATA_DIR or ~/.ctx)",
    )
    parser.add_argument(
        "--days",
        type=int,
        default=14,
        help="lookback window in days (default: 14)",
    )
    parser.add_argument(
        "--max-sessions",
        type=int,
        default=20,
        help="max sessions per workspace (default: 20)",
    )
    parser.add_argument(
        "--min-events",
        type=int,
        default=200,
        help="min events per session to qualify as high-activity (default: 200)",
    )
    parser.add_argument(
        "--out",
        type=Path,
        help="optional output path (JSON is always emitted to stdout)",
    )
    return parser.parse_args()


def resolve_data_dir(cli: argparse.Namespace) -> Path:
    if cli.data_dir:
        return cli.data_dir.expanduser()
    env = os.environ.get("CTX_DATA_DIR")
    if env:
        return Path(env).expanduser()
    return Path.home() / ".ctx"


def find_workspace_dbs(data_root: Path) -> List[Path]:
    base = data_root / "db" / "workspaces"
    if not base.exists():
        return []
    return sorted(base.glob("*/db.sqlite"))


def open_db(path: Path) -> sqlite3.Connection:
    conn = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    return conn


def parse_ts(value: Optional[str]) -> Optional[datetime]:
    if not value:
        return None
    text = value.strip()
    if text.endswith("Z"):
        text = text[:-1] + "+00:00"
    try:
        return datetime.fromisoformat(text)
    except ValueError:
        return None


def format_cutoff(value: datetime) -> str:
    return value.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.%fZ")


def percentile(sorted_values: List[int], pct: float) -> Optional[int]:
    if not sorted_values:
        return None
    idx = int(round((len(sorted_values) - 1) * pct))
    return sorted_values[idx]


def build_distribution(values: List[int], buckets: List[int]) -> Distribution:
    values_sorted = sorted(values)
    count = len(values_sorted)
    histogram = []
    overflow = 0
    counts = [0 for _ in buckets]
    for value in values_sorted:
        placed = False
        for idx, bound in enumerate(buckets):
            if value <= bound:
                counts[idx] += 1
                placed = True
                break
        if not placed:
            overflow += 1
    for bound, count_bucket in zip(buckets, counts):
        histogram.append({"le": bound, "count": count_bucket})
    if count == 0:
        return Distribution(
            count=0,
            min=None,
            max=None,
            p50=None,
            p95=None,
            p99=None,
            histogram=histogram,
            overflow=0,
        )
    return Distribution(
        count=count,
        min=values_sorted[0],
        max=values_sorted[-1],
        p50=percentile(values_sorted, 0.50),
        p95=percentile(values_sorted, 0.95),
        p99=percentile(values_sorted, 0.99),
        histogram=histogram,
        overflow=overflow,
    )


def extract_chunk_size(payload_raw: str) -> Optional[int]:
    try:
        payload = json.loads(payload_raw)
    except json.JSONDecodeError:
        return None
    content = payload.get("content_fragment") or payload.get("content")
    if not isinstance(content, str):
        return None
    return len(content)


def extract_git_status_key(payload_raw: str, session_id: str) -> Optional[str]:
    try:
        payload = json.loads(payload_raw)
    except json.JSONDecodeError:
        return None
    if payload.get("kind") != "git_status_snapshot":
        return None
    worktree_id = payload.get("worktree_id")
    if isinstance(worktree_id, str) and worktree_id:
        return worktree_id
    return session_id


def query_high_activity_sessions(
    conn: sqlite3.Connection,
    cutoff: str,
    max_sessions: int,
    min_events: int,
) -> List[Tuple[str, int]]:
    rows = conn.execute(
        """
        SELECT session_id, COUNT(*) AS cnt
        FROM session_events
        WHERE created_at >= ?
        GROUP BY session_id
        HAVING cnt >= ?
        ORDER BY cnt DESC
        LIMIT ?
        """,
        (cutoff, min_events, max_sessions),
    ).fetchall()
    if rows:
        return [(row["session_id"], row["cnt"]) for row in rows]
    rows = conn.execute(
        """
        SELECT session_id, COUNT(*) AS cnt
        FROM session_events
        WHERE created_at >= ?
        GROUP BY session_id
        ORDER BY cnt DESC
        LIMIT ?
        """,
        (cutoff, max_sessions),
    ).fetchall()
    return [(row["session_id"], row["cnt"]) for row in rows]


def select_in_clause(values: Iterable[str]) -> Tuple[str, List[str]]:
    values_list = list(values)
    if not values_list:
        return "", []
    placeholders = ",".join(["?"] * len(values_list))
    return f"({placeholders})", values_list


def main() -> None:
    cli = parse_args()
    data_root = resolve_data_dir(cli)
    workspace_dbs = find_workspace_dbs(data_root)
    if not workspace_dbs:
        raise SystemExit(f"No workspace DBs found under {data_root}")

    cutoff_dt = datetime.now(timezone.utc) - timedelta(days=cli.days)
    cutoff = format_cutoff(cutoff_dt)
    generated_at = format_cutoff(datetime.now(timezone.utc))

    chunk_sizes: List[int] = []
    chunk_delays_ms: List[int] = []
    tool_calls_per_turn: List[int] = []
    tool_types: Dict[str, int] = defaultdict(int)
    artifact_sizes: List[int] = []
    git_change_delays_ms: List[int] = []

    session_count = 0
    event_count = 0

    for db_path in workspace_dbs:
        conn = open_db(db_path)
        sessions = query_high_activity_sessions(
            conn, cutoff, cli.max_sessions, cli.min_events
        )
        if not sessions:
            conn.close()
            continue

        session_ids = [session_id for session_id, _ in sessions]
        event_count += sum(cnt for _, cnt in sessions)
        session_count += len(session_ids)

        in_clause, params = select_in_clause(session_ids)
        if not in_clause:
            conn.close()
            continue

        rows = conn.execute(
            f"""
            SELECT session_id, turn_id, created_at, payload_json
            FROM session_events
            WHERE event_type = 'assistant_chunk'
              AND created_at >= ?
              AND session_id IN {in_clause}
            ORDER BY session_id, turn_id, created_at
            """,
            [cutoff] + params,
        ).fetchall()

        last_chunk_ts: Dict[Tuple[str, Optional[str]], datetime] = {}
        for row in rows:
            session_id = row["session_id"]
            turn_id = row["turn_id"]
            created_at = parse_ts(row["created_at"])
            size = extract_chunk_size(row["payload_json"])
            if size is not None:
                chunk_sizes.append(size)
            if created_at is None:
                continue
            key = (session_id, turn_id)
            last = last_chunk_ts.get(key)
            if last is not None:
                delta_ms = int((created_at - last).total_seconds() * 1000)
                if delta_ms >= 0:
                    chunk_delays_ms.append(delta_ms)
            last_chunk_ts[key] = created_at

        rows = conn.execute(
            f"""
            SELECT turn_id, tool_kind
            FROM session_turn_tools
            WHERE created_at >= ?
              AND session_id IN {in_clause}
            """,
            [cutoff] + params,
        ).fetchall()
        per_turn: Dict[str, int] = defaultdict(int)
        for row in rows:
            turn_id = row["turn_id"]
            per_turn[turn_id] += 1
            tool_kind = row["tool_kind"] or "unknown"
            tool_types[tool_kind] += 1
        tool_calls_per_turn.extend(per_turn.values())

        rows = conn.execute(
            f"""
            SELECT bytes
            FROM artifacts
            WHERE created_at >= ?
              AND session_id IN {in_clause}
            """,
            [cutoff] + params,
        ).fetchall()
        for row in rows:
            if row["bytes"] is not None:
                artifact_sizes.append(int(row["bytes"]))

        rows = conn.execute(
            f"""
            SELECT session_id, created_at, payload_json
            FROM session_events
            WHERE event_type = 'notice'
              AND created_at >= ?
              AND session_id IN {in_clause}
            ORDER BY session_id, created_at
            """,
            [cutoff] + params,
        ).fetchall()
        last_git_ts: Dict[str, datetime] = {}
        for row in rows:
            key = extract_git_status_key(row["payload_json"], row["session_id"])
            if key is None:
                continue
            created_at = parse_ts(row["created_at"])
            if created_at is None:
                continue
            last = last_git_ts.get(key)
            if last is not None:
                delta_ms = int((created_at - last).total_seconds() * 1000)
                if delta_ms >= 0:
                    git_change_delays_ms.append(delta_ms)
            last_git_ts[key] = created_at

        conn.close()

    output = OrderedDict(
        [
            ("generated_at", generated_at),
            ("cutoff_at", cutoff),
            ("data_root", str(data_root)),
            (
                "session_sample",
                OrderedDict(
                    [
                        ("sessions", session_count),
                        ("events", event_count),
                        ("workspaces", len(workspace_dbs)),
                    ]
                ),
            ),
            (
                "message_chunk_sizes",
                build_distribution(chunk_sizes, CHUNK_SIZE_BUCKETS).__dict__,
            ),
            (
                "inter_chunk_delay_ms",
                build_distribution(chunk_delays_ms, DELAY_MS_BUCKETS).__dict__,
            ),
            (
                "tool_calls",
                OrderedDict(
                    [
                        (
                            "per_turn",
                            build_distribution(
                                tool_calls_per_turn, TOOL_CALL_BUCKETS
                            ).__dict__,
                        ),
                        (
                            "types",
                            OrderedDict(
                                sorted(tool_types.items(), key=lambda item: item[0])
                            ),
                        ),
                    ]
                ),
            ),
            (
                "artifact_sizes_bytes",
                build_distribution(artifact_sizes, ARTIFACT_SIZE_BUCKETS).__dict__,
            ),
            (
                "git_change_cadence_ms",
                build_distribution(git_change_delays_ms, DELAY_MS_BUCKETS).__dict__,
            ),
        ]
    )

    payload = json.dumps(output, indent=2)
    if cli.out:
        cli.out.parent.mkdir(parents=True, exist_ok=True)
        cli.out.write_text(payload + "\n")
    print(payload)


if __name__ == "__main__":
    main()
