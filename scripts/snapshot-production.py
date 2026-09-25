#!/usr/bin/env python3
"""Quiesce the sole production Machine, snapshot its volume, and restore service."""
import copy
import json
import os
import sys
import time
import urllib.request


def request(method, path, body=None, *, expect_json=True):
    """Call the Fly Machines API without printing credentials or configuration."""
    req = urllib.request.Request(
        'https://api.machines.dev/v1/apps/pwmtf/' + path,
        data=None if body is None else json.dumps(body).encode(), method=method,
        headers={'Authorization': 'Bearer ' + os.environ['FLY_API_TOKEN'],
                 'Content-Type': 'application/json'})
    with urllib.request.urlopen(req, timeout=60) as response:
        if not expect_json:
            return None
        data = response.read()
        return json.loads(data) if data else None


def wait(machine, target):
    """Wait boundedly for a state, starting stopped Machines when recovering."""
    for _ in range(60):
        state = request('GET', f'machines/{machine}')['state']
        if state == target:
            return
        if target == 'started' and state == 'stopped':
            request('POST', f'machines/{machine}/start', {})
        time.sleep(2)
    raise RuntimeError(f'Machine did not reach {target} within the polling limit')


def snapshot():
    """Preserve original configuration on both successful and failed snapshots."""
    if os.environ.get('FLY_APP_NAME', 'pwmtf') != 'pwmtf':
        raise RuntimeError('Snapshot must target canonical pwmtf app')
    machines = [m for m in request('GET', 'machines') if m['state'] != 'destroyed']
    if not machines:
        print('No production Machine; initial-deployment snapshot skipped')
        return
    if len(machines) != 1:
        raise RuntimeError('Expected exactly one production Machine')
    machine = machines[0]['id']
    config = request('GET', f'machines/{machine}')['config']
    mounts = config.get('mounts', [])
    if len(mounts) != 1 or mounts[0].get('path') != '/data':
        raise RuntimeError('Expected one production /data volume mount')
    volume = mounts[0]['volume']
    volumes = request('GET', 'volumes')
    matches = [v for v in volumes if v['id'] == volume and v['name'] == 'pwmtf_data'
               and v['region'] == 'ord' and v['encrypted'] and v['state'] == 'created']
    if len(matches) != 1:
        raise RuntimeError('Attached volume is not the canonical encrypted production volume')
    quiesced = copy.deepcopy(config)
    for service in quiesced.get('services', []):
        service['autostart'] = False
        service['min_machines_running'] = 0
    # Same skip_launch/config restoration sequence as WWMTF. Install recovery
    # before the API mutation because a failed response can still have applied it.
    try:
        print('Quiescing production Machine for volume snapshot', flush=True)
        request('POST', f'machines/{machine}', {'config': quiesced, 'skip_launch': True})
        wait(machine, 'stopped')
        # Fly's snapshot-create endpoint confirms acceptance via HTTP status;
        # its client does not require a JSON body or snapshot ID.
        request('POST', f'volumes/{volume}/snapshots', expect_json=False)
        print('Fly volume snapshot request accepted', flush=True)
    finally:
        print('Restoring original Machine configuration and service', flush=True)
        request('POST', f'machines/{machine}', {'config': config, 'skip_launch': True})
        wait(machine, 'started')
    print('Snapshot request accepted; Machine restarted. Snapshot completion and restore are not verified.')


if __name__ == '__main__':
    try:
        if not os.environ.get('FLY_API_TOKEN'):
            raise RuntimeError('FLY_API_TOKEN is required')
        snapshot()
    except Exception as error:
        # API bodies/configuration may contain secrets; never dump exceptions.
        print(f'Production snapshot failed ({type(error).__name__}); inspect Machine state before retrying. Provider details suppressed.', file=sys.stderr)
        sys.exit(1)
