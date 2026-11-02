#!/usr/bin/env python3
"""Original, disposable SQLite fixtures for installer/updater schema contracts.

These checks simulate harness phases only, not application migrations or GUI.
"""
import importlib.util
import json
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import unittest

from fixture_schema import source_schema

SCRIPTS = Path(__file__).resolve().parent


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'), SCRIPTS / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


RELEASE = load('release-updater-fixture')
MAC = load('macos-updater-acceptance')


def database(path, schema):
    with sqlite3.connect(path) as db:
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
    def test_target_schema_is_exact_source_not_installed_database(self):
        self.assertEqual(source_schema(SCRIPTS.parent), 4)
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
            with sqlite3.connect(path) as db:
                db.execute('PRAGMA user_version=4')
            self.assertEqual(installer(path, 'upgrade', 4).returncode, 0)
            self.assertNotEqual(installer(path, 'upgrade', 2).returncode, 0)

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
            with sqlite3.connect(baseline) as db:
                db.execute('PRAGMA user_version=4')
            with self.assertRaisesRegex(AssertionError, 'telemetry_preferences_v1'):
                RELEASE.verify_data(baseline, 4)
            with sqlite3.connect(baseline) as db:
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


if __name__ == '__main__':
    unittest.main()
