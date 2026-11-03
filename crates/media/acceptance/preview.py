#!/usr/bin/env python3
"""Exercise preview against disposable trees, never the operator's archive or store."""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def tree_snapshot(root):
    snapshot = {}
    for path in sorted(root.rglob('*')):
        relative = str(path.relative_to(root))
        if path.is_symlink():
            snapshot[relative] = ('symlink', os.readlink(path))
        elif path.is_file():
            snapshot[relative] = ('file', hashlib.sha256(path.read_bytes()).hexdigest())
        elif path.is_dir():
            snapshot[relative] = ('directory',)
    return snapshot


def main():
    if len(sys.argv) != 2:
        raise SystemExit('usage: preview.py <isolated-debug-media-binary>')
    binary = Path(sys.argv[1]).resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix='media-preview-acceptance-') as temporary:
        root = Path(temporary).resolve()
        source = root / 'source'
        archive = root / 'archive'
        source.mkdir()
        archive.mkdir()
        collection = source / '2016-01-Trip'
        collection.mkdir()
        (collection / 'clip.mp4').write_bytes(b'fixture bytes, not a real video')
        pending = source / 'Uncertain-event'
        pending.mkdir()
        (pending / 'photo.jpg').write_bytes(b'fixture bytes, not a real photo')
        unmapped = source / 'Unmapped'
        unmapped.mkdir()
        (unmapped / 'not-approved-for-metadata.jpg').write_bytes(b'unmapped fixture')
        (source / 'loose.txt').write_text('unassigned fixture')
        draft = root / 'organization.json'
        forbidden_db = root / 'must-not-open.sqlite'
        absent_overlay = root / 'must-not-create-overlay'
        env = dict(os.environ, SJEL_DB_PATH=str(forbidden_db),
                   SJEL_OVERLAY_ROOT=str(absent_overlay), SJEL_PERSONAL_ROOT=str(absent_overlay))

        def invoke(*args, extra_env=None):
            child_env = dict(env)
            child_env.update(extra_env or {})
            return subprocess.run([str(binary), *args], env=child_env,
                                  capture_output=True, text=True, timeout=30)

        identity = invoke('volume-id', '--root', str(source))
        if identity.returncode:
            raise SystemExit('scratch filesystem UUID unavailable; acceptance not run')
        uuid = json.loads(identity.stdout)['uuid']
        config = {
            'draft_version': 1,
            'source_root': str(source),
            'archive_root': str(archive),
            'expected_volume_uuid': uuid,
            'organization': {'categories': [{'name': 'Trips'}, {'name': 'Events'}]},
            'mappings': [
                {'source_collection': collection.name,
                 'proposed_destination_relative': 'Trips/2016/' + collection.name,
                 'status': 'destination_reviewed'},
                {'source_collection': pending.name,
                 'candidate_destination_relative': 'Events/' + pending.name,
                 'status': 'provisional_event_membership_needs_review'},
            ],
            'execution': {'file_moves_authorized': False},
        }
        draft.write_text(json.dumps(config))
        before_source, before_archive = tree_snapshot(source), tree_snapshot(archive)
        before_draft = draft.read_bytes()

        def unchanged():
            assert tree_snapshot(source) == before_source, 'source changed'
            assert tree_snapshot(archive) == before_archive, 'archive changed'
            assert draft.read_bytes() == before_draft, 'draft changed'
            assert not forbidden_db.exists(), 'preview opened the database'
            assert not absent_overlay.exists(), 'preview resolved a writable overlay'

        result = invoke('preview', '--structure', str(draft))
        assert result.returncode == 0, result.stderr
        report = json.loads(result.stdout)
        assert isinstance(report, dict)
        assert report['complete'] and not report['moves_authorized']
        assert 'Trips/2016/' + collection.name in result.stdout
        review = next(item for item in report['collections'] if item['name'] == pending.name)
        assert review['status'] == 'review' and review['proposal_destination_relative'] is None
        unchanged()

        wrong_volume = dict(config, expected_volume_uuid='00000000-0000-0000-0000-000000000000')
        draft.write_text(json.dumps(wrong_volume))
        refused = invoke('preview', '--structure', str(draft))
        assert refused.returncode != 0
        assert 'volume UUID mismatch' in json.loads(refused.stdout)['error']
        draft.write_bytes(before_draft)
        unchanged()

        for extra in [('--apply',), ('--db', str(forbidden_db)),
                      ('--metadata', '--metadata'), ('--structure', str(draft))]:
            refused = invoke('preview', '--structure', str(draft), *extra)
            assert refused.returncode != 0, 'unsafe or repeated option accepted'
            unchanged()

        fake_bin = root / 'bin'
        fake_bin.mkdir()
        extractor = fake_bin / 'exiftool'
        extractor.write_text('#!' + sys.executable + '\n' + '''
import json, os, sys
from pathlib import Path
assert sys.argv[1:3] == ['-config', ''], 'ExifTool user configuration was not disabled'
paths = [value for value in sys.argv[1:] if not value.startswith('-') and Path(value).is_file()]
assert all('/Unmapped/' not in path for path in paths), 'unmapped metadata was inspected'
rows = [{'SourceFile': path, 'Keys:CreationDate': '2016:01:05 12:00:00+01:00',
         'QuickTime:CreateDate': '2026:03:13 12:00:00',
         'GPS:GPSLatitude': 37.1234567, 'GPS:GPSLongitude': -122.7654321,
         'ExifTool:Warning': 'private diagnostic fixture'} for path in paths]
mode = os.environ.get('PREVIEW_FIXTURE_MODE', 'valid')
if mode == 'missing': rows = []
if mode == 'duplicate' and rows: rows.append(rows[0])
if mode == 'unexpected': rows.append({'SourceFile': '/outside/unknown.jpg'})
if mode == 'added' and paths: (Path(paths[0]).parent / 'added-during-extraction.jpg').write_bytes(b'new fixture')
if mode == 'removed' and paths: Path(paths[0]).unlink()
print(json.dumps(rows))
''')
        extractor.chmod(0o700)
        metadata_env = {'PATH': str(fake_bin) + os.pathsep + env.get('PATH', '')}
        result = invoke('preview', '--structure', str(draft), '--metadata', extra_env=metadata_env)
        assert result.returncode == 0, result.stderr
        metadata_report = json.loads(result.stdout)
        not_inspected = next(item for item in metadata_report['collections'] if item['name'] == unmapped.name)
        assert not_inspected['metadata'] is None
        assert '2016-01-05' in result.stdout
        for private_value in ('37.1234567', '-122.7654321', 'private diagnostic fixture'):
            assert private_value not in result.stdout + result.stderr, 'private metadata leaked'
        unchanged()

        for mode in ('missing', 'duplicate', 'unexpected'):
            failure = invoke('preview', '--structure', str(draft), '--metadata',
                             extra_env=dict(metadata_env, PREVIEW_FIXTURE_MODE=mode))
            assert failure.returncode != 0, 'incomplete metadata accepted: ' + mode
            unchanged()

        failure = invoke('preview', '--structure', str(draft), '--metadata',
                         extra_env=dict(metadata_env, PREVIEW_FIXTURE_MODE='added'))
        assert failure.returncode != 0, 'new file during extraction went unnoticed'
        for path in source.rglob('added-during-extraction.jpg'):
            path.unlink()
        unchanged()
        originals = {path: path.read_bytes() for path in source.rglob('*') if path.is_file()}
        failure = invoke('preview', '--structure', str(draft), '--metadata',
                         extra_env=dict(metadata_env, PREVIEW_FIXTURE_MODE='removed'))
        assert failure.returncode != 0, 'removed file during extraction went unnoticed'
        for path, content in originals.items():
            if not path.exists():
                path.write_bytes(content)
        unchanged()
        print('preview acceptance passed: read-only trees/draft/store, option refusal, wrong-volume refusal, metadata scope/privacy, and extraction completeness')


if __name__ == '__main__':
    main()
