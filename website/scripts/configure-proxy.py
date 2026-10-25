#!/usr/bin/env python3
"""Run only on 161.97.140.89. NPM credentials never leave the server."""
import json
import socket
import sys
import urllib.error
import urllib.request
from pathlib import Path

DOMAIN = 'neutrondrive.sigmacodez.dev'
if '161.97.140.89' not in {result[4][0] for result in socket.getaddrinfo(DOMAIN, 443, socket.AF_INET)}:
    raise SystemExit('Domain does not resolve to the deployment server.')

def api(path, data=None, token=None, method=None):
    headers = {'Content-Type': 'application/json'}
    if token:
        headers['Authorization'] = 'Bearer ' + token
    request = urllib.request.Request('http://127.0.0.1:81/api' + path,
        data=json.dumps(data).encode() if data is not None else None,
        headers=headers, method=method)
    with urllib.request.urlopen(request, timeout=300) as response:
        return json.load(response)

try:
    token = api('/tokens', json.loads(Path('/opt/codezwerk-proxy/admin-credentials.json').read_text()))['token']
except urllib.error.HTTPError as error:
    raise SystemExit(f'NPM authentication failed (HTTP {error.code}). Update the existing server credentials file; no proxy changes were made.') from None
if '--check-auth' in sys.argv:
    print('Proxy authentication verified.')
    raise SystemExit(0)
hosts = api('/nginx/proxy-hosts', token=token)
existing = next((h for h in hosts if DOMAIN in h['domain_names']), None)
if existing and existing['domain_names'] != [DOMAIN]:
    raise SystemExit('Refusing to modify a proxy host shared with other domains.')
config = {
    'domain_names': [DOMAIN], 'forward_scheme': 'http',
    'forward_host': 'neutrondrive-web', 'forward_port': 31219,
    'certificate_id': existing['certificate_id'] if existing else 0,
    'ssl_forced': bool(existing and existing['certificate_id']),
    'hsts_enabled': bool(existing and existing['certificate_id']), 'hsts_subdomains': False,
    'http2_support': True, 'trust_forwarded_proto': False,
    'block_exploits': True, 'caching_enabled': False,
    'allow_websocket_upgrade': False, 'access_list_id': 0,
    'advanced_config': 'client_max_body_size 1k;\nclient_body_timeout 10s;\nclient_header_timeout 10s;\nsend_timeout 20s;\naccess_log off;',
    'locations': []
}
if existing:
    host = api('/nginx/proxy-hosts/' + str(existing['id']), config, token, 'PUT')
else:
    host = api('/nginx/proxy-hosts', config, token)
certs = api('/nginx/certificates', token=token)
cert = next((c for c in certs if c['provider'] == 'letsencrypt' and c['domain_names'] == [DOMAIN]), None)
if not cert:
    cert = api('/nginx/certificates', {'provider':'letsencrypt', 'domain_names':[DOMAIN], 'meta':{'dns_challenge':False}}, token)
api('/nginx/proxy-hosts/' + str(host['id']), {'certificate_id':cert['id'], 'ssl_forced':True,
    'http2_support':True, 'hsts_enabled':True, 'hsts_subdomains':False}, token, 'PUT')
print('Proxy configured; HTTPS enforced; certificate renewal managed by Nginx Proxy Manager.')
