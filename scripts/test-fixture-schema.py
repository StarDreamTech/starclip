#!/usr/bin/env python3
"""Original, disposable SQLite fixtures for installer/updater schema contracts.

These checks simulate harness phases only, not application migrations or GUI.
"""
from contextlib import closing
import ast
from types import SimpleNamespace
import importlib.util
import hashlib
import io
import json
from pathlib import Path
import shutil
import sqlite3
import plistlib
import subprocess
import sys
import tempfile
import tarfile
import unittest
from unittest.mock import patch

from fixture_schema import source_schema
from updater_baselines import download_package, public_baseline, verify_package

SCRIPTS = Path(__file__).resolve().parent


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'), SCRIPTS / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


RELEASE = load('release-updater-fixture')
MAC = load('macos-updater-acceptance')


def database(path, schema):
    with closing(sqlite3.connect(path)) as db, db:
        db.executescript('''
          CREATE TABLE groups(id INTEGER PRIMARY KEY, name TEXT, color TEXT);
          CREATE TABLE clips(id INTEGER PRIMARY KEY, content TEXT, title TEXT, kind TEXT,
            pinned INTEGER DEFAULT 0, keep_forever INTEGER DEFAULT 0, group_id INTEGER,
            created_at INTEGER, updated_at INTEGER, copy_count INTEGER DEFAULT 0);
          CREATE TABLE preferences(key TEXT PRIMARY KEY, value TEXT);
          CREATE TABLE clip_images(clip_id INTEGER, width INTEGER, height INTEGER,
            bytes INTEGER, png BLOB, thumbnail BLOB);
        ''')
        db.execute('PRAGMA user_version=' + str(schema))


def installer(path, mode, schema):
    return subprocess.run([sys.executable, str(SCRIPTS / 'installer-fixture.py'), str(path), mode, str(schema)],
                          capture_output=True, text=True, timeout=15)


