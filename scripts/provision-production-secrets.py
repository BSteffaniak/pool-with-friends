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
        # Labels are fixed: never echo request URLs, token IDs, or query values.
        endpoint = path.split('?', 1)[0]
        label = {
            '/zones': 'discover zone',
            '/user/tokens/permission_groups': 'list token permission groups',
            '/user/tokens': 'create project token' if method == 'POST' else 'list managed tokens',
        }.get(endpoint, 'revoke previous managed token' if method == 'DELETE' else 'API request')
        operation = f'Cloudflare {label} ({method})'
        request = urllib.request.Request(
            'https://api.cloudflare.com/client/v4' + path,
            data=None if body is None else json.dumps(body).encode(), method=method,
            headers={'Authorization': f'Bearer {token}', 'Content-Type': 'application/json'})

        def failure(status, payload):
            # Provider messages may echo inputs. Only numeric codes are safe to
            # expose; deliberately omit message text, headers, and response bodies.
            errors = payload.get('errors', []) if isinstance(payload, dict) else []
            codes = sorted({str(error['code']) for error in errors
                            if isinstance(error, dict) and type(error.get('code')) is int}) if isinstance(errors, list) else []
            detail = f'; error codes: {", ".join(codes)}' if codes else ''
            if status in (401, 403):
                hint = 'Check provisioning-token validity, User API Tokens Read/Edit, and target-zone access.'
            elif status == 429:
                hint = 'Rate limited; wait before retrying.'
            elif status == 400:
                hint = 'Check token policy permissions and whether the provisioning credential may grant them.'
            else:
                hint = 'Check Cloudflare availability and provisioning-token permissions.'
            if method in ('POST', 'DELETE'):
                hint += ' Token mutation was not confirmed; inspect provider state before retrying.'
            return ProvisionError(f'{operation}: HTTP {status}{detail}. {hint}')

        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                status = response.status
                payload = json.load(response)
            if not isinstance(payload, dict):
                raise ValueError
            if not payload.get('success'):
                raise failure(status, payload)
            return payload
        except urllib.error.HTTPError as error:
            try:
                payload = json.loads(error.read(65536))
            except (ValueError, OSError):
                payload = {}
            finally:
                error.close()
            raise failure(error.code, payload) from None
        except (urllib.error.URLError, TimeoutError, OSError):
            raise ProvisionError(f'{operation}: network/TLS/timeout failure. Check connectivity and certificate trust; inspect token state before retrying mutations.') from None
        except ValueError:
            raise ProvisionError(f'{operation}: invalid JSON or unexpected response shape. Response suppressed; inspect token state before retrying mutations.') from None

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
                raise ProvisionError(f'Cloudflare zone discovery returned {len(zones)} results without one exact match. Use the root zone (for example hyperchad.dev), not the app subdomain; grant Zone Read for that zone to CLOUDFLARE_PROVISION_TOKEN.')
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
