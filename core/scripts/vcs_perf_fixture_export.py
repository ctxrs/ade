#!/usr/bin/env python3

import argparse
import json
import os
import shutil
import sqlite3
import subprocess
import sys
import tarfile
import tempfile
import time
from pathlib import Path


def run(cmd, cwd=None, capture_output=True, text=True, check=True):
    return subprocess.run(
        cmd,
        cwd=cwd,
        capture_output=capture_output,
        text=text,
        check=check,
    )


def git_output(repo_root: Path, *args: str, text: bool = True) -> str | bytes:
    completed = subprocess.run(
        ["git", "-C", str(repo_root), *args],
        capture_output=True,
        check=True,
        text=text,
    )
    return completed.stdout


def backup_sqlite(src: Path, dest: Path) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    with sqlite3.connect(f"file:{src}?mode=ro", uri=True) as source:
        with sqlite3.connect(dest) as target:
            source.backup(target)


def load_workspace_row(global_db: Path, workspace_root: Path) -> dict[str, str]:
    with sqlite3.connect(global_db) as conn:
        conn.row_factory = sqlite3.Row
        row = conn.execute(
            """
            SELECT id, name, root_path
            FROM workspaces
            WHERE root_path = ?
            """,
            (str(workspace_root),),
        ).fetchone()
    if row is None:
        raise SystemExit(f"workspace not found for root_path={workspace_root}")
    return dict(row)


def path_exists(path: str | None) -> bool:
    return bool(path) and Path(path).exists()


def prune_global_db(global_db: Path, workspace_id: str) -> None:
    with sqlite3.connect(global_db) as conn:
        conn.execute("PRAGMA foreign_keys=ON")
        conn.execute("DELETE FROM workspace_attachments WHERE workspace_id <> ?", (workspace_id,))
        conn.execute("DELETE FROM workspaces WHERE id <> ?", (workspace_id,))
        conn.commit()
    with sqlite3.connect(global_db) as conn:
        conn.execute("VACUUM")


def compute_missing_empty_task_ids(workspace_db: Path) -> list[str]:
    with sqlite3.connect(workspace_db) as conn:
        conn.row_factory = sqlite3.Row
        rows = conn.execute(
            """
            SELECT
              t.id,
              t.primary_worktree_id,
              wt.root_path,
              COALESCE(session_counts.session_count, 0) AS session_count
            FROM tasks t
            LEFT JOIN worktrees wt ON wt.id = t.primary_worktree_id
            LEFT JOIN (
              SELECT task_id, COUNT(*) AS session_count
              FROM sessions
              GROUP BY task_id
            ) session_counts ON session_counts.task_id = t.id
            WHERE t.archived_at IS NULL
            """
        ).fetchall()
    missing_ids: list[str] = []
    for row in rows:
        if row["session_count"] != 0:
            continue
        primary_worktree_id = row["primary_worktree_id"]
        if not primary_worktree_id:
            continue
        if path_exists(row["root_path"]):
            continue
        missing_ids.append(row["id"])
    return missing_ids


def prune_workspace_db(workspace_db: Path, drop_task_ids: list[str]) -> None:
    placeholders = ",".join("?" for _ in drop_task_ids)
    with sqlite3.connect(workspace_db) as conn:
        conn.execute("PRAGMA foreign_keys=ON")
        conn.execute("DELETE FROM tasks WHERE archived_at IS NOT NULL")
        if drop_task_ids:
            conn.execute(f"DELETE FROM tasks WHERE id IN ({placeholders})", drop_task_ids)
        conn.execute(
            """
            DELETE FROM worktrees
            WHERE id NOT IN (
              SELECT primary_worktree_id FROM tasks WHERE primary_worktree_id IS NOT NULL
              UNION
              SELECT DISTINCT worktree_id FROM sessions
            )
            """
        )
        conn.execute("DELETE FROM artifacts WHERE task_id NOT IN (SELECT id FROM tasks)")
        conn.execute("DELETE FROM workspace_task_index")
        conn.execute(
            """
            INSERT INTO workspace_task_index(task_id, workspace_id)
            SELECT id, workspace_id FROM tasks
            """
        )
        conn.execute("DELETE FROM workspace_session_index")
        conn.execute(
            """
            INSERT INTO workspace_session_index(session_id, workspace_id)
            SELECT id, workspace_id FROM sessions
            """
        )
        conn.execute("DELETE FROM workspace_worktree_index")
        conn.execute(
            """
            INSERT INTO workspace_worktree_index(worktree_id, workspace_id)
            SELECT id, workspace_id FROM worktrees
            """
        )
        conn.commit()
    with sqlite3.connect(workspace_db) as conn:
        conn.execute("VACUUM")


