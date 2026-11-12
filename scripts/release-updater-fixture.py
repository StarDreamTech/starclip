#!/usr/bin/env python3
"""Disposable-runner HTTPS/data fixtures for authentic signed release artifacts.

The caller must bind the downloaded complete-release artifact to a successful
release.yml run. The run head is either the tag commit or a descendant whose
compare diff stays outside application source. Package bytes must match the
published release. This server does not exercise GitHub's public download
infrastructure. It never disables TLS or updater signatures.
"""
import argparse
from contextlib import closing
import base64
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import re
import sqlite3
import ssl
import struct
import subprocess
import time
from urllib.parse import urlsplit
import zlib
from fixture_schema import source_schema

PUBLIC_KEY = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDVEQjQ4MUM1NTgxMDBGRjEKUldUeER4Qll4WUcwWFpqaHJOVDZvRGxTcXNZK3FVdGVpTnFxMVNJbmgyRmFNeUVtVFRJcCtVaGEK"
ORGANIZATION_REPOSITORY = "StarDreamTech/starclip"
# Installed clients through 0.2.5 only accept this download prefix. The signed
# organization manifest never uses it; the compatibility repository is a separate mirror.
LEGACY_DOWNLOAD_REPOSITORY = "cpython666/starclip"
ENDPOINT = f"https://github.com/{ORGANIZATION_REPOSITORY}/releases/latest/download/latest.json"
LEGACY_ENDPOINT = f"https://github.com/{LEGACY_DOWNLOAD_REPOSITORY}/releases/latest/download/latest.json"
ORGANIZATION_DOWNLOAD_PREFIX = f"https://github.com/{ORGANIZATION_REPOSITORY}/releases/download/"
LEGACY_DOWNLOAD_PREFIX = f"https://github.com/{LEGACY_DOWNLOAD_REPOSITORY}/releases/download/"
SETTINGS = dict(paused=True, pauseRevision=0, uiLanguage="system", theme="forest", layout="list",
                maxHistory=0, shortcut="CommandOrControl+Shift+V", alwaysOnTop=False,
                retentionDays=0, showStartup=False, stickerPack="none")
GROWTH = dict(version=1, enabled=False, xp=55, day="2026-10-29", completedTasks=["greet"],
              interactionDays=1, playDays=0, lastInteractionDay="2026-10-29",
              achievements=["first-meeting"], unlockedItems=["welcome-star", "moon-glasses"],
              equipment=dict(back=None, face="moon-glasses", hand=None, charm="welcome-star"), revision=4)
PREFERENCES = {"settings": SETTINGS, "starpet_growth_v1": GROWTH,
               "starpet_v1": dict(enabled=False, mood="happy", color="#FFD66E", size=100),
               "telemetry_preferences_v1": dict(revision=0, enabled=False)}
NOTE, TITLE, GROUP = "Starclip CI original note: a quiet orbit.", "CI original note", "CI original group"
PIXELS = bytes([39, 51, 87, 255, 255, 214, 110, 255, 123, 190, 176, 255, 147, 126, 245, 128])
IMAGE_IDENTITY = "sha256:" + hashlib.sha256(struct.pack("<II", 2, 2) + PIXELS).hexdigest()


def png_chunk(kind, body):
    return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))


PNG = (b"\x89PNG\r\n\x1a\n" + png_chunk(b"IHDR", struct.pack(">IIBBBBB", 2, 2, 8, 6, 0, 0, 0))
       + png_chunk(b"IDAT", zlib.compress(b"\0" + PIXELS[:8] + b"\0" + PIXELS[8:]))
       + png_chunk(b"IEND", b""))


