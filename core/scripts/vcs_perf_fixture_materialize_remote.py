#!/usr/bin/env python3

import argparse
import json
import shutil
import sqlite3
import subprocess
import sys
import tarfile
from pathlib import Path


def run(cmd, cwd=None, check=True):
    subprocess.run(cmd, cwd=cwd, check=check)


def clone_bundle(bundle_path: Path, repo_dir: Path) -> None:
    run(["git", "clone", "--quiet", "--no-checkout", str(bundle_path), str(repo_dir)])


def copy_file(src: Path, dest: Path) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(src, dest)


def rewrite_paths(
    global_db: Path,
    workspace_db: Path,
    workspace_id: str,
    workspace_root: Path,
    worktree_roots: dict[str, Path],
) -> None:
    with sqlite3.connect(global_db) as conn:
        conn.execute("UPDATE workspaces SET root_path = ? WHERE id = ?", (str(workspace_root), workspace_id))
        conn.commit()

    with sqlite3.connect(workspace_db) as conn:
        conn.execute("PRAGMA foreign_keys=ON")
        conn.execute("UPDATE workspaces SET root_path = ? WHERE id = ?", (str(workspace_root), workspace_id))
        for worktree_id, root in worktree_roots.items():
            conn.execute("UPDATE worktrees SET root_path = ? WHERE id = ?", (str(root), worktree_id))
        conn.commit()


def extract_untracked_archive(archive_path: Path, dest_dir: Path) -> None:
    if not archive_path.exists():
        return
    with tarfile.open(archive_path, "r:gz") as archive:
        archive.extractall(dest_dir)


def materialize(args: argparse.Namespace) -> int:
    fixture_dir = args.fixture_dir.expanduser().resolve()
    dest_root = args.dest_root.expanduser().resolve()
    if dest_root.exists():
        if args.force:
            shutil.rmtree(dest_root)
        else:
            raise SystemExit(f"destination already exists: {dest_root}")
    dest_root.mkdir(parents=True, exist_ok=True)

    manifest = json.loads((fixture_dir / "manifest.json").read_text(encoding="utf-8"))
    workspace_id = manifest["workspace"]["id"]
    repo_dir = dest_root / "repo"
    worktrees_root = dest_root / "worktrees"
    data_root = dest_root / "data"
    bundle_path = fixture_dir / "repo.bundle"

    clone_bundle(bundle_path, repo_dir)
    run(["git", "-C", str(repo_dir), "checkout", "--detach", manifest["workspace"]["main_head_commit"]])

    remote_worktrees: dict[str, Path] = {}
    for worktree in manifest["worktrees"]:
        worktree_id = worktree["id"]
        worktree_dir = worktrees_root / worktree_id
        remote_worktrees[worktree_id] = worktree_dir
        run(
            [
                "git",
                "-C",
                str(repo_dir),
                "worktree",
                "add",
                "--detach",
                str(worktree_dir),
                worktree["head_commit"],
            ]
        )
        overlay_dir = fixture_dir / worktree["overlay_dir"]
        staged_patch = overlay_dir / "staged.patch"
        unstaged_patch = overlay_dir / "unstaged.patch"
        if staged_patch.exists():
            run(
                [
                    "git",
                    "-C",
                    str(worktree_dir),
                    "apply",
                    "--binary",
                    "--index",
                    str(staged_patch),
                ]
            )
        if unstaged_patch.exists():
            run(["git", "-C", str(worktree_dir), "apply", "--binary", str(unstaged_patch)])
        extract_untracked_archive(overlay_dir / "untracked.tar.gz", worktree_dir)

    global_db = data_root / "db" / "db.sqlite"
    workspace_db = data_root / "db" / "workspaces" / workspace_id / "db.sqlite"
    copy_file(fixture_dir / "global-db.sqlite", global_db)
    copy_file(fixture_dir / "workspace-db.sqlite", workspace_db)
    rewrite_paths(global_db, workspace_db, workspace_id, repo_dir, remote_worktrees)

    copy_file(fixture_dir / "manifest.json", dest_root / "manifest.json")
    summary = {
        "fixture_root": str(dest_root),
        "repo_dir": str(repo_dir),
        "data_root": str(data_root),
        "workspace_id": workspace_id,
        "worktree_count": len(remote_worktrees),
    }
    print(json.dumps(summary))
    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Materialize a VCS perf fixture on a remote machine.")
    parser.add_argument("--fixture-dir", type=Path, required=True)
    parser.add_argument("--dest-root", type=Path, required=True)
    parser.add_argument("--force", action="store_true")
    return parser.parse_args()


def main() -> int:
    return materialize(parse_args())


if __name__ == "__main__":
    sys.exit(main())
