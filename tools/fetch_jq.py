"""Fetch one fixed public macOS test artifact, never execute it here."""
import hashlib
import json
from pathlib import Path
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
DEST = ROOT/'tests/fixtures/generated/reference/jq-macos-amd64'
SHA256 = 'e94b266e3c26690550006abe63152b782280f4e14374accdf04cbde844f00bc0'
SIZE = 871768
URL = 'https://github.com/jqlang/jq/releases/download/jq-1.8.2/jq-macos-amd64'


def fetch():
    if DEST.exists():
        data = DEST.read_bytes()
    else:
        with urllib.request.urlopen(URL, timeout=60) as response:
            data = response.read(SIZE+1)
    if len(data) != SIZE or hashlib.sha256(data).hexdigest() != SHA256:
        raise RuntimeError('jq artifact size/hash mismatch; refusing to use it')
    DEST.parent.mkdir(parents=True, exist_ok=True)
    if not DEST.exists():
        DEST.write_bytes(data)
    # The artifact is executed by the CLI differential on Darwin; a plain write
    # leaves it non-executable under a fresh checkout.
    DEST.chmod(DEST.stat().st_mode | 0o111)
    (DEST.parent/'provenance.json').write_text(json.dumps({
        'repository':'https://github.com/jqlang/jq', 'release':'jq-1.8.2',
        'release_url':'https://github.com/jqlang/jq/releases/tag/jq-1.8.2',
        'asset_url':URL, 'size':SIZE, 'sha256':SHA256,
        'reference_mac_run':False,
    }, indent=2)+'\n')
    print('Verified jq-1.8.2 macOS amd64:', SHA256)


if __name__ == '__main__':
    fetch()
