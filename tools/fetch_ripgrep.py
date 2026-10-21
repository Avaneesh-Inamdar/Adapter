"""Fetch one fixed public macOS test artifact, never execute it here."""
import hashlib
import io
import json
from pathlib import Path
import tarfile
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
DEST = ROOT/'tests/fixtures/generated/reference/ripgrep-macos-amd64'
TARBALL = 'ripgrep-15.2.0-x86_64-apple-darwin.tar.gz'
MEMBER = 'ripgrep-15.2.0-x86_64-apple-darwin/rg'
TARBALL_SIZE = 1878284
TARBALL_SHA256 = 'af7825fcc69a2afc7a7aea55fc9af90e26421d8f20fe59df32e233c0b8a231c1'
BINARY_SIZE = 4387000
BINARY_SHA256 = '0c9a0066db0d26b640777db88045b0ccdd58509a746700e43e1c4ff8707a5ed0'
URL = 'https://github.com/BurntSushi/ripgrep/releases/download/15.2.0/' + TARBALL


def extract_rg(archive):
    with tarfile.open(fileobj=io.BytesIO(archive), mode='r:gz') as bundle:
        member = bundle.getmember(MEMBER)
        if not member.isfile():
            raise RuntimeError('ripgrep archive member is not a regular file')
        source = bundle.extractfile(member)
        if source is None:
            raise RuntimeError('ripgrep archive member cannot be read')
        return source.read(BINARY_SIZE + 1)


def fetch():
    if DEST.exists():
        data = DEST.read_bytes()
    else:
        with urllib.request.urlopen(URL, timeout=120) as response:
            archive = response.read(TARBALL_SIZE + 1)
        if len(archive) != TARBALL_SIZE or hashlib.sha256(archive).hexdigest() != TARBALL_SHA256:
            raise RuntimeError('ripgrep archive size/hash mismatch; refusing to use it')
        data = extract_rg(archive)
    if len(data) != BINARY_SIZE or hashlib.sha256(data).hexdigest() != BINARY_SHA256:
        raise RuntimeError('ripgrep artifact size/hash mismatch; refusing to use it')
    DEST.parent.mkdir(parents=True, exist_ok=True)
    if not DEST.exists():
        DEST.write_bytes(data)
    (DEST.parent/'provenance-ripgrep.json').write_text(json.dumps({
        'repository':'https://github.com/BurntSushi/ripgrep', 'release':'15.2.0',
        'release_url':'https://github.com/BurntSushi/ripgrep/releases/tag/15.2.0',
        'asset_url':URL, 'archive_size':TARBALL_SIZE, 'archive_sha256':TARBALL_SHA256,
        'member':MEMBER, 'size':BINARY_SIZE, 'sha256':BINARY_SHA256,
        'reference_mac_run':False,
    }, indent=2)+'\n', encoding='utf-8', newline='\n')
    print('Verified ripgrep-15.2.0 macOS amd64:', BINARY_SHA256)


if __name__ == '__main__':
    fetch()