def guard_runner():
    if (os.environ.get("GITHUB_ACTIONS") != "true"
            or os.environ.get("RUNNER_ENVIRONMENT") != "github-hosted"
            or os.environ.get("GITHUB_REPOSITORY") != "StarDreamTech/starclip"):
        raise RuntimeError("Only a disposable GitHub-hosted Starclip runner may use these fixtures")
    return Path(os.environ["RUNNER_TEMP"]).resolve(strict=True)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def compatibility_manifest(manifest):
    """Copy of the organization manifest with download URLs rewritten for the compatibility mirror.

    Minisign covers the package bytes, not the URL. The signed latest.json on disk stays unchanged.
    """
    rewritten = json.loads(json.dumps(manifest))
    for item in rewritten["platforms"].values():
        url = str(item.get("url", ""))
        if not url.startswith(ORGANIZATION_DOWNLOAD_PREFIX):
            raise ValueError("Signed release URL is not an organization download address")
        item["url"] = LEGACY_DOWNLOAD_PREFIX + url[len(ORGANIZATION_DOWNLOAD_PREFIX):]
    return rewritten


ALLOWED_DIFF_PREFIXES = (".github/", "docs/", ".agents/")


def is_allowed_diff_path(file):
    if not isinstance(file, str) or not file or len(file) > 512 or "\\" in file or file.startswith("/") or "\0" in file:
        return False
    parts = file.split("/")
    if any(part in ("", ".", "..") for part in parts):
        return False
    return any(file.startswith(prefix) for prefix in ALLOWED_DIFF_PREFIXES)


def validate_published_binding(binding, *, version, source_ref, source_head_sha, release_run_id):
    """Reject a binding that does not record the tag source and an allowed commit match."""
    if not isinstance(binding, dict):
        raise ValueError("Published release binding is missing")
    mode = binding.get("mode")
    if mode not in ("exact", "ci-only"):
        raise ValueError("Unknown release commit match")
    if binding.get("repository") != "StarDreamTech/starclip":
        raise ValueError("Published release binding is for a different repository")
    if binding.get("tag") != source_ref or binding.get("version") != version or source_ref != "v" + version:
        raise ValueError("Published release binding does not match the requested tag")
    tag_sha, run_head, files = binding.get("tagSha"), binding.get("runHeadSha"), binding.get("files")
    if tag_sha != source_head_sha or not re.fullmatch(r"[a-f0-9]{40}", str(run_head or "")):
        raise ValueError("Schema source is not the tag commit recorded in the published binding")
    if binding.get("runId") != release_run_id:
        raise ValueError("Published release binding does not match the release run")
    if mode == "exact" and (run_head != tag_sha or files != []):
        raise ValueError("Exact match disagrees with the release run head")
    if mode == "ci-only" and (run_head == tag_sha or not isinstance(files, list) or not files):
        raise ValueError("CI-only match disagrees with the recorded diff")
    if mode == "ci-only":
        for name in files:
            if not is_allowed_diff_path(name):
                raise ValueError("Recorded diff includes a path outside CI and documentation")
    return binding


def verify_assets(assets, version):
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise ValueError("Expected stable target version")
    expected = {"latest.json", "release-notes.md", f"Starclip_{version}_aarch64.dmg",
                f"Starclip_{version}_aarch64.app.tar.gz", f"Starclip_{version}_aarch64.app.tar.gz.sig",
                f"Starclip_{version}_x64-setup.exe", f"Starclip_{version}_x64-setup.exe.sig"}
    hashes = {}
    for row in (assets / "SHA256SUMS").read_text(encoding="utf-8").splitlines():
        match = re.fullmatch(r"([a-f0-9]{64})  ([A-Za-z0-9_.-]+)", row)
        if not match or match[2] in hashes:
            raise ValueError("Unexpected or duplicate SHA256SUMS entry")
        hashes[match[2]] = match[1]
    if set(hashes) != expected:
        raise ValueError("Incomplete or unexpected signed release artifact set")
    for name, digest in hashes.items():
        file = assets / name
        if file.is_symlink() or not file.is_file() or sha(file) != digest:
            raise ValueError("Release asset checksum mismatch: " + name)
    manifest = json.loads((assets / "latest.json").read_text(encoding="utf-8"))
    if manifest["version"] != version or set(manifest["platforms"]) != {"darwin-aarch64", "windows-x86_64"}:
        raise ValueError("Release manifest target/platform mismatch")
    mapping = {urlsplit(ENDPOINT).path: "latest.json", urlsplit(LEGACY_ENDPOINT).path: "latest.json"}
    for platform, suffix in (("darwin-aarch64", "aarch64.app.tar.gz"), ("windows-x86_64", "x64-setup.exe")):
        name = f"Starclip_{version}_{suffix}"
        item = manifest["platforms"][platform]
        url = f"https://github.com/{ORGANIZATION_REPOSITORY}/releases/download/v{version}/{name}"
        if item["url"] != url or item["signature"] != (assets / (name + ".sig")).read_text(encoding="utf-8").strip():
            raise ValueError("Signed release URL or signature differs from the manifest")
        signature = base64.b64decode(item["signature"], validate=True).decode("utf-8").splitlines()
        if len(signature) != 4 or not signature[2].startswith("trusted comment: "):
            raise ValueError("Unexpected Tauri signature format")
        if not re.search(r"(?:^|\t)version:" + re.escape(version) + r"(?:\t|$)", signature[2]):
            raise ValueError("Tauri signature is not bound to the requested version")
        mapping[urlsplit(url).path] = name
        mapping[urlsplit(LEGACY_DOWNLOAD_PREFIX + url[len(ORGANIZATION_DOWNLOAD_PREFIX):]).path] = name
    return manifest, hashes, mapping


