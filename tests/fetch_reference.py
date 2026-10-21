"""Fetch one public macOS release for inspection only; preserve provenance."""
import hashlib
import json
from pathlib import Path
import urllib.request
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/"tests/fixtures/generated/reference"
def get(url):
    with urllib.request.urlopen(urllib.request.Request(url,headers={"User-Agent":"Adapter-compatibility-research"}),timeout=60) as r:
        return r.read(32*1024*1024)
def fetch():
    OUT.mkdir(parents=True,exist_ok=True)
    release=json.loads(get("https://api.github.com/repos/jqlang/jq/releases/latest"))
    asset=next(a for a in release["assets"] if a["name"]=="jq-macos-amd64")
    data=get(asset["browser_download_url"])
    sha=hashlib.sha256(data).hexdigest()
    digest=asset.get("digest")
    if digest and digest != "sha256:"+sha: raise RuntimeError("release asset digest mismatch")
    (OUT/"jq-macos-amd64").write_bytes(data)
    evidence={"repository":"https://github.com/jqlang/jq","release":release["tag_name"],"release_url":release["html_url"],"asset_url":asset["browser_download_url"],"size":len(data),"sha256":sha,"github_digest":digest,"reference_mac_run":False}
    (OUT/"provenance.json").write_text(json.dumps(evidence,indent=2)+"\n")
    print(json.dumps(evidence,indent=2))
if __name__=="__main__":fetch()
