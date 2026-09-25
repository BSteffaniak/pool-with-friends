#!/usr/bin/env python3
"""Hermetic snapshot ordering and recovery tests; no production API calls."""
import importlib.util
from pathlib import Path
import unittest
import io
import urllib.error
from unittest.mock import patch, MagicMock

spec = importlib.util.spec_from_file_location('snapshot', Path(__file__).with_name('snapshot-production.py'))
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)


class SnapshotTests(unittest.TestCase):
    def exercise(self, fail_snapshot=False, fail_quiesce=False):
        events = []
        original = {'mounts': [{'path': '/data', 'volume': 'vol-1'}],
                    'services': [{'autostart': True, 'min_machines_running': 1}]}
        def request(method, path, body=None, *, expect_json=True):
            if method == 'GET':
                return {'machines': [{'id': 'm1', 'state': 'started'}],
                        'machines/m1': {'config': original},
                        'volumes': [{'id': 'vol-1', 'name': 'pwmtf_data', 'region': 'ord',
                                     'encrypted': True, 'state': 'created'}]}[path]
            events.append((path, body))
            if path == 'machines/m1' and not body['config']['services'][0]['autostart'] and fail_quiesce:
                raise RuntimeError('ambiguous update failure')
            if path.endswith('/snapshots'):
                if fail_snapshot:
                    raise RuntimeError('snapshot failure')
                self.assertFalse(expect_json)
                self.assertIsNone(body)
                return None
        with patch.object(m, 'request', side_effect=request), patch.object(m, 'wait', side_effect=lambda machine, state: events.append(('wait', state))):
            if fail_snapshot or fail_quiesce:
                with self.assertRaises(RuntimeError):
                    m.snapshot()
            else:
                m.snapshot()
        self.assertEqual(events[-2], ('machines/m1', {'config': original, 'skip_launch': True}))
        self.assertEqual(events[-1], ('wait', 'started'))
        self.assertTrue(original['services'][0]['autostart'])
        return events

    def test_success_order(self):
        events = self.exercise()
        self.assertFalse(events[0][1]['config']['services'][0]['autostart'])
        self.assertEqual(events[0][1]['config']['services'][0]['min_machines_running'], 0)
        self.assertEqual(events[1], ('wait', 'stopped'))
        self.assertEqual(events[2][0], 'volumes/vol-1/snapshots')

    def test_snapshot_http_success_needs_no_body(self):
        for body in (b'', b'{}', b'accepted'):
            with self.subTest(body=body):
                response = MagicMock()
                response.__enter__.return_value = response
                response.read.return_value = body
                with patch.dict(m.os.environ, {'FLY_API_TOKEN': 'test-token'}), patch.object(m.urllib.request, 'urlopen', return_value=response):
                    self.assertIsNone(m.request('POST', 'volumes/vol-1/snapshots', expect_json=False))
                response.read.assert_not_called()
                response.__exit__.assert_called_once()

    def test_snapshot_http_failure_is_not_accepted(self):
        error = urllib.error.HTTPError('https://example.invalid', 503, 'Unavailable', {}, io.BytesIO())
        with patch.dict(m.os.environ, {'FLY_API_TOKEN': 'test-token'}), patch.object(m.urllib.request, 'urlopen', side_effect=error):
            with self.assertRaises(urllib.error.HTTPError):
                m.request('POST', 'volumes/vol-1/snapshots', expect_json=False)

    def test_snapshot_failure_restores(self):
        self.exercise(fail_snapshot=True)

    def test_ambiguous_quiesce_failure_restores(self):
        self.exercise(fail_quiesce=True)

    def test_first_deploy_skips(self):
        with patch.object(m, 'request', return_value=[]) as api:
            m.snapshot()
            api.assert_called_once_with('GET', 'machines')

    def test_multiple_machines_rejected(self):
        with patch.object(m, 'request', return_value=[{'state': 'started'}, {'state': 'stopped'}]):
            with self.assertRaises(RuntimeError):
                m.snapshot()

    def test_wait_timeout(self):
        with patch.object(m, 'request', return_value={'state': 'starting'}), patch.object(m.time, 'sleep'):
            with self.assertRaises(RuntimeError):
                m.wait('m1', 'stopped')


if __name__ == '__main__':
    unittest.main()
