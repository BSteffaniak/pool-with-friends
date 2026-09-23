#!/usr/bin/env python3
"""Provision per-project GitHub environment secrets without displaying values."""
import argparse
import json
import os
import re
import subprocess
import sys
import urllib.error
import urllib.parse
import urllib.request


class ProvisionError(Exception):
    """A sanitized provisioning failure safe to display."""


def command(args, body=None):
    """Capture all subprocess output; never surface provider credential diagnostics."""
    try:
        result = subprocess.run(args, input=body, text=True, capture_output=True,
                                timeout=120, check=False)
    except (OSError, subprocess.TimeoutExpired):
        raise ProvisionError(f'{args[0]} unavailable or timed out; inspect provider state before retrying') from None
    if result.returncode:
        raise ProvisionError(f'{args[0]} operation failed; provider output suppressed')
    return result.stdout


class Cloudflare:
    """Use a separate user API-token provisioning credential, never an app token."""
    def request(self, method, path, body=None):
        token = os.environ.get('CLOUDFLARE_PROVISION_TOKEN')
        if not token:
            raise ProvisionError('Set CLOUDFLARE_PROVISION_TOKEN with API Tokens Read/Edit and target-zone access')
        request = urllib.request.Request(
            'https://api.cloudflare.com/client/v4' + path,
            data=None if body is None else json.dumps(body).encode(), method=method,
            headers={'Authorization': f'Bearer {token}', 'Content-Type': 'application/json'})
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                payload = json.load(response)
            if not payload.get('success'):
                raise ProvisionError('Cloudflare operation failed; response suppressed')
            return payload
        except (urllib.error.URLError, ValueError):
            raise ProvisionError('Cloudflare request failed; inspect token state before retrying') from None

    def items(self, path):
        """Read all pages rather than silently missing older managed tokens."""
        page = 1
        while True:
            data = self.request('GET', path + ('&' if '?' in path else '?') + f'page={page}&per_page=50')
            yield from data['result']
            if page >= data.get('result_info', {}).get('total_pages', 1):
                return
            page += 1


def provision(args):
    """Skip by GitHub secret name; mutate providers only for selected missing secrets."""
    repo = args.repo
    environment = urllib.parse.quote(args.environment, safe='')
    endpoint = f'repos/{repo}/environments/{environment}'
    # Require an existing environment: do not accidentally weaken its protection rules.
    command(['gh', 'api', endpoint])
    existing = set(json.loads(command(['gh', 'secret', 'list', '--repo', repo,
                                      '--env', args.environment, '--json', 'name',
                                      '--jq', '[.[].name]'])))
    selected = args.only or ['CLOUDFLARE_ACCOUNT_ID', 'CLOUDFLARE_API_TOKEN',
                             'FLY_API_TOKEN', 'PWMTF_GOOGLE_CLIENT_ID', 'PWMTF_GOOGLE_CLIENT_SECRET']
    cf = Cloudflare()
    managed_name = f'github:{repo}:{args.environment}:cloudflare'
    for secret in selected:
        if secret in existing and not args.refresh:
            print(f'SKIP {secret}')
            continue
        created = None
        old = []
        if secret in ('CLOUDFLARE_ACCOUNT_ID', 'CLOUDFLARE_API_TOKEN'):
            zones = list(cf.items('/zones?name=' + urllib.parse.quote(args.zone)))
            if len(zones) != 1 or zones[0]['name'] != args.zone:
                raise ProvisionError('Expected exactly one accessible target zone')
            zone = zones[0]
            if secret == 'CLOUDFLARE_ACCOUNT_ID':
                value = zone['account']['id']
            else:
                old = [t['id'] for t in cf.items('/user/tokens') if t['name'] == managed_name]
                groups = list(cf.items('/user/tokens/permission_groups'))
                names = ['Zone Read', 'DNS Write', 'Zone Settings Write', 'Dynamic URL Redirects Write']
                permissions = []
                for name in names:
                    matches = [g for g in groups if g['name'] == name and 'com.cloudflare.api.account.zone' in g.get('scopes', [])]
                    if len(matches) != 1:
                        raise ProvisionError(f'Cannot uniquely resolve Cloudflare permission: {name}')
                    permissions.append({'id': matches[0]['id']})
                result = cf.request('POST', '/user/tokens', {
                    'name': managed_name, 'policies': [{'effect': 'allow',
                    'resources': {f'com.cloudflare.api.account.zone.{zone["id"]}': '*'},
                    'permission_groups': permissions}]})['result']
                created, value = result['id'], result['value']
        elif secret == 'FLY_API_TOKEN':
            # flyctl JSON only returns the token, not a revocation ID. Never put
            # a token in argv to revoke it or guess which older tokens are ours.
            value = json.loads(command(['flyctl', 'tokens', 'create', 'deploy', '--app', args.fly_app,
                '--name', f'github:{repo}:{args.environment}', '--expiry', args.fly_expiry, '--json']))['token']
        else:
            value = os.environ.get(secret)
            if not value:
                raise ProvisionError(f'Set {secret} in the local environment; Google client creation is manual')
        try:
            command(['gh', 'secret', 'set', secret, '--repo', repo, '--env', args.environment], value)
        except ProvisionError:
            # A timeout/failure may occur AFTER GitHub accepted the write. Deleting
            # the new credential would then break the installed secret. Fail safely.
            raise ProvisionError(f'{secret} upload not confirmed. New credential retained; inspect GitHub/provider state and refresh. Old credentials were not revoked.') from None
        print(f'SET {secret}')
        if created:
            for token_id in old:
                if token_id != created:
                    cf.request('DELETE', '/user/tokens/' + token_id)
            if old:
                print('Revoked previous Cloudflare tokens with this exact managed project name')
        if secret == 'FLY_API_TOKEN':
            print('Fly: review/revoke previous project tokens by ID with flyctl tokens list/revoke; automatic revocation is unavailable here')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo', required=True, help='GitHub OWNER/REPO')
    parser.add_argument('--zone', default='hyperchad.dev')
    parser.add_argument('--fly-app', default='pwmtf')
    parser.add_argument('--fly-expiry', default='8760h')
    parser.add_argument('--environment', default='Production')
    parser.add_argument('--refresh', action='store_true')
    parser.add_argument('--only', action='append', choices=['CLOUDFLARE_ACCOUNT_ID', 'CLOUDFLARE_API_TOKEN', 'FLY_API_TOKEN', 'PWMTF_GOOGLE_CLIENT_ID', 'PWMTF_GOOGLE_CLIENT_SECRET'])
    args = parser.parse_args()
    if not re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', args.repo):
        parser.error('--repo must be OWNER/REPO')
    try:
        provision(args)
    except ProvisionError as error:
        print(f'ERROR: {error}', file=sys.stderr)
        return 1
    except (KeyError, TypeError, ValueError):
        print('ERROR: Unexpected provider response; inspect provider state before retrying', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