def prepare(args, temporary):
    assets = args.assets.resolve(strict=True)
    output = args.output.resolve()
    if not assets.is_relative_to(temporary) or not output.is_relative_to(temporary) or output.exists():
        raise ValueError("Assets and a new private output folder must be inside RUNNER_TEMP")
    if args.source_ref != "v" + args.version or not re.fullmatch(r"[a-f0-9]{40}", args.source_head_sha):
        raise ValueError("Tag and tag commit are required")
    source = args.source_directory.resolve(strict=True)
    actual_head = subprocess.run(["git", "-C", str(source), "rev-parse", "HEAD"], check=True, capture_output=True, text=True, timeout=15).stdout.strip()
    if actual_head != args.source_head_sha:
        raise ValueError("Target schema source is not the tag commit")
    binding_path = args.published_binding.resolve(strict=True)
    if not binding_path.is_relative_to(temporary):
        raise ValueError("Published binding must be inside RUNNER_TEMP")
    binding = validate_published_binding(json.loads(binding_path.read_text(encoding="utf-8")),
                                         version=args.version, source_ref=args.source_ref,
                                         source_head_sha=args.source_head_sha, release_run_id=args.release_run_id)
    verified = subprocess.run(["node", str(Path(__file__).resolve().parent / "release-run-binding.mjs"),
                               "verify-assets", "--binding", str(binding_path), "--assets", str(assets)],
                              check=False, capture_output=True, text=True, timeout=120)
    if verified.returncode != 0:
        raise ValueError((verified.stderr or verified.stdout).strip() or "Published asset verification failed")
    if verified.stdout.strip():
        print(verified.stdout.strip())
    target_schema = source_schema(source)
    manifest, hashes, mapping = verify_assets(assets, args.version)
    output.mkdir(mode=0o700)
    config = output / "certificate.cnf"
    config.write_text("""[req]
distinguished_name = dn
prompt = no
[dn]
CN = Starclip disposable runner fixture
[ca]
basicConstraints = critical,CA:TRUE,pathlen:0
keyUsage = critical,keyCertSign,cRLSign
subjectKeyIdentifier = hash
[server]
basicConstraints = critical,CA:FALSE
keyUsage = critical,digitalSignature,keyEncipherment
extendedKeyUsage = serverAuth
subjectAltName = DNS:github.com
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid,issuer
""", encoding="utf-8")
    def openssl(*arguments):
        subprocess.run(["openssl", *map(str, arguments)], check=True, capture_output=True, timeout=60)
    openssl("req", "-x509", "-newkey", "rsa:2048", "-nodes", "-sha256", "-days", "2", "-config", config,
            "-extensions", "ca", "-keyout", output / "ca.key", "-out", output / "ca.crt")
    openssl("req", "-newkey", "rsa:2048", "-nodes", "-sha256", "-config", config,
            "-subj", "/CN=github.com", "-keyout", output / "server.key", "-out", output / "server.csr")
    openssl("x509", "-req", "-in", output / "server.csr", "-CA", output / "ca.crt", "-CAkey", output / "ca.key",
            "-CAcreateserial", "-days", "2", "-sha256", "-extfile", config, "-extensions", "server", "-out", output / "server.crt")
    openssl("verify", "-CAfile", output / "ca.crt", output / "server.crt")
    for file in output.iterdir():
        file.chmod(0o600)
    target = manifest["platforms"][args.platform]
    filename = Path(urlsplit(target["url"]).path).name
    metadata = dict(latestManifestSha256=hashes["latest.json"], targetVersion=args.version,
                    platform=args.platform, targetSchema=target_schema, targetFilename=filename, targetSha256=hashes[filename],
                    targetUrl=target["url"], releaseRunId=args.release_run_id, sourceRef=args.source_ref,
                    sourceHeadSha=args.source_head_sha, commitMatch=binding["mode"],
                    releaseRunHeadSha=binding["runHeadSha"], publicKey=PUBLIC_KEY, endpoint=ENDPOINT,
                    assetDirectory=str(assets), mapping=mapping, hashes=hashes,
                    transport="runner-local HTTPS fixture; public GitHub delivery not exercised")
    (output / "fixture.json").write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
    (output / "fixture.json").chmod(0o600)
    print(json.dumps({key: value for key, value in metadata.items() if key not in ("mapping", "hashes", "assetDirectory")}))


