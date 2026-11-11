#!/usr/bin/env python3
"""Real updater UI on a disposable GitHub macOS ARM runner, never a user Mac.

Release mode defaults to genuine public 0.2.4. Older public packages and tag source with
comparison metadata 0.2.2 remain explicit baselines for signed draft artifacts
over a runner-local trusted HTTPS fixture. Historical 0.2.2 may require explicitly recorded manual recovery;
that never counts as an automatic restart pass or public GitHub delivery test.
Without --fixture, retain the legacy comparison-0.2.1 to public-0.2.2 experiment.
"""
import argparse
import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import plistlib
import re
import signal
import socket
import sqlite3
import subprocess
import sys
import tarfile
import tempfile
import time

from updater_baselines import PUBLIC_BASELINES, public_baseline


IDENTIFIER = "com.starclip.desktop"
COMPARISON = "0.2.1"
TARGET = "0.2.2"
ENDPOINT = "https://github.com/StarDreamTech/starclip/releases/latest/download/latest.json"
MANIFEST_HASH = "10d0e5646027d4bd0804446b477bd6fafc10f232b0d3135b44b46be3595a706d"
TARGET_URL = "https://github.com/StarDreamTech/starclip/releases/download/v0.2.2/Starclip_0.2.2_aarch64.app.tar.gz"
TARGET_HASH = "83113a8c37739335cbc11d905c5b3bc089bba4aa0ad9b380a5d39199b63d224e"
PUBLIC_KEY = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDVEQjQ4MUM1NTgxMDBGRjEKUldUeER4Qll4WUcwWFpqaHJOVDZvRGxTcXNZK3FVdGVpTnFxMVNJbmgyRmFNeUVtVFRJcCtVaGEK"
SETTINGS = dict(paused=True, pauseRevision=0, uiLanguage="system", theme="forest", layout="list",
                maxHistory=0, shortcut="CommandOrControl+Shift+V", alwaysOnTop=False,
                retentionDays=0, showStartup=False, stickerPack="none")
NOTE, TITLE, GROUP = "Starclip CI original note: a quiet orbit.", "CI original note", "CI original group"


def command(*args, **kwargs):
    return subprocess.run(args, check=True, timeout=120, **kwargs)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def fresh_runner():
    if (platform.system() != "Darwin" or platform.machine() != "arm64"
            or os.environ.get("GITHUB_ACTIONS") != "true"
            or os.environ.get("RUNNER_ENVIRONMENT") != "github-hosted"
            or os.environ.get("GITHUB_REPOSITORY") != "StarDreamTech/starclip"):
        raise RuntimeError("This experiment requires a disposable GitHub-hosted macOS ARM runner")
    data = Path.home() / "Library/Application Support" / IDENTIFIER
    for path in (data, Path.home() / "Library/Caches" / IDENTIFIER,
                 Path.home() / "Library/WebKit" / IDENTIFIER,
                 Path.home() / "Library/Preferences" / (IDENTIFIER + ".plist"),
                 Path("/Applications/Starclip.app"), Path.home() / "Applications/Starclip.app"):
        if path.exists() or path.is_symlink():
            raise RuntimeError("Existing Starclip installation or profile; refusing this experiment")
    if subprocess.run(["pgrep", "-ix", "starclip"], capture_output=True).returncode != 1:
        raise RuntimeError("A Starclip process exists, or process discovery failed")
    config = json.loads((Path(__file__).resolve().parent.parent / "src-tauri/tauri.conf.json").read_text())
    updater = config["plugins"]["updater"]
    if (config["identifier"] != IDENTIFIER or updater["pubkey"] != PUBLIC_KEY
            or updater["endpoints"] != [ENDPOINT] or updater["requireSignedVersion"] is not True):
        raise RuntimeError("Production identity, key, endpoint or signed-version enforcement changed")
    return Path(os.environ["RUNNER_TEMP"]).resolve(strict=True), data