def read_manifest_rows(workspace_db: Path) -> tuple[list[dict], list[dict]]:
    with sqlite3.connect(workspace_db) as conn:
        conn.row_factory = sqlite3.Row
        tasks = [
            dict(row)
            for row in conn.execute(
                """
                SELECT id, title, status, primary_session_id, primary_worktree_id
                FROM tasks
                WHERE archived_at IS NULL
                ORDER BY COALESCE(last_activity_at, updated_at) DESC
                """
            ).fetchall()
        ]
        worktrees = [
            dict(row)
            for row in conn.execute(
                """
                WITH needed_worktrees AS (
                  SELECT primary_worktree_id AS id
                  FROM tasks
                  WHERE archived_at IS NULL AND primary_worktree_id IS NOT NULL
                  UNION
                  SELECT DISTINCT worktree_id AS id
                  FROM sessions
                  WHERE task_id IN (
                    SELECT id FROM tasks WHERE archived_at IS NULL
                  )
                )
                SELECT id, root_path, git_branch, base_commit_sha, vcs_ref
                FROM worktrees
                WHERE id IN (SELECT id FROM needed_worktrees)
                ORDER BY created_at DESC
                """
            ).fetchall()
        ]
    return tasks, worktrees


def summarize_porcelain(path: Path) -> dict[str, int]:
    raw = git_output(path, "status", "--porcelain=v1", "-z", "--untracked-files=all", text=False)
    entries = [entry for entry in raw.decode("utf-8", "replace").split("\0") if entry]
    tracked = 0
    untracked = 0
    for entry in entries:
        if entry.startswith("?? "):
            untracked += 1
        else:
            tracked += 1
    return {
        "tracked_entries": tracked,
        "untracked_entries": untracked,
        "total_entries": tracked + untracked,
    }


def write_patch_file(path: Path, content: bytes) -> int:
    if not content:
        return 0
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(content)
    return len(content)


def write_untracked_archive(repo_root: Path, output_path: Path) -> tuple[int, int]:
    raw = git_output(repo_root, "ls-files", "-o", "--exclude-standard", "-z", text=False)
    relpaths = [entry for entry in raw.decode("utf-8", "replace").split("\0") if entry]
    if not relpaths:
        return 0, 0
    output_path.parent.mkdir(parents=True, exist_ok=True)
    total_bytes = 0
    with tarfile.open(output_path, "w:gz") as archive:
        for relpath in sorted(relpaths):
            source = repo_root / relpath
            if not source.is_file():
                continue
            total_bytes += source.stat().st_size
            archive.add(source, arcname=relpath, recursive=False)
    return len(relpaths), total_bytes


def build_bundle(repo_root: Path, refs: dict[str, str], bundle_path: Path) -> dict[str, str]:
    token = f"ctx-vcs-fixture-{int(time.time())}-{os.getpid()}"
    created_refs: dict[str, str] = {}
    try:
        for name, commit in refs.items():
            ref_name = f"refs/heads/{token}/{name}"
            run(["git", "-C", str(repo_root), "update-ref", ref_name, commit], text=False)
            created_refs[name] = ref_name
        run(
            [
                "git",
                "-C",
                str(repo_root),
                "bundle",
                "create",
                str(bundle_path),
                *created_refs.values(),
            ],
            text=False,
        )
    finally:
        for ref_name in created_refs.values():
            subprocess.run(
                ["git", "-C", str(repo_root), "update-ref", "-d", ref_name],
                capture_output=True,
                text=True,
            )
    return created_refs


