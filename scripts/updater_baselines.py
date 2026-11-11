#!/usr/bin/env python3
"""Pinned genuine public updater baselines; download/inspect, never install."""
import argparse
import hashlib
import json
from pathlib import Path
import plistlib
import shutil
import tarfile
import tempfile
import time
import urllib.error
import urllib.request


PUBLIC_BASELINES = {
    "public-0.2.5": {
        "version": "0.2.5", "schema": 5,
        "darwin-aarch64": {
            "filename": "Starclip_0.2.5_aarch64.app.tar.gz", "size": 8622787,
            "sha256": "d84b91c021e6805374b7034e36c62a0a2054e6a889a793206d7c66dc36fec6d8",
            "executableSha256": "c21b6d351b7b1dae934262fac48e3aa0d602bdfb2033d38e5740c543430c185e",
        },
        "windows-x86_64": {
            "filename": "Starclip_0.2.5_x64-setup.exe", "size": 7128086,
            "sha256": "c80060f343ea57715fd923edfe850162bc5374f9ac940c4843df9e3fa793eeff",
        },
    },
    "public-0.2.4": {
        "version": "0.2.4", "schema": 4,
        "darwin-aarch64": {
            "filename": "Starclip_0.2.4_aarch64.app.tar.gz", "size": 8391207,
            "sha256": "f44ebe5de73bcc479b16e9dfaa43ea26a8a6e841a055621c767fb17d26234273",
            "executableSha256": "c7b0da1a1b4a55a6cf61e57aa005afcaaa269fa11106b28bf81489f42e0a731a",
        },
        "windows-x86_64": {
            "filename": "Starclip_0.2.4_x64-setup.exe", "size": 6917240,
            "sha256": "a0f712d2d8d6f9bcb18656f8287e540475e1f31c4b3dca217fbcd7601ab541f1",
        },
    },
    "public-0.2.2": {
        "version": "0.2.2", "schema": 2,
        "darwin-aarch64": {
            "filename": "Starclip_0.2.2_aarch64.app.tar.gz", "size": 6402985,
            "sha256": "83113a8c37739335cbc11d905c5b3bc089bba4aa0ad9b380a5d39199b63d224e",
            "executableSha256": "ec6d08edf90f2d51b1cb75cf9271103b82f3e68763f1d94931a8e66ae94fe40f",
        },
        "windows-x86_64": {
            "filename": "Starclip_0.2.2_x64-setup.exe", "size": 5139609,
            "sha256": "5974a9d4584045c571f51e08657cfd228fd38bee87a9a7169392d924f1309bdd",
        },
    },
    "public-0.2.3": {
        "version": "0.2.3", "schema": 2,
        "darwin-aarch64": {
            "filename": "Starclip_0.2.3_aarch64.app.tar.gz", "size": 7796156,
            "sha256": "23f2458d5b4d55dd7a81022d28c524593b715946029c7e309a11b763233bb087",
            "executableSha256": "72c26106f7210e2e4570ec8bde928c9474be3b7db56084c1dbb487e0f5afe38f",
        },
        "windows-x86_64": {
            "filename": "Starclip_0.2.3_x64-setup.exe", "size": 6354060,
            "sha256": "9e1339852e0eebfb90d14f383aee95d8795deac08147e426f4a74268f51f0055",
        },
    },
}


def public_baseline(origin, platform):
    release = PUBLIC_BASELINES[origin]
    return dict(origin=origin, version=release["version"], schema=release["schema"],
                platform=platform, **release[platform],
                url=f"https://github.com/StarDreamTech/starclip/releases/download/v{release['version']}/{release[platform]['filename']}")


def verify_package(path, expected):
    if path.stat().st_size != expected["size"] or hashlib.sha256(path.read_bytes()).hexdigest() != expected["sha256"]:
        raise ValueError("The public baseline size/SHA-256 differs from the reviewed release asset")
    if expected["platform"] == "darwin-aarch64":
        with tarfile.open(path, "r:gz") as bundle:
            def member(suffix):
                matches = [m for m in bundle.getmembers() if m.isfile() and m.name.endswith(suffix)]
                if len(matches) != 1:
                    raise ValueError("The public baseline must contain one app plist and executable")
                return bundle.extractfile(matches[0]).read()
            info = plistlib.loads(member("/Contents/Info.plist"))
            if (info.get("CFBundleIdentifier"), info.get("CFBundleShortVersionString"), info.get("CFBundleVersion"), info.get("CFBundleExecutable")) != ("com.starclip.desktop", expected["version"], expected["version"], "starclip"):
                raise ValueError("The public baseline bundle has an unexpected internal version or identity")
            if hashlib.sha256(member("/Contents/MacOS/starclip")).hexdigest() != expected["executableSha256"]:
                raise ValueError("The public baseline executable differs from the reviewed release asset")


def download_package(output, expected):
    if output.exists() or output.is_symlink():
        raise FileExistsError("Refusing to overwrite an existing baseline download")
    # Only the complete, verified public package is moved to the requested path.
    with tempfile.TemporaryDirectory(prefix="starclip-baseline-", dir=output.parent) as temporary:
        path = Path(temporary) / "download"
        for attempt in range(3):
            try:
                with urllib.request.urlopen(expected["url"], timeout=180) as response, path.open("wb") as file:
                    shutil.copyfileobj(response, file)
                break
            except (urllib.error.URLError, TimeoutError):
                if attempt == 2:
                    raise
                time.sleep(1)
        verify_package(path, expected)
        path.replace(output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("metadata", "download", "verify"))
    parser.add_argument("--origin", choices=tuple(PUBLIC_BASELINES), default="public-0.2.5")
    parser.add_argument("--platform", choices=("darwin-aarch64", "windows-x86_64"), required=True)
    parser.add_argument("--package", type=Path)
    args = parser.parse_args()
    expected = public_baseline(args.origin, args.platform)
    if args.operation != "metadata":
        if args.package is None:
            parser.error("--package is required for download/verify")
        (download_package if args.operation == "download" else verify_package)(args.package, expected)
    print(json.dumps(expected))


if __name__ == "__main__":
    main()