def verify_database(path, expected_schema):
    with contextlib.closing(sqlite3.connect(path.as_uri() + "?mode=ro", uri=True, timeout=5)) as db, db:
        assert db.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
        assert db.execute("PRAGMA user_version").fetchone()[0] == expected_schema, "Unexpected baseline/target database schema"
        assert json.loads(db.execute("SELECT value FROM preferences WHERE key='settings'").fetchone()[0]) == SETTINGS
        assert db.execute("SELECT content,title,kind,pinned,keep_forever,group_id,copy_count FROM clips").fetchall() == [(NOTE, TITLE, "text", 1, 0, 1001, 7)], "Fixed record changed or canary captured"
        assert db.execute("SELECT id,name,color FROM groups").fetchall() == [(1001, GROUP, "#7BBEB0")]


def seed_database(path, expected_schema):
    assert path.is_file(), "Only the application may create its synthetic database"
    with contextlib.closing(sqlite3.connect(path.as_uri() + "?mode=rw", uri=True, timeout=5)) as db, db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == expected_schema
        assert db.execute("SELECT count(*) FROM clips").fetchone()[0] == 0
        assert db.execute("SELECT count(*) FROM groups").fetchone()[0] == 0
        now = int(time.time() * 1000)
        with db:
            db.execute("INSERT INTO groups(id,name,color) VALUES(1001,?,?)", (GROUP, "#7BBEB0"))
            db.execute("INSERT INTO clips(id,content,title,kind,pinned,group_id,created_at,updated_at,copy_count) VALUES(1001,?,?,?,1,1001,?,?,7)", (NOTE, TITLE, "text", now, now))
            db.execute("INSERT INTO preferences(key,value) VALUES('settings',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", (json.dumps(SETTINGS),))
            db.execute("INSERT INTO preferences(key,value) VALUES('onboarding_v1','true') ON CONFLICT(key) DO UPDATE SET value=excluded.value")
        db.execute("PRAGMA wal_checkpoint(TRUNCATE)")
    verify_database(path, expected_schema)


def verify_app(app, version):
    info = plistlib.loads((app / "Contents/Info.plist").read_bytes())
    assert (info.get("CFBundleIdentifier"), info.get("CFBundleShortVersionString"), info.get("CFBundleExecutable")) == (IDENTIFIER, version, "starclip")
    binary = app / "Contents/MacOS/starclip"
    assert binary.resolve(strict=True).is_relative_to(app.resolve(strict=True))
    assert command("lipo", "-archs", str(binary), capture_output=True, text=True).stdout.split() == ["arm64"]
    command("codesign", "--verify", "--deep", "--strict", str(app))
    return digest(binary)


def baseline_comparison(origin, baseline_schema, target_schema, *, release_mode, allow_manual_recovery):
    if origin in PUBLIC_BASELINES:
        assert release_mode, "Public baselines require an authentic signed release fixture"
        expected = public_baseline(origin, "darwin-aarch64")
        assert baseline_schema == expected["schema"], "The immutable public baseline schema changed"
        comparison = expected["version"]
    else:
        assert origin == "source-comparison"
        assert baseline_schema == (target_schema if release_mode else 2), "Source-comparison baseline must use the exact source schema"
        comparison = "0.2.2" if release_mode else "0.2.1"
    assert not allow_manual_recovery or (release_mode and origin == "public-0.2.2"), "Manual recovery is only allowed for the historical public 0.2.2 baseline"
    return comparison