class FixtureSchemaTests(unittest.TestCase):
    def test_compatibility_manifest_rewrites_download_urls_without_touching_signatures(self):
        signature = "same-signature"
        manifest = {
            "version": "0.2.6",
            "notes": "说明",
            "platforms": {
                "darwin-aarch64": {"signature": signature, "url": "https://github.com/StarDreamTech/starclip/releases/download/v0.2.6/Starclip_0.2.6_aarch64.app.tar.gz"},
                "windows-x86_64": {"signature": signature, "url": "https://github.com/StarDreamTech/starclip/releases/download/v0.2.6/Starclip_0.2.6_x64-setup.exe"},
            },
        }
        rewritten = RELEASE.compatibility_manifest(manifest)
        self.assertEqual(manifest["platforms"]["darwin-aarch64"]["url"], "https://github.com/StarDreamTech/starclip/releases/download/v0.2.6/Starclip_0.2.6_aarch64.app.tar.gz")
        self.assertEqual(rewritten["platforms"]["darwin-aarch64"]["signature"], signature)
        self.assertEqual(rewritten["platforms"]["windows-x86_64"]["url"], "https://github.com/cpython666/starclip/releases/download/v0.2.6/Starclip_0.2.6_x64-setup.exe")
        self.assertEqual(rewritten["notes"], "说明")
        with self.assertRaisesRegex(ValueError, "organization"):
            RELEASE.compatibility_manifest({"platforms": {"darwin-aarch64": {"url": "https://github.com/cpython666/starclip/releases/download/v0.2.6/pkg"}}})

    def test_public_baseline_selection_and_restart_requirements(self):
        for version, schema in (("0.2.2", 2), ("0.2.3", 2), ("0.2.4", 4), ("0.2.5", 5)):
            origin = "public-" + version
            for platform in ("darwin-aarch64", "windows-x86_64"):
                expected = public_baseline(origin, platform)
                self.assertEqual((expected["version"], expected["schema"]), (version, schema))
                self.assertTrue(expected["url"].endswith("/v" + version + "/" + expected["filename"]))
                self.assertRegex(expected["sha256"], r"^[a-f0-9]{64}$")
            self.assertEqual(MAC.baseline_comparison(origin, schema, 5, release_mode=True, allow_manual_recovery=False), version)
            with self.assertRaises(AssertionError):
                MAC.baseline_comparison(origin, schema + 1, 5, release_mode=True, allow_manual_recovery=False)
        self.assertEqual(MAC.baseline_comparison("public-0.2.2", 2, 4, release_mode=True, allow_manual_recovery=True), "0.2.2")
        for origin, schema in (("public-0.2.3", 2), ("public-0.2.4", 4), ("public-0.2.5", 5), ("source-comparison", 4)):
            with self.assertRaisesRegex(AssertionError, "Manual recovery"):
                MAC.baseline_comparison(origin, schema, 4, release_mode=True, allow_manual_recovery=True)
        self.assertEqual(MAC.baseline_comparison("source-comparison", 4, 4, release_mode=True, allow_manual_recovery=False), "0.2.2")
        self.assertEqual(MAC.baseline_comparison("source-comparison", 2, 2, release_mode=False, allow_manual_recovery=False), "0.2.1")
        metadata = subprocess.run([sys.executable, str(SCRIPTS / "updater_baselines.py"), "metadata", "--platform", "windows-x86_64"],
                                  check=True, capture_output=True, text=True, timeout=15)
        self.assertEqual(json.loads(metadata.stdout)["origin"], "public-0.2.5")

    def test_pinned_public_package_rejects_bytes_and_wrong_internal_version(self):
        binary = b"Starclip original synthetic executable; never run."
        with tempfile.TemporaryDirectory() as temp:
            package = Path(temp) / "original.tar.gz"
            expected = public_baseline("public-0.2.3", "darwin-aarch64")
            expected["executableSha256"] = hashlib.sha256(binary).hexdigest()

            def archive(version):
                with tarfile.open(package, "w:gz") as bundle:
                    for name, contents in (
                        ("Contents/Info.plist", plistlib.dumps(dict(CFBundleIdentifier="com.starclip.desktop", CFBundleShortVersionString=version,
                                                                 CFBundleVersion=version, CFBundleExecutable="starclip"))),
                        ("Contents/MacOS/starclip", binary),
                    ):
                        member = tarfile.TarInfo("Starclip.app/" + name)
                        member.size = len(contents)
                        bundle.addfile(member, io.BytesIO(contents))
                expected.update(size=package.stat().st_size, sha256=hashlib.sha256(package.read_bytes()).hexdigest())

            archive("0.2.3")
            verify_package(package, expected)
            for changed in (dict(size=expected["size"] + 1), dict(sha256="0" * 64)):
                with self.assertRaisesRegex(ValueError, "size/SHA-256"):
                    verify_package(package, dict(expected, **changed))
            with self.assertRaisesRegex(ValueError, "executable"):
                verify_package(package, dict(expected, executableSha256="0" * 64))
            archive("0.2.2")
            with self.assertRaisesRegex(ValueError, "internal version"):
                verify_package(package, expected)
            with patch("updater_baselines.urllib.request.urlopen", return_value=io.BytesIO(package.read_bytes())):
                output = Path(temp) / "verified.tar.gz"
                with self.assertRaises(ValueError):
                    download_package(output, expected)
                self.assertFalse(output.exists(), "Invalid downloads must never become baseline packages")
            archive("0.2.3")
            with patch("updater_baselines.urllib.request.urlopen", return_value=io.BytesIO(package.read_bytes())):
                download_package(output, expected)
            verify_package(output, expected)
            with self.assertRaises(FileExistsError):
                download_package(output, expected)

    def test_updater_workflows_keep_public_five_default_and_manual_two_explicit(self):
        workflows = SCRIPTS.parent / ".github/workflows"
        for name in ("macos-updater-acceptance.yml", "windows-updater-acceptance.yml"):
            text = (workflows / name).read_text(encoding="utf-8")
            self.assertIn("default: public-0.2.5", text)
            self.assertIn("public-0.2.5, public-0.2.4, public-0.2.3, public-0.2.2", text)
            self.assertIn("scripts/updater_baselines.py download", text)
        text = (workflows / "macos-updater-acceptance.yml").read_text(encoding="utf-8")
        self.assertIn('if [ "$BASELINE" = public-0.2.2 ]; then\n            flags=(--allow-manual-recovery)', text)
        self.assertIn("source-comparison", text)
        self.assertIn('scripts/updater_baselines.py metadata --origin "$BASELINE"', text)

    def test_updater_workflows_match_published_assets_without_requiring_the_same_commit(self):
        workflows = SCRIPTS.parent / ".github/workflows"
        script = (SCRIPTS / "release-run-binding.mjs").read_text(encoding="utf-8")
        fixture = (SCRIPTS / "release-updater-fixture.py").read_text(encoding="utf-8")
        self.assertIn("匹配方式：精确匹配", script)
        self.assertIn("匹配方式：仅 CI 文件差异 + 资产 sha256 一致", script)
        for prefix in (".github/", "docs/", ".agents/"):
            self.assertIn(f"'{prefix}'", script)
            self.assertIn(f'"{prefix}"', fixture)
        for name in ("macos-updater-acceptance.yml", "windows-updater-acceptance.yml"):
            text = (workflows / name).read_text(encoding="utf-8")
            self.assertIn("node scripts/release-run-binding.mjs bind", text)
            self.assertIn("node scripts/release-run-binding.mjs verify-assets", text)
            self.assertIn("--published-binding", text)
            self.assertNotIn("immutable tag build", text)
            self.assertNotIn("exact target tag commit", text)
            self.assertNotIn("head_sha !== commit", text)

    def test_published_binding_accepts_exact_tag_and_documentation_only_diff(self):
        tag, run = "a" * 40, "b" * 40
        files = [".github/workflows/release.yml", "docs/releases.md",
                 ".agents/skills/starclip/SKILL.md", ".agents/skills/starclip/references/development.md"]
        exact = dict(repository="StarDreamTech/starclip", tag="v0.2.6", version="0.2.6",
                     tagSha=tag, runHeadSha=tag, runId=43050585288, mode="exact", files=[])
        self.assertEqual(RELEASE.validate_published_binding(
            exact, version="0.2.6", source_ref="v0.2.6", source_head_sha=tag, release_run_id=43050585288)["mode"], "exact")
        ci = dict(exact, mode="ci-only", runHeadSha=run, files=files)
        self.assertEqual(RELEASE.validate_published_binding(
            ci, version="0.2.6", source_ref="v0.2.6", source_head_sha=tag, release_run_id=43050585288)["files"], files)
        for changed in (
            dict(ci, files=files + ["src/App.vue"]),
            dict(ci, files=["scripts/release-manifest.mjs"]),
            dict(ci, files=["CHANGELOG.md"]),
            dict(ci, files=["docs/../../src/main.rs"]),
            dict(exact, runHeadSha=run),
            dict(ci, runHeadSha=tag),
            dict(ci, repository="other/starclip"),
        ):
            with self.assertRaises(ValueError):
                RELEASE.validate_published_binding(changed, version="0.2.6", source_ref="v0.2.6",
                                                   source_head_sha=tag, release_run_id=43050585288)

    def test_target_schema_is_exact_source_not_installed_database(self):
        self.assertEqual(source_schema(SCRIPTS.parent), 5)
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            old = root / 'src-tauri/src/store.rs'
            old.parent.mkdir(parents=True)
            old.write_text('pub const SCHEMA_VERSION: u32 = 2;\n', encoding='utf-8')
            self.assertEqual(source_schema(root), 2)
            old.unlink()
            current = root / 'src-tauri/src/store/mod.rs'
            current.parent.mkdir()
            current.write_text('pub const SCHEMA_VERSION: u32 = 4;\n', encoding='utf-8')
            self.assertEqual(source_schema(root), 4)
            for invalid in ('', 'pub const SCHEMA_VERSION: u32 = 0;\n',
                            'pub const SCHEMA_VERSION: u32 = 2;\npub const SCHEMA_VERSION: u32 = 4;\n'):
                current.write_text(invalid, encoding='utf-8')
                with self.assertRaises(ValueError):
                    source_schema(root)

    def test_installer_preserves_historical_zero_and_requires_target_four(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            fresh = root / 'fresh.sqlite3'
            database(fresh, 4)
            self.assertEqual(installer(fresh, 'fresh', 4).returncode, 0)
            self.assertNotEqual(installer(fresh, 'fresh', 2).returncode, 0)
            path = root / 'upgrade.sqlite3'
            database(path, 0)
            self.assertEqual(installer(path, 'seed', 0).returncode, 0)
            saved = root / 'recovery/original-before-migration'
            saved.mkdir(parents=True)
            shutil.copyfile(path, saved / 'starclip.sqlite3')
            (saved / 'snapshot.json').write_text(json.dumps(dict(reason='before-migration')), encoding='utf-8')
            # Harness contract simulation only. Rust tests verify real migrations.
            with closing(sqlite3.connect(path)) as db, db:
                db.execute('PRAGMA user_version=4')
            self.assertEqual(installer(path, 'upgrade', 4).returncode, 0)
            self.assertNotEqual(installer(path, 'upgrade', 2).returncode, 0)

    def test_installer_cleanup_mode_requires_intact_current_data_and_no_old_migration_point(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            path = root / 'upgrade.sqlite3'
            database(path, 0)
            self.assertEqual(installer(path, 'seed', 0).returncode, 0)
            with closing(sqlite3.connect(path)) as db, db:
                db.execute('PRAGMA user_version=4')
            self.assertEqual(installer(path, 'upgrade-cleaned', 4).returncode, 0)
            self.assertNotEqual(installer(path, 'upgrade', 4).returncode, 0)
            with closing(sqlite3.connect(path)) as db, db:
                db.execute("UPDATE clips SET title='damaged original fixture'")
            self.assertNotEqual(installer(path, 'upgrade-cleaned', 4).returncode, 0)

    def test_release_baseline_two_and_target_four_are_strict_and_keep_opt_out(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            for schema in (2, 4):
                path = root / ('release-' + str(schema) + '.sqlite3')
                database(path, schema)
                self.assertEqual(RELEASE.seed_data(path, schema)['schema'], schema)
                with self.assertRaises(AssertionError):
                    RELEASE.verify_data(path, 4 if schema == 2 else 2)
            baseline = root / 'release-2.sqlite3'
            # Model the normalization required by the target harness. This does
            # not replace actual application migration/snapshot coverage.
            with closing(sqlite3.connect(baseline)) as db, db:
                db.execute('PRAGMA user_version=4')
            with self.assertRaisesRegex(AssertionError, 'telemetry_preferences_v1'):
                RELEASE.verify_data(baseline, 4)
            with closing(sqlite3.connect(baseline)) as db, db:
                db.execute("UPDATE preferences SET value=? WHERE key='telemetry_preferences_v1'",
                           (json.dumps(dict(revision=0, mode='off')),))
            self.assertEqual(RELEASE.verify_data(baseline, 4)['fixedImages'], 1)
            with self.assertRaises(AssertionError):
                RELEASE.verify_data(baseline, 2)

    def test_historical_macos_updater_fixture_retains_baseline_two(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / 'legacy.sqlite3'
            database(path, 2)
            MAC.seed_database(path, 2)
            MAC.verify_database(path, 2)
            with self.assertRaises(AssertionError):
                MAC.verify_database(path, 4)


    def test_macos_updater_checks_cleanup_for_development_targets_and_keeps_legacy_contract(self):
        # Exercise the actual nested harness check with disposable files, without running an app.
        tree = ast.parse((SCRIPTS / 'macos-updater-acceptance.py').read_text(encoding='utf-8'))
        check = next(item for item in ast.walk(tree) if isinstance(item, ast.FunctionDef) and item.name == 'fixture')
        with tempfile.TemporaryDirectory() as temp:
            data = Path(temp)
            current = data / 'starclip.sqlite3'
            current.write_bytes(b'original current database fixture')
            old_app = data / 'application-backups' / 'fixed-point' / 'Starclip.app'
            old_app.mkdir(parents=True)
            output = []
            context = dict(MAC.__dict__, data=data, database=current, release_fixture=False,
                           target_schema=5, baseline_schema=2, baseline_hash='original-app', COMPARISON='0.2.3',
                           verify_database=lambda path, schema: None, verify_app=lambda path, version: 'original-app',
                           write=lambda name, value: output.append(value))
            exec(compile(ast.Module(body=[check], type_ignores=[]), '<updater fixture>', 'exec'), context)
            with patch('sys.stdout', io.StringIO()):
                context['fixture']('after-install')
            self.assertEqual(output[-1]['recoveryPoints'], 0)
            point = data / 'recovery' / 'fixed-point'
            point.mkdir(parents=True)
            saved = point / 'starclip.sqlite3'
            saved.write_bytes(b'original baseline database fixture')
            (point / 'snapshot.json').write_text(json.dumps(dict(reason='before-update', schemaVersion=2,
                id='fixed-point', files={'starclip.sqlite3': {'sha256': MAC.digest(saved)}})))
            ticks = iter([0, 11])
            context['time'] = SimpleNamespace(monotonic=lambda: next(ticks), sleep=lambda seconds: None)
            with self.assertRaisesRegex(AssertionError, 'obsolete automatic data backups'):
                context['fixture']('after-install')
            context['target_schema'] = 4
            with patch('sys.stdout', io.StringIO()):
                context['fixture']('after-install')
            self.assertEqual(output[-1]['recoveryPoints'], 1)
            self.assertEqual(current.read_bytes(), b'original current database fixture')


if __name__ == '__main__':
    unittest.main()