def export_fixture(args: argparse.Namespace) -> int:
    data_root = args.data_root.expanduser().resolve()
    workspace_root = args.workspace_root.expanduser().resolve()
    out_dir = args.out_dir.expanduser().resolve()
    if out_dir.exists():
        if args.force:
            shutil.rmtree(out_dir)
        else:
            raise SystemExit(f"output directory already exists: {out_dir}")
    out_dir.mkdir(parents=True, exist_ok=True)

    global_db_src = data_root / "db" / "db.sqlite"
    workspace_row = load_workspace_row(global_db_src, workspace_root)
    workspace_id = workspace_row["id"]
    workspace_db_src = data_root / "db" / "workspaces" / workspace_id / "db.sqlite"
    if not workspace_db_src.exists():
        raise SystemExit(f"workspace db not found: {workspace_db_src}")

    global_db_out = out_dir / "global-db.sqlite"
    workspace_db_out = out_dir / "workspace-db.sqlite"
    backup_sqlite(global_db_src, global_db_out)
    backup_sqlite(workspace_db_src, workspace_db_out)
    prune_global_db(global_db_out, workspace_id)

    missing_task_ids = compute_missing_empty_task_ids(workspace_db_out)
    prune_workspace_db(workspace_db_out, missing_task_ids)
    tasks, worktrees = read_manifest_rows(workspace_db_out)

    refs: dict[str, str] = {}
    main_head_commit = git_output(workspace_root, "rev-parse", "HEAD").strip()
    refs["main"] = main_head_commit

    worktree_manifest: list[dict] = []
    overlays_dir = out_dir / "overlays"
    for row in worktrees:
        worktree_id = row["id"]
        source_root = Path(row["root_path"])
        if not source_root.exists():
            continue
        head_commit = git_output(source_root, "rev-parse", "HEAD").strip()
        refs[f"worktree-{worktree_id}"] = head_commit
        tracked_staged = git_output(source_root, "diff", "--cached", "--binary", "HEAD", text=False)
        tracked_unstaged = git_output(source_root, "diff", "--binary", text=False)
        overlay_root = overlays_dir / worktree_id
        staged_path = overlay_root / "staged.patch"
        unstaged_path = overlay_root / "unstaged.patch"
        untracked_archive = overlay_root / "untracked.tar.gz"
        staged_bytes = write_patch_file(staged_path, tracked_staged)
        unstaged_bytes = write_patch_file(unstaged_path, tracked_unstaged)
        untracked_files, untracked_bytes = write_untracked_archive(source_root, untracked_archive)
        porcelain = summarize_porcelain(source_root)
        worktree_manifest.append(
            {
                "id": worktree_id,
                "source_root": str(source_root),
                "head_commit": head_commit,
                "db_git_branch": row["git_branch"],
                "base_commit_sha": row["base_commit_sha"],
                "vcs_ref": row["vcs_ref"],
                "dirty": {
                    **porcelain,
                    "staged_patch_bytes": staged_bytes,
                    "unstaged_patch_bytes": unstaged_bytes,
                    "untracked_files": untracked_files,
                    "untracked_bytes": untracked_bytes,
                },
            }
        )

    bundle_refs = build_bundle(workspace_root, refs, out_dir / "repo.bundle")
    manifest = {
        "workspace": {
            "id": workspace_id,
            "name": workspace_row["name"],
            "source_root": str(workspace_root),
            "main_head_commit": main_head_commit,
            "bundle_ref": bundle_refs["main"],
        },
        "counts": {
            "tasks": len(tasks),
            "worktrees": len(worktree_manifest),
        },
        "tasks": tasks,
        "worktrees": [
            {
                **worktree,
                "bundle_ref": bundle_refs[f"worktree-{worktree['id']}"],
                "overlay_dir": f"overlays/{worktree['id']}",
            }
            for worktree in worktree_manifest
        ],
        "dropped_task_ids": missing_task_ids,
        "created_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    }
    (out_dir / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"out_dir": str(out_dir), "workspace_id": workspace_id, "counts": manifest["counts"]}))
    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Export an active-workspace VCS perf fixture with exact dirty worktree overlays."
    )
    parser.add_argument("--workspace-root", type=Path, required=True)
    parser.add_argument("--data-root", type=Path, default=Path("~/.ctx"))
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--force", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    return export_fixture(args)


if __name__ == "__main__":
    sys.exit(main())