@contextlib.contextmanager
def fixture_transport(directory, temporary):
    """Temporarily trust and route only this disposable runner's update traffic."""
    assert directory.resolve(strict=True).is_relative_to(temporary)
    evidence = temporary / "starclip-macos-updater-evidence"

    def event(stage, **details):
        entry = dict(stage=stage, timestamp=time.time(), **details)
        with (evidence / "transport-stages.jsonl").open("a", encoding="utf-8") as file:
            file.write(json.dumps(entry) + "\n")
        print("HTTPS fixture: " + stage, flush=True)

    def transport_command(stage, *arguments):
        event(stage + "-started")
        result = subprocess.run(arguments, check=True, capture_output=True, text=True, timeout=30)
        event(stage + "-finished")
        return result

    helper = Path(__file__).with_name("release-updater-fixture.py")
    hosts = Path("/etc/hosts")
    before = hosts.read_bytes()
    saved = directory / "hosts.original"
    saved.write_bytes(before)
    saved.chmod(0o600)
    mapped = directory / "hosts.fixture"
    mapped.write_bytes(before + b"\n127.0.0.1 github.com # starclip-disposable-release-fixture\n")
    certificate = directory / "ca.crt"
    fingerprint = command("openssl", "x509", "-in", str(certificate), "-noout", "-fingerprint", "-sha1", capture_output=True, text=True).stdout.strip().split("=")[-1].replace(":", "")
    assert re.fullmatch(r"[A-Fa-f0-9]{40}", fingerprint)
    server = None
    server_log = (evidence / "fixture-server.log").open("ab", buffering=0)
    changed_hosts = False
    try:
        transport_command("install-trust", "sudo", "-n", "security", "add-trusted-cert", "-d", "-r", "trustRoot", "-p", "ssl",
                "-k", "/Library/Keychains/System.keychain", str(certificate))
        event("start-server")
        server = subprocess.Popen(["sudo", "-n", "-E", sys.executable, str(helper), "serve", "--directory", str(directory)],
                                  start_new_session=True, stdout=server_log, stderr=server_log)
        transport_command("map-host", "sudo", "-n", "cp", str(mapped), str(hosts))
        changed_hosts = True
        transport_command("flush-dns-cache", "sudo", "-n", "dscacheutil", "-flushcache")
        transport_command("restart-dns-responder", "sudo", "-n", "killall", "mDNSResponder")
        event("system-host-resolution", addresses=sorted({item[4][0] for item in socket.getaddrinfo("github.com", 443, type=socket.SOCK_STREAM)}),
              curlConfigPresent=(Path.home() / ".curlrc").is_file(), hostsFile=str(hosts.resolve()), hostsMode=oct(hosts.stat().st_mode & 0o777),
              curlVersion=command("/usr/bin/curl", "-q", "--version", capture_output=True, text=True).stdout.splitlines()[0])
        event("probe-trusted-https", serverPid=server.pid)
        deadline = time.monotonic() + 20
        last_probe = None
        while time.monotonic() < deadline:
            if server.poll() is not None:
                raise RuntimeError("Runner-local HTTPS fixture exited during startup")
            probe = subprocess.run(["/usr/bin/curl", "-q", "--fail", "--silent", "--show-error", "--max-time", "2",
                                    "--noproxy", "github.com",
                                    "--write-out", "%{stderr}\nhttp_status=%{http_code} remote_ip=%{remote_ip}\n",
                                    ENDPOINT], capture_output=True, timeout=5)
            last_probe = dict(returncode=probe.returncode, responseSha256=hashlib.sha256(probe.stdout).hexdigest(),
                              diagnostic=probe.stderr.decode("utf-8", errors="replace"))
            if probe.returncode == 0 and hashlib.sha256(probe.stdout).hexdigest() == MANIFEST_HASH:
                event("trusted-https-ready", **last_probe)
                break
            time.sleep(0.5)
        else:
            event("trusted-https-probe-failed", **(last_probe or {}))
            raise RuntimeError("Trusted HTTPS fixture was not reachable with normal certificate validation")
        yield
    finally:
        event("cleanup-started")
        errors = []
        def cleanup_command(*arguments):
            try:
                event("cleanup-command", command=list(arguments[:4]))
                return subprocess.run(arguments, check=False, capture_output=True, text=True, timeout=30)
            except Exception as error:
                errors.append(str(error))
                return None
        # Removing the last admin trust entry can request authorization UI on
        # the hosted runner. Revoke this CA with a standard deny rule instead;
        # that one deny record remains until this disposable VM is destroyed.
        # Do not change SIP or authorization rules to make removal succeed.
        event("revoke-fixture-trust")
        revoked = cleanup_command("sudo", "-n", "security", "add-trusted-cert", "-d", "-r", "deny", "-p", "ssl", str(certificate))
        rejection = None
        if revoked is not None and revoked.returncode == 0 and server is not None and server.poll() is None and changed_hosts:
            # The same server and hosts mapping are still active. Curl 60
            # specifically means certificate verification failed, unlike a
            # stopped server, HTTP rejection, or any disabled TLS validation.
            rejection = cleanup_command("/usr/bin/curl", "-q", "--fail", "--silent", "--show-error", "--max-time", "5",
                                        "--resolve", "github.com:443:127.0.0.1", "--noproxy", "github.com",
                                        "--write-out", "%{stderr}\nhttp_status=%{http_code} remote_ip=%{remote_ip}\n", ENDPOINT)
        trust_revoked = revoked is not None and revoked.returncode == 0 and rejection is not None and rejection.returncode == 60
        event("revoked-tls-probe", trustRevoked=trust_revoked,
              returncode=rejection.returncode if rejection is not None else None,
              diagnostic=rejection.stderr if rejection is not None else None)
        try:
            if changed_hosts:
                transport_command("restore-hosts", "sudo", "-n", "cp", str(saved), str(hosts))
                transport_command("restore-dns-cache", "sudo", "-n", "dscacheutil", "-flushcache")
                subprocess.run(["sudo", "-n", "killall", "-HUP", "mDNSResponder"], check=False, capture_output=True, timeout=30)
            assert hosts.read_bytes() == before, "Temporary hosts mapping was not restored"
        except Exception as error:
            errors.append(str(error))
        try:
            if server is not None:
                event("stop-server", serverPid=server.pid)
                subprocess.run(["sudo", "/bin/kill", "-TERM", "-" + str(server.pid)], check=False, capture_output=True, timeout=30)
                try:
                    server.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    subprocess.run(["sudo", "/bin/kill", "-KILL", "-" + str(server.pid)], check=False, capture_output=True, timeout=30)
                    server.wait(timeout=10)
        except Exception as error:
            errors.append(str(error))
        # Exact certificate/key removal still runs if another cleanup failed.
        deleted = cleanup_command("sudo", "-n", "security", "delete-certificate", "-Z", fingerprint, "/Library/Keychains/System.keychain")
        remaining = cleanup_command("security", "find-certificate", "-a", "-Z", "/Library/Keychains/System.keychain")
        certificate_absent = remaining is not None and remaining.returncode == 0 and fingerprint.upper() not in remaining.stdout.upper()
        for name in ("ca.key", "server.key"):
            try:
                (directory / name).unlink(missing_ok=True)
            except OSError as error:
                errors.append(str(error))
        server_log.close()
        (evidence / "transport-cleanup.json").write_text(json.dumps(dict(hostsRestored=hosts.read_bytes() == before,
                                           serverStopped=server is None or server.poll() is not None,
                                           certificateAbsent=certificate_absent,
                                           trustRevoked=trust_revoked,
                                           trustRevocationStatus=revoked.returncode if revoked is not None else None,
                                           revokedTlsProbeStatus=rejection.returncode if rejection is not None else None,
                                           temporaryDenyRecordUntilVmDisposal=revoked is not None and revoked.returncode == 0,
                                           privateKeysRemoved=not any((directory / name).exists() for name in ("ca.key", "server.key")),
                                           certificateRemovalStatus=deleted.returncode if deleted is not None else None,
                                           errors=errors), indent=2) + "\n")
        event("cleanup-finished", errors=errors)
        if errors or not certificate_absent or not trust_revoked or deleted is None or deleted.returncode != 0:
            raise RuntimeError("Disposable HTTPS fixture cleanup did not fully succeed; inspect transport-cleanup.json")


