#!/usr/bin/env python3
"""Native DMG startup/overwrite smoke, exclusively on fresh GitHub macOS runners."""
import argparse
import json
import os
from pathlib import Path
import platform
import plistlib
import re
import signal
import subprocess
import tempfile
import time
from fixture_schema import source_schema


def command(*args, **kwargs):
    if kwargs.get("text") or kwargs.get("universal_newlines"):
        kwargs.setdefault("encoding", "utf-8")
        kwargs.setdefault("errors", "replace")
    return subprocess.run(args, check=True, timeout=120, **kwargs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--installer", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--scenario", choices=("fresh", "upgrade"), required=True)
    parser.add_argument("--baseline", type=Path)
    args = parser.parse_args()
    if (platform.system() != "Darwin" or platform.machine() != "arm64"
            or os.environ.get("GITHUB_ACTIONS") != "true"
            or os.environ.get("RUNNER_ENVIRONMENT") != "github-hosted"):
        raise RuntimeError("Native smoke requires a fresh GitHub-hosted macOS arm64 runner")
    if not re.fullmatch(r"\d+\.\d+\.\d+", args.version):
        raise ValueError("Expected a stable candidate version")
    runner_temp = Path(os.environ["RUNNER_TEMP"]).resolve(strict=True)
    user_directory = Path.home()
    data = user_directory / "Library/Application Support/com.starclip.desktop"
    for path in (data, user_directory / "Library/Caches/com.starclip.desktop",
                 user_directory / "Library/WebKit/com.starclip.desktop",
                 user_directory / "Library/Preferences/com.starclip.desktop.plist",
                 Path("/Applications/Starclip.app"), user_directory / "Applications/Starclip.app"):
        if path.exists() or path.is_symlink():
            raise RuntimeError("Existing Starclip data or installation found; refusing to run")
    if subprocess.run(["pgrep", "-ix", "starclip"], capture_output=True).returncode != 1:
        raise RuntimeError("Starclip is already running, or process detection failed")
    if args.scenario == "upgrade" and (
            not args.baseline or tuple(map(int, args.version.split("."))) <= (0, 1, 0)):
        raise ValueError("Upgrade smoke requires a verified 0.1.0 DMG and newer candidate")
    package = args.installer.resolve(strict=True)
    baseline = args.baseline.resolve(strict=True) if args.baseline else None
    scratch = Path(tempfile.mkdtemp(prefix="starclip-macos-smoke-", dir=runner_temp))
    installation = scratch / "Starclip.app"
    database = data / "starclip.sqlite3"
    scripts = Path(__file__).resolve().parent
    inspector = scratch / "inspect-window"
    command("xcrun", "swiftc", str(scripts / "macos-smoke.swift"), "-o", str(inspector))
    process = None

    def verify_app(app, expected):
        with (app / "Contents/Info.plist").open("rb") as file:
            info = plistlib.load(file)
        if (info.get("CFBundleIdentifier") != "com.starclip.desktop"
                or info.get("CFBundleShortVersionString") != expected
                or info.get("CFBundleExecutable") != "starclip"):
            raise RuntimeError("DMG application identity/version/executable mismatch")
        binary = app / "Contents/MacOS/starclip"
        if not binary.resolve(strict=True).is_relative_to(app.resolve(strict=True)):
            raise RuntimeError("Application executable escapes its bundle")
        architectures = command("lipo", "-archs", str(binary), capture_output=True, text=True).stdout.split()
        if architectures != ["arm64"]:
            raise RuntimeError("DMG application must contain the expected arm64 architecture")
        command("codesign", "--verify", "--deep", "--strict", str(app))

    def install(dmg, expected):
        mount = Path(tempfile.mkdtemp(prefix="mount-", dir=scratch))
        stage = Path(tempfile.mkdtemp(prefix="stage-", dir=scratch)) / "Starclip.app"
        command("hdiutil", "attach", "-readonly", "-nobrowse", "-mountpoint", str(mount), str(dmg))
        try:
            verify_app(mount / "Starclip.app", expected)
            command("ditto", str(mount / "Starclip.app"), str(stage))
        finally:
            command("hdiutil", "detach", str(mount))
        verify_app(stage, expected)
        if installation.exists():
            installation.rename(scratch / "previous-0.1.0.app")
        stage.rename(installation)
        print(f"DMG installation verified: {expected}", flush=True)

    def start_window():
        nonlocal process
        # Write an original canary; never inspect clipboard contents.
        command("pbcopy", input=b"Starclip CI original clipboard canary: a quiet orbit.")
        process = subprocess.Popen([str(installation / "Contents/MacOS/starclip")],
                                   cwd=scratch, start_new_session=True,
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            if process.poll() is not None:
                raise RuntimeError("Installed application exited during startup")
            state = json.loads(command(str(inspector), str(process.pid), capture_output=True, text=True).stdout)
            if state["window"] and database.is_file():
                # 0.1.0 ignores the initial clipboard value. Change the canary
                # after its watcher has started to catch an unpaused regression.
                time.sleep(1)
                command("pbcopy", input=b"Starclip CI second original canary: a bright orbit.")
                print("Native application and visible main window are ready", flush=True)
                return
            time.sleep(0.5)
        raise RuntimeError("Native main-window evidence or database was not ready; UI smoke is unverified")

    def stop():
        nonlocal process
        if process is not None and process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=10)
        process = None

    target_schema = source_schema(scripts.parent)

    def check(mode, schema):
        if mode == "upgrade" and (scripts.parent / "src-tauri/src/storage/recovery_cleanup.rs").is_file():
            mode = "upgrade-cleaned"
        command("python3", str(scripts / "installer-fixture.py"), str(database), mode, str(schema))

    try:
        if args.scenario == "upgrade":
            install(baseline, "0.1.0")
            start_window()
            for _ in range(3):
                time.sleep(1)
                check("fresh", 0)
            stop()
            check("seed", 0)
        install(package, args.version)
        start_window()
        for _ in range(5):
            time.sleep(1)
            if process.poll() is not None:
                raise RuntimeError("Installed application exited during observation")
            check(args.scenario, target_schema)
        stop()
        check(args.scenario, target_schema)
        summary = f"- macOS native {args.scenario} smoke passed: DMG {args.version}, visible main window, continuous canary exclusion and fixed database checks. Absent fresh settings use the verified paused default. No UI buttons or in-app updater were exercised.\n"
        with Path(os.environ["GITHUB_STEP_SUMMARY"]).open("a") as file:
            file.write(summary)
        print(summary.strip())
    finally:
        stop()


if __name__ == "__main__":
    main()
