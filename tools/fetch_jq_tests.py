"""Fetch jq's pinned upstream test data (jq.test and tests/modules)."""
import hashlib
import io
import json
from pathlib import Path
import tarfile
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
ARCHIVE = ROOT / 'target/artifacts/jq-1.8.2-source.tar.gz'
DEST = ROOT / 'tests/fixtures/generated/reference/jq-1.8.2-tests'
URL = 'https://codeload.github.com/jqlang/jq/tar.gz/refs/tags/jq-1.8.2'
SHA256 = '188d03d029e8242a366d2286f07638924afe997120f6057e606f3c7a33984312'
SIZE = 1440150
PREFIX = 'jq-jq-1.8.2/tests/'


def fetch():
    if ARCHIVE.exists():
        data = ARCHIVE.read_bytes()
    else:
        request = urllib.request.Request(
            URL, headers={'User-Agent': 'Adapter-compatibility-research'})
        with urllib.request.urlopen(request, timeout=120) as response:
            data = response.read(SIZE + 1)
    if len(data) != SIZE or hashlib.sha256(data).hexdigest() != SHA256:
        raise RuntimeError('jq source size/hash mismatch; refusing to use it')
    ARCHIVE.parent.mkdir(parents=True, exist_ok=True)
    if not ARCHIVE.exists():
        ARCHIVE.write_bytes(data)
    DEST.mkdir(parents=True, exist_ok=True)
    extracted = []
    with tarfile.open(fileobj=io.BytesIO(data), mode='r:gz') as archive:
        for member in archive.getmembers():
            if not member.isfile() or not member.name.startswith(PREFIX):
                continue
            relative = member.name[len(PREFIX):]
            if relative != 'jq.test' and not relative.startswith('modules/'):
                continue
            target = DEST / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(archive.extractfile(member).read())
            extracted.append(relative)
    if 'jq.test' not in extracted or len(extracted) < 10:
        raise RuntimeError('jq test data extraction was incomplete')
    (DEST / 'provenance.json').write_text(json.dumps({
        'repository': 'https://github.com/jqlang/jq',
        'release': 'jq-1.8.2',
        'release_url': 'https://github.com/jqlang/jq/releases/tag/jq-1.8.2',
        'source_url': URL,
        'size': SIZE,
        'sha256': SHA256,
        'extracted_files': sorted(extracted),
        'reference_mac_run': False,
    }, indent=2) + '\n')
    print(f'Verified jq-1.8.2 test data: {len(extracted)} files, {SHA256}')


if __name__ == '__main__':
    fetch()