def serve(directory, temporary):
    directory = directory.resolve(strict=True)
    if not directory.is_relative_to(temporary):
        raise ValueError("Fixture directory must be runner-owned")
    metadata = json.loads((directory / "fixture.json").read_text(encoding="utf-8"))
    assets = Path(metadata["assetDirectory"]).resolve(strict=True)
    if not assets.is_relative_to(temporary):
        raise ValueError("Fixture assets must be runner-owned")
    _, hashes, mapping = verify_assets(assets, metadata["targetVersion"])
    if hashes != metadata["hashes"] or mapping != metadata["mapping"]:
        raise ValueError("Prepared fixture assets changed")

    legacy_manifest_path = urlsplit(LEGACY_ENDPOINT).path

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            if self.headers.get("Host") not in ("github.com", "github.com:443") or self.path not in mapping:
                self.send_error(404)
                return
            path = assets / mapping[self.path]
            if self.path == legacy_manifest_path:
                body = (json.dumps(compatibility_manifest(json.loads(path.read_text(encoding="utf-8"))), indent=2) + "\n").encode()
                content_type, length = "application/json", len(body)
            else:
                body, content_type, length = None, ("application/json" if path.name == "latest.json" else "application/octet-stream"), path.stat().st_size
            self.send_response(200)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(length))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            if body is not None:
                self.wfile.write(body)
                return
            with path.open("rb") as file:
                while block := file.read(1024 * 1024):
                    self.wfile.write(block)

        def log_message(self, *_):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 443), Handler)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_2
    context.load_cert_chain(directory / "server.crt", directory / "server.key")
    server.socket = context.wrap_socket(server.socket, server_side=True)
    print("Runner-local HTTPS release fixture ready", flush=True)
    server.serve_forever()


def preferences_for_schema(expected_schema):
    # Genuine public 0.2.2 and 0.2.3 used the boolean shape. The schema-4
    # migration normalizes it to mode without changing the explicit opt-out.
    values = dict(PREFERENCES)
    if expected_schema >= 3:
        values["telemetry_preferences_v1"] = dict(revision=0, mode="off")
    return values


