#!/usr/bin/env python3
"""Offline tests: no real provider credentials or API calls."""
import io
import urllib.error
import argparse
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('provision', Path(__file__).with_name('provision-production-secrets.py'))
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)


class ProvisionTests(unittest.TestCase):
    def args(self, **kwargs):
        return argparse.Namespace(**dict({'repo': 'owner/project', 'environment': 'Production',
            'only': ['CLOUDFLARE_API_TOKEN'], 'refresh': False, 'zone': 'example.com',
            'fly_app': 'example', 'fly_expiry': '8760h'}, **kwargs))

    def test_skip_never_calls_provider(self):
        with patch.object(m, 'command', side_effect=['{}', '["CLOUDFLARE_API_TOKEN"]']) as cmd, patch.object(m.Cloudflare, 'request') as api:
            m.provision(self.args())
            self.assertEqual(cmd.call_count, 2)
            api.assert_not_called()

    def rotate(self, upload_fails=False, refresh=True):
        events = []
        def cmd(args, body=None):
            if args[1:3] == ['secret', 'list']:
                return '["CLOUDFLARE_API_TOKEN"]' if refresh else '[]'
            if args[1:3] == ['secret', 'set']:
                events.append('upload')
                self.assertEqual(body, 'sensitive-value')
                self.assertNotIn(body, args)
                if upload_fails:
                    raise m.ProvisionError('failed')
            return '{}'
        def items(path):
            if path.startswith('/zones'):
                return [{'name': 'example.com', 'id': 'zone-id', 'account': {'id': 'account-id'}}]
            if path == '/user/tokens':
                return [{'name': 'github:owner/project:Production:cloudflare', 'id': 'old'}, {'name': 'unrelated', 'id': 'keep'}]
            return [{'name': name, 'id': name, 'scopes': ['com.cloudflare.api.account.zone']} for name in
                    ['Zone Read', 'DNS Write', 'Zone Settings Write', 'Dynamic URL Redirects Write']]
        def request(method, path, body=None):
            events.append((method, path))
            if method == 'POST':
                self.assertEqual(body['policies'][0]['resources'], {'com.cloudflare.api.account.zone.zone-id': '*'})
                return {'result': {'id': 'new', 'value': 'sensitive-value'}}
        with patch.object(m, 'command', side_effect=cmd), patch.object(m.Cloudflare, 'items', side_effect=items), patch.object(m.Cloudflare, 'request', side_effect=request):
            if upload_fails:
                with self.assertRaisesRegex(m.ProvisionError, 'not confirmed'):
                    m.provision(self.args(refresh=refresh))
            else:
                m.provision(self.args(refresh=refresh))
        return events

    def test_refresh_uploads_before_revoke_and_preserves_unrelated(self):
        self.assertEqual(self.rotate(), [('POST', '/user/tokens'), 'upload', ('DELETE', '/user/tokens/old')])

    def test_missing_creates(self):
        self.assertEqual(self.rotate(refresh=False)[0], ('POST', '/user/tokens'))

    def test_ambiguous_upload_never_revokes(self):
        self.assertEqual(self.rotate(upload_fails=True), [('POST', '/user/tokens'), 'upload'])

    def test_fly_scoped_creation_and_stdin_upload(self):
        with patch.object(m, 'command', side_effect=['{}', '[]', '{"token":"fly-secret"}', '']) as cmd:
            m.provision(self.args(only=['FLY_API_TOKEN']))
            self.assertIn('example', cmd.call_args_list[2].args[0])
            self.assertEqual(cmd.call_args_list[3].args[1], 'fly-secret')

    def test_google_missing_input_fails_without_writing(self):
        with patch.dict(m.os.environ, {}, clear=True), patch.object(m, 'command', side_effect=['{}', '[]']) as cmd:
            with self.assertRaisesRegex(m.ProvisionError, 'manual'):
                m.provision(self.args(only=['PWMTF_GOOGLE_CLIENT_SECRET']))
            self.assertEqual(cmd.call_count, 2)

    def test_http_diagnostics_are_actionable_and_redacted(self):
        for status in (400, 401, 403, 429, 500):
            error = urllib.error.HTTPError('https://secret-url', status, 'secret-reason', {},
                io.BytesIO(b'{"errors":[{"code":10000,"message":"secret-value"}]}'))
            with patch.dict(m.os.environ, {'CLOUDFLARE_PROVISION_TOKEN': 'secret-token'}), patch.object(m.urllib.request, 'urlopen', side_effect=error):
                with self.assertRaises(m.ProvisionError) as raised:
                    m.Cloudflare().request('POST', '/user/tokens', {'value': 'secret-body'})
            text = str(raised.exception)
            self.assertIn('create project token (POST)', text)
            self.assertIn(f'HTTP {status}', text)
            self.assertIn('10000', text)
            self.assertIn('not confirmed', text)
            self.assertNotIn('secret-', text)

    def test_network_diagnostics_redact_exception(self):
        with patch.dict(m.os.environ, {'CLOUDFLARE_PROVISION_TOKEN': 'secret-token'}), patch.object(m.urllib.request, 'urlopen', side_effect=urllib.error.URLError('secret-value')):
            with self.assertRaises(m.ProvisionError) as raised:
                m.Cloudflare().request('GET', '/user/tokens')
        self.assertIn('list managed tokens', str(raised.exception))
        self.assertIn('network/TLS/timeout', str(raised.exception))
        self.assertNotIn('secret-', str(raised.exception))

    def test_malformed_http_error_body_is_suppressed(self):
        error = urllib.error.HTTPError('secret-url', 403, 'secret', {}, io.BytesIO(b'secret html'))
        with patch.dict(m.os.environ, {'CLOUDFLARE_PROVISION_TOKEN': 'secret-token'}), patch.object(m.urllib.request, 'urlopen', side_effect=error):
            with self.assertRaises(m.ProvisionError) as raised:
                m.Cloudflare().request('GET', '/user/tokens/permission_groups')
        self.assertIn('list token permission groups', str(raised.exception))
        self.assertIn('HTTP 403', str(raised.exception))
        self.assertNotIn('secret', str(raised.exception))

    def test_subprocess_error_is_redacted(self):
        with patch.object(m.subprocess, 'run', return_value=argparse.Namespace(returncode=1, stdout='secret', stderr='secret')):
            with self.assertRaises(m.ProvisionError) as raised:
                m.command(['gh'])
            self.assertNotIn('secret', str(raised.exception))


if __name__ == '__main__':
    unittest.main()