def main():
    global COMPARISON, TARGET, MANIFEST_HASH, TARGET_URL, TARGET_HASH
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preflight", action="store_true")
    parser.add_argument("--app", type=Path)
    parser.add_argument("--inspector", type=Path)
    parser.add_argument("--fixture", type=Path)
    parser.add_argument("--baseline-origin", choices=("source-comparison", *PUBLIC_BASELINES))
    parser.add_argument("--baseline-schema", type=int, help="Exact baseline schema; defaults to the pinned genuine public baseline metadata")
    parser.add_argument("--allow-manual-recovery", action="store_true")
    args = parser.parse_args()
    args.baseline_origin = args.baseline_origin or ("public-0.2.4" if args.fixture else "source-comparison")
    if args.baseline_schema is None:
        args.baseline_schema = PUBLIC_BASELINES.get(args.baseline_origin, {}).get("schema", 2)
    runner_temp, data = fresh_runner()
    if args.preflight:
        print("Disposable runner and absent Starclip profile verified")
        return
    if not args.app or not args.inspector:
        parser.error("--app and --inspector are required outside preflight")
    release_fixture = None
    if args.fixture:
        args.fixture = args.fixture.resolve(strict=True)
        assert args.fixture.is_relative_to(runner_temp)
        release_fixture = json.loads((args.fixture / "fixture.json").read_text())
        assert release_fixture["platform"] == "darwin-aarch64" and release_fixture["publicKey"] == PUBLIC_KEY
        assert release_fixture["endpoint"] == ENDPOINT
        TARGET = release_fixture["targetVersion"]
        MANIFEST_HASH = release_fixture["latestManifestSha256"]
        TARGET_URL, TARGET_HASH = release_fixture["targetUrl"], release_fixture["targetSha256"]
    baseline_schema = args.baseline_schema
    target_schema = release_fixture["targetSchema"] if release_fixture else 2
    assert type(target_schema) is int and target_schema > 0 and baseline_schema > 0
    COMPARISON = baseline_comparison(args.baseline_origin, baseline_schema, target_schema,
                                     release_mode=bool(release_fixture), allow_manual_recovery=args.allow_manual_recovery)
    assert tuple(map(int, TARGET.split("."))) > tuple(map(int, COMPARISON.split("."))), "The signed target must be newer than the selected baseline"
    baseline_package = (public_baseline(args.baseline_origin, "darwin-aarch64")
                        if args.baseline_origin in PUBLIC_BASELINES else None)
    inspector = args.inspector.resolve(strict=True)
    assert inspector.is_relative_to(runner_temp), "UI helper must be runner-owned"
    command(str(inspector), "preflight")
    evidence = runner_temp / "starclip-macos-updater-evidence"
    evidence.mkdir(exist_ok=True)
    assert not (evidence / "result.json").exists(), "Acceptance evidence from an earlier run exists"
    scratch = Path(tempfile.mkdtemp(prefix="starclip-macos-updater-", suffix=".noindex", dir=runner_temp))
    installation = scratch / "Starclip.app"
    database = data / "starclip.sqlite3"
    binary = installation / "Contents/MacOS/starclip"
    process, active_pid = None, None
    stage = "preflight"
    baseline_hash = ""
    restart_mode = "automatic"
    transport = contextlib.ExitStack()
    app_error_log = (evidence / "synthetic-app-stderr.log").open("ab", buffering=0)

    def write(name, result):
        (evidence / name).write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n")

    def pinned_manifest():
        raw = command("/usr/bin/curl", "-q", "--fail", "--silent", "--show-error", "--location", "--retry", "3", "--max-time", "60", "--noproxy", "github.com", ENDPOINT, capture_output=True).stdout
        assert hashlib.sha256(raw).hexdigest() == MANIFEST_HASH, "Update manifest differs from the pinned acceptance target"
        manifest = json.loads(raw)
        assert manifest["version"] == TARGET
        assert manifest["platforms"]["darwin-aarch64"]["url"] == TARGET_URL
        assert manifest["platforms"]["darwin-aarch64"]["signature"]
        write("latest.json", manifest)

    def ui(operation, *arguments):
        return json.loads(command(str(inspector), operation, *map(str, arguments), capture_output=True, text=True).stdout)

    def wait_ui(pattern, *, seconds=60, press=False, text=False, confirmation=False):
        flags = (["--text"] if text else []) + (["--confirmation"] if confirmation else [])
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if ui("find", active_pid, pattern, *flags)["found"]:
                if press:
                    assert ui("press", active_pid, pattern, *flags)["found"]
                return
            time.sleep(0.5)
        raise RuntimeError("Expected UI was not ready: " + pattern)

    def wait_window():
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            if ui("inspect", active_pid)["window"] and database.is_file():
                return
            time.sleep(0.5)
        raise RuntimeError("The owned native main window did not appear")

    def start():
        nonlocal process, active_pid
        # Only write original synthetic canaries; do not inspect a clipboard.
        command("pbcopy", input=b"Starclip CI original clipboard canary: a quiet orbit.")
        process = subprocess.Popen([str(binary)], cwd=scratch, start_new_session=True,
                                   stdout=subprocess.DEVNULL, stderr=app_error_log)
        active_pid = process.pid
        wait_window()
        write("accessibility-prepare-" + str(active_pid) + ".json", ui("prepare", active_pid))
        command("pbcopy", input=b"Starclip CI second original canary: a bright orbit.")

    def stop():
        nonlocal active_pid
        owned = ui("processes", installation)["pids"] if installation.exists() else []
        if process is not None and process.poll() is None and process.pid not in owned:
            owned.append(process.pid)
        for pid in owned:
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline and ui("processes", installation)["pids"]:
            time.sleep(0.2)
        for pid in ui("processes", installation)["pids"]:
            os.kill(pid, signal.SIGKILL)
        if process is not None:
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=10)
        active_pid = None

    def fixture(mode):
        def verify(path, expected_schema):
            if release_fixture:
                return json.loads(command(sys.executable, str(Path(__file__).with_name("release-updater-fixture.py")), "verify-data", "--database", str(path), "--expected-schema", str(expected_schema), capture_output=True, text=True).stdout)
            verify_database(path, expected_schema)
            return dict(schema=expected_schema, paused=True, fixedRecords=1, fixedGroups=1)
        details = verify(database, target_schema if mode == "after-install" else baseline_schema)
        def snapshots(reasons):
            found = []
            for path in (data / "recovery").glob("*/snapshot.json"):
                try:
                    manifest = json.loads(path.read_text())
                except FileNotFoundError:
                    continue  # The background cleanup may remove a snapshot during this read.
                if manifest["reason"] in reasons:
                    found.append((path, manifest))
            return found
        points = snapshots(("before-update",))
        if mode == "after-install" and target_schema >= 5:
            # Development schema 5 cleans obsolete automatic snapshots after validation.
            deadline = time.monotonic() + 10
            points = snapshots(("before-update", "before-migration"))
            while points and time.monotonic() < deadline:
                time.sleep(0.2)
                points = snapshots(("before-update", "before-migration"))
            assert not points, "Validated upgrade left obsolete automatic data backups"
            backups = list((data / "application-backups").glob("*/Starclip.app"))
            assert len(backups) == 1, "Expected one independently preserved old application"
            assert verify_app(backups[0], COMPARISON) == baseline_hash, "Original application backup changed"
        elif mode == "after-install":
            assert len(points) == 1, "Expected one pre-update recovery point"
            path, manifest = points[0]
            assert manifest["schemaVersion"] == baseline_schema
            snapshot = path.parent / "starclip.sqlite3"
            assert digest(snapshot) == manifest["files"]["starclip.sqlite3"]["sha256"]
            verify(snapshot, baseline_schema)
            backup = data / "application-backups" / manifest["id"] / "Starclip.app"
            assert verify_app(backup, COMPARISON) == baseline_hash, "Original application backup changed"
        else:
            assert not points, "Recovery point created before user confirmation"
            assert not (data / "application-backups").exists(), "Application backup created before confirmation"
        result = dict(stage=mode, **details, recoveryPoints=len(points))
        write(mode + ".json", result)
        print(json.dumps(result), flush=True)

    try:
        if release_fixture:
            transport.enter_context(fixture_transport(args.fixture, runner_temp))
            write("release-target.json", release_fixture)
        pinned_manifest()
        # Independently pin the expected installed executable. This does not
        # replace the updater's real HTTPS download or its signature checks.
        archive = command("/usr/bin/curl", "-q", "--fail", "--silent", "--show-error", "--location", "--retry", "3", "--max-time", "60", "--noproxy", "github.com", TARGET_URL, capture_output=True).stdout
        assert hashlib.sha256(archive).hexdigest() == TARGET_HASH
        with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as bundle:
            members = [m for m in bundle if m.name.endswith("/Contents/MacOS/starclip") and m.isfile()]
            assert len(members) == 1
            target_executable_hash = hashlib.sha256(bundle.extractfile(members[0]).read()).hexdigest()
        stage = "fresh-launch-and-seed"
        verify_app(args.app.resolve(strict=True), COMPARISON)
        command("ditto", str(args.app), str(installation))
        baseline_hash = verify_app(installation, COMPARISON)
        if baseline_package:
            assert baseline_hash == baseline_package["executableSha256"], "Baseline is not the selected genuine public macOS executable"
        start()
        time.sleep(2)
        command("python3", str(Path(__file__).with_name("installer-fixture.py")), str(database), "fresh", str(baseline_schema))
        stop()
        if release_fixture:
            command(sys.executable, str(Path(__file__).with_name("release-updater-fixture.py")), "seed-data", "--database", str(database), "--expected-schema", str(baseline_schema))
        else:
            seed_database(database, baseline_schema)
        start()
        original_pid = active_pid
        wait_ui(r"^(?:开启剪贴记录|Start recording)$")
        fixture("before-check")
        stage = "check"
        wait_ui(r"^(?:偏好设置|Preferences)(?:\s|$)", press=True)
        wait_ui(r"^(?:帮助|Help)$", press=True)
        wait_ui(r"^(?:检查更新|Check for updates)$", press=True)
        wait_ui(r"^(?:新版本|Version)\s*" + re.escape(TARGET) + r"(?:\s|$)", text=True)
        pinned_manifest()
        stage = "download-and-verify"
        wait_ui(r"^(?:下载更新|Download update)$", press=True)
        wait_ui(r"^(?:稍后再说|Later)$", seconds=180)
        # Ready is reached only after the genuine updater verifies the public
        # signature and bound version; no IPC or network result is injected.
        fixture("ready-before-confirmation")
        stage = "later-keeps-current-app"
        wait_ui(r"^(?:稍后再说|Later)$", press=True)
        time.sleep(2)
        assert not ui("find", active_pid, r"^(?:稍后再说|Later)$")["found"]
        assert digest(binary) == baseline_hash
        assert process.poll() is None and active_pid == original_pid
        fixture("before-install")
        stage = "confirm-install"
        pinned_manifest()
        wait_ui(r"^(?:安装并重启|Install and restart)$", press=True)
        wait_ui(r"^(?:稍后再说|Later)$")
        wait_ui(r"^(?:安装并重启|Install and restart)$", press=True, confirmation=True)
        stage = "automatic-restart"
        deadline = time.monotonic() + 180
        new_pids = []
        while time.monotonic() < deadline:
            new_pids = [pid for pid in ui("processes", installation)["pids"] if pid != original_pid]
            if process.poll() is not None and len(new_pids) == 1:
                break
            time.sleep(0.5)
        if process.poll() is None or len(new_pids) != 1:
            assert args.allow_manual_recovery and process.poll() is not None and not new_pids, "No unique automatic restart at original install path"
            # Historical public 0.2.2 skips normal Exit cleanup. Explicitly
            # distinguish opening the already-installed target from auto restart.
            assert verify_app(installation, TARGET) == target_executable_hash
            fixture("after-install")
            write("manual-recovery.json", dict(reason="Public 0.2.2 installed the target but did not keep a restarted process", oldProcessReturnCode=process.poll(), automaticRestart=False))
            restart_mode = "manual-recovery"
            start()
        else:
            active_pid = new_pids[0]
        installed_hash = verify_app(installation, TARGET)
        assert installed_hash == target_executable_hash and installed_hash != baseline_hash
        stage = "restarted-paused-with-fixed-data"
        wait_window()
        write("accessibility-prepare-" + str(active_pid) + ".json", ui("prepare", active_pid))
        wait_ui(r"^(?:开启剪贴记录|Start recording)$")
        command("pbcopy", input=b"Starclip CI post-update original canary: a new orbit.")
        for _ in range(3):
            time.sleep(1)
            fixture("after-install")
        restarted_pid = active_pid
        stop()
        fixture("after-install")
        write("result.json", dict(result="passed" if restart_mode == "automatic" else "passed-with-manual-recovery",
                                  restartMode=restart_mode, baselineOrigin=args.baseline_origin,
                                  baselinePackage=baseline_package,
                                  sourceCommit=release_fixture["sourceHeadSha"] if release_fixture else os.environ["GITHUB_SHA"], comparisonVersion=COMPARISON,
                                  targetVersion=TARGET, manifestSha256=MANIFEST_HASH, targetArchiveSha256=TARGET_HASH,
                                  originalPid=original_pid, restartedPid=restarted_pid,
                                  originalExecutableSha256=baseline_hash, installedExecutableSha256=installed_hash))
        with Path(os.environ["GITHUB_STEP_SUMMARY"]).open("a") as file:
            file.write(f"- macOS native updater: signed download, confirmation, data/application recovery copies, replacement and preserved fixed data passed. Restart mode: **{restart_mode}**. Baseline: {args.baseline_origin} ({COMPARISON}); target: {TARGET}.\n")
            if release_fixture:
                file.write("- Authentic signed release artifacts were served over a runner-local trusted HTTPS fixture at the original URLs. Public GitHub delivery is not covered. Manual recovery, if recorded, does not count as an automatic restart pass.\n")
            else:
                file.write("- The source only overrides comparison metadata; target is pinned public 0.2.2, not a future target release.\n")
    except Exception as error:
        write("result.json", dict(result="failed", stage=stage, message=str(error)))
        diagnostic = dict(oldProcessReturnCode=process.poll() if process else None)
        try:
            diagnostic["runningApplications"] = ui("diagnose-processes", installation)
            diagnostic["matchedPids"] = ui("processes", installation)["pids"]
            # Inspect only this scratch directory's executable paths, never
            # arguments/environment or processes from another user profile.
            rows = command("ps", "-ax", "-o", "pid=,comm=", capture_output=True, text=True).stdout.splitlines()
            diagnostic["scratchProcesses"] = [row.strip() for row in rows if str(scratch) + "/" in row]
            if installation.exists():
                info = plistlib.loads((installation / "Contents/Info.plist").read_bytes())
                diagnostic["installedVersion"] = info.get("CFBundleShortVersionString")
                diagnostic["installedExecutableSha256"] = digest(binary)
                diagnostic["expectedExecutableSha256"] = target_executable_hash
        except Exception as diagnostic_error:
            diagnostic["diagnosticError"] = str(diagnostic_error)
        write("failure-diagnostics.json", diagnostic)
        if stage == "automatic-restart":
            try:
                # Preserve the independent backup/data result even when
                # automatic restart fails. This does not mark the run passed.
                fixture("after-install")
            except Exception as fixture_error:
                write("after-install-error.json", dict(message=str(fixture_error)))
        if active_pid:
            try:
                write("owned-window-ax.json", ui("dump", active_pid))
            except Exception:
                pass
        raise
    finally:
        try:
            stop()
        finally:
            try:
                transport.close()
            finally:
                app_error_log.close()
        # VM disposal removes the test app and profile. Evidence deliberately
        # excludes databases, application backups, clipboard and credentials.


if __name__ == "__main__":
    main()