def verify_data(path, expected_schema):
    if not path.is_file() or path.is_symlink():
        raise ValueError("Application-created synthetic database is required")
    with closing(sqlite3.connect(path.as_uri() + "?mode=ro", uri=True, timeout=5)) as db, db:
        assert db.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
        assert db.execute("PRAGMA user_version").fetchone()[0] == expected_schema, "Unexpected target/baseline database schema"
        for key, value in preferences_for_schema(expected_schema).items():
            row = db.execute("SELECT value FROM preferences WHERE key=?", (key,)).fetchone()
            assert row is not None and json.loads(row[0]) == value, "Saved preference changed: " + key
        assert db.execute("SELECT id,content,title,kind,pinned,keep_forever,group_id,copy_count FROM clips ORDER BY id").fetchall() == [
            (1001, NOTE, TITLE, "text", 1, 0, 1001, 7),
            (1002, IMAGE_IDENTITY, "CI original four-pixel image", "image", 0, 1, 1001, 0)], "Fixed records changed or canary captured"
        assert db.execute("SELECT id,name,color FROM groups").fetchall() == [(1001, GROUP, "#7BBEB0")]
        assert db.execute("SELECT clip_id,width,height,bytes,png,thumbnail FROM clip_images").fetchall() == [(1002, 2, 2, len(PNG), PNG, PNG)], "Original PNG or thumbnail changed"
    return dict(schema=expected_schema, paused=True, fixedRecords=2, fixedImages=1, fixedGroups=1,
                growthVerified=True, telemetryDisabled=True, imageSha256=hashlib.sha256(PNG).hexdigest())


def seed_data(path, expected_schema):
    if not path.is_file() or path.is_symlink():
        raise ValueError("Only the application may create its synthetic database")
    with closing(sqlite3.connect(path.as_uri() + "?mode=rw", uri=True, timeout=5)) as db, db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == expected_schema, "Unexpected seed baseline schema"
        assert db.execute("SELECT count(*) FROM clips").fetchone()[0] == 0
        assert db.execute("SELECT count(*) FROM groups").fetchone()[0] == 0
        now = int(time.time() * 1000)
        with db:
            db.execute("INSERT INTO groups(id,name,color) VALUES(1001,?,?)", (GROUP, "#7BBEB0"))
            db.execute("INSERT INTO clips(id,content,title,kind,pinned,group_id,created_at,updated_at,copy_count) VALUES(1001,?,?,?,1,1001,?,?,7)", (NOTE, TITLE, "text", now, now))
            db.execute("INSERT INTO clips(id,content,title,kind,keep_forever,group_id,created_at,updated_at) VALUES(1002,?,?,'image',1,1001,?,?)", (IMAGE_IDENTITY, "CI original four-pixel image", now, now))
            db.execute("INSERT INTO clip_images(clip_id,width,height,bytes,png,thumbnail) VALUES(1002,2,2,?,?,?)", (len(PNG), PNG, PNG))
            for key, value in preferences_for_schema(expected_schema).items():
                db.execute("INSERT INTO preferences(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", (key, json.dumps(value)))
            db.execute("INSERT INTO preferences(key,value) VALUES('onboarding_v1','true') ON CONFLICT(key) DO UPDATE SET value=excluded.value")
        db.execute("PRAGMA wal_checkpoint(TRUNCATE)")
    return verify_data(path, expected_schema)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    preparation = commands.add_parser("prepare")
    for name in ("assets", "output"):
        preparation.add_argument("--" + name, type=Path, required=True)
    for name in ("version", "source-ref", "source-head-sha"):
        preparation.add_argument("--" + name, required=True)
    preparation.add_argument("--source-directory", type=Path, required=True)
    preparation.add_argument("--published-binding", type=Path, required=True)
    preparation.add_argument("--release-run-id", type=int, required=True)
    preparation.add_argument("--platform", choices=("darwin-aarch64", "windows-x86_64"), required=True)
    commands.add_parser("serve").add_argument("--directory", type=Path, required=True)
    for name in ("seed-data", "verify-data"):
        data_command = commands.add_parser(name)
        data_command.add_argument("--database", type=Path, required=True)
        data_command.add_argument("--expected-schema", type=int, required=True)
    args = parser.parse_args()
    temporary = guard_runner()
    if args.command == "prepare":
        prepare(args, temporary)
    elif args.command == "serve":
        serve(args.directory, temporary)
    else:
        path = args.database.resolve(strict=True)
        if args.expected_schema < 1: parser.error("--expected-schema must be positive")
        print(json.dumps(seed_data(path, args.expected_schema) if args.command == "seed-data" else verify_data(path, args.expected_schema)))


if __name__ == "__main__":
    main()
