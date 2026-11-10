"""Fixed-data installer smoke checks; callers must first claim fresh CI app data.

Never discovers user profiles, reads the clipboard, or creates a missing DB.
"""
import json, pathlib, sqlite3, sys, time
path, mode, expected_schema = pathlib.Path(sys.argv[1]), sys.argv[2], int(sys.argv[3])
assert mode in ('fresh', 'seed', 'upgrade', 'upgrade-cleaned'), 'Unknown smoke database mode'
assert path.is_file(), 'Smoke database is missing'
uri = path.as_uri() + ('?mode=rw' if mode == 'seed' else '?mode=ro')
db = sqlite3.connect(uri, uri=True, timeout=5)
content, title, group = 'Starclip CI original note: a quiet orbit.', 'CI original note', 'CI original group'
settings = dict(paused=True, theme='forest', layout='list', maxHistory=0,
    shortcut='CommandOrControl+Shift+V', alwaysOnTop=False, retentionDays=0,
    showStartup=False, stickerPack='none')
assert db.execute('PRAGMA integrity_check').fetchone()[0] == 'ok', 'Database integrity check failed'
if mode == 'seed':
    assert db.execute('PRAGMA user_version').fetchone()[0] == 0, 'Baseline schema is not the verified 0.1.0 schema'
    assert db.execute('SELECT count(*) FROM clips').fetchone()[0] == 0, 'Baseline unexpectedly recorded content'
    assert db.execute('SELECT count(*) FROM groups').fetchone()[0] == 0, 'Baseline is not empty'
    now = int(time.time() * 1000)
    with db:
        db.execute('INSERT INTO groups(id,name,color) VALUES(1001,?,?)', (group, '#7BBEB0'))
        # The public Mac 0.1.0 predates keep_forever; its upgrade must add the
        # default column itself. Use only columns present in both baselines.
        db.execute('INSERT INTO clips(id,content,title,kind,pinned,group_id,created_at,updated_at,copy_count) VALUES(1001,?,?,?,1,1001,?,?,7)', (content, title, 'text', now, now))
        db.execute("INSERT INTO preferences(key,value) VALUES('settings',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", (json.dumps(settings),))
    db.execute('PRAGMA wal_checkpoint(TRUNCATE)')
else:
    assert db.execute('PRAGMA user_version').fetchone()[0] == expected_schema, 'Unexpected database schema'
    saved = db.execute("SELECT value FROM preferences WHERE key='settings'").fetchone()
    if mode == 'fresh':
        assert saved is None or json.loads(saved[0])['paused'] is True, 'Fresh settings are not paused'
        assert db.execute('SELECT count(*) FROM clips').fetchone()[0] == 0, 'Fresh application recorded the canary'
    else:
        assert saved is not None and json.loads(saved[0]) == settings, 'Upgrade did not preserve settings'
        assert db.execute('SELECT content,title,kind,pinned,keep_forever,group_id,copy_count FROM clips').fetchall() == [(content,title,'text',1,0,1001,7)], 'Upgrade changed the fixed record or recorded the canary'
        assert db.execute('SELECT name,color FROM groups WHERE id=1001').fetchone() == (group,'#7BBEB0'), 'Upgrade changed the fixed group'
        def migration_points():
            snapshots = list((path.parent / 'recovery').glob('*/snapshot.json'))
            points = []
            for snapshot in snapshots:
                try:
                    reason = json.loads(snapshot.read_text(encoding='utf-8'))['reason']
                except FileNotFoundError:
                    continue  # The app may finish cleanup while the harness reads its list.
                if reason == 'before-migration':
                    points.append(snapshot)
            return points
        valid = migration_points()
        if mode == 'upgrade-cleaned':
            deadline = time.monotonic() + 10
            while valid and time.monotonic() < deadline:
                time.sleep(0.2)
                valid = migration_points()
            assert not valid, 'Validated upgrade left obsolete pre-migration backups'
        else:
            assert valid, 'Pre-migration recovery point is missing'
            with sqlite3.connect((valid[0].parent / 'starclip.sqlite3').as_uri() + '?mode=ro', uri=True) as backup:
                assert backup.execute('SELECT content,title FROM clips').fetchall() == [(content,title)], 'Recovery point did not preserve the fixed record'
db.close()
print('Fixed database check passed: ' + mode)
