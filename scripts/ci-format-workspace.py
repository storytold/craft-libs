#!/usr/bin/env python3
"""Check every workspace package separately to bound rustfmt's Windows argv."""
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def workspace_names(metadata):
    members = metadata["workspace_members"]
    packages = metadata["packages"]
    if not members or len(members) != len(set(members)):
        raise ValueError("workspace member inventory is empty or duplicated")
    by_id = {package["id"]: package for package in packages}
    if len(by_id) != len(packages):
        raise ValueError("metadata contains duplicate package IDs")
    names = []
    for member in members:
        name = by_id[member]["name"]
        # Package names are data, never command text or option fragments.
        if not isinstance(name, str) or not re.fullmatch(r"[A-Za-z0-9_][A-Za-z0-9_-]*", name):
            raise ValueError("invalid workspace package name")
        names.append(name)
    if len(names) != len(set(names)):
        raise ValueError("workspace package names are ambiguous")
    return sorted(names)


def check_workspace():
    result = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--offline", "--locked", "--format-version", "1"],
        cwd=ROOT, check=True, stdout=subprocess.PIPE, text=True, encoding="utf-8",
    )
    names = workspace_names(json.loads(result.stdout))
    # From this virtual workspace root, cargo fmt normally checks all workspace packages.
    # Keep Cargo responsible for target kinds, editions and rustfmt configuration.
    # One package per call retains all its targets without the workspace-sized argv.
    print(f"Checking formatting for {len(names)} workspace packages", flush=True)
    for name in names:
        subprocess.run(["cargo", "fmt", "--check", "--package", name], cwd=ROOT, check=True)


if __name__ == "__main__":
    check_workspace()
