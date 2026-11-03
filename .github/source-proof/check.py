"""Read-only proof of an exact public source record and every reachable Git object."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

expected = json.loads(Path(__file__).with_name("expected.json").read_text())
phase = sys.argv[1]
path = Path(os.environ["RUNNER_TEMP"])
floor = 20 * 2**30
free = shutil.disk_usage(path).free
assert free >= floor + 2**30, f"Insufficient dedicated runner capacity: {free} bytes"
print(f"Runner available: {free} bytes; budget: 1 GiB; reserve: 20 GiB", flush=True)
helper = path / "htree-tools/git-remote-htree"
helper_sha = hashlib.sha256(helper.read_bytes()).hexdigest()
assert helper_sha == "ab3177515c89b59c82dd433b43361314a7b6e31c6f493469309257c8019ec21c"
with tempfile.TemporaryDirectory(prefix="source-proof-", dir=path) as directory:
    root = Path(directory)
    config = root / "config"
    config.mkdir()
    (config / "config.toml").write_text(
        '[nostr]\nrelays = ["wss://upload.iris.to/ws", "wss://cdn.iris.to/ws", "wss://relay.ditto.pub", "wss://temp.iris.to"]\n'
        '[blossom]\nservers = []\nread_servers = ["https://cdn.iris.to", "https://upload.iris.to"]\nwrite_servers = []\n'
    )
    # Deliberately do not inherit credentials, signing state, Git alternates or
    # helper caches from the workflow checkout or prior proof phases.
    env = dict(PATH=str(helper.parent) + os.pathsep + os.environ["PATH"], HOME=str(root),
               HTREE_CONFIG_DIR=str(config), HTREE_DATA_DIR=str(root / "empty-store"),
               HTREE_PREFER_LOCAL_DAEMON="0", HTREE_LOCAL_DAEMON_ONLY="0", NOSTR_PREFER_LOCAL="0",
               GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull, GIT_NO_REPLACE_OBJECTS="1")
    def git(*args, cwd=root, timeout=150):
        return subprocess.check_output(["git", *args], cwd=cwd, env=env, text=True, timeout=timeout).strip()
    def refs():
        lines = git("ls-remote", "--symref", expected["source"]).splitlines()
        assert "ref: refs/heads/master\tHEAD" in lines, "Unexpected or missing default branch"
        return {r:o for line in lines if not line.startswith("ref:") for o,r in [line.split()] if not r.endswith("^{}")}
    if phase == "qualify":
        assert refs() in [expected["before"], expected["expected"]], "Unexpected original references"
        print("Public baseline and pinned helper verified", flush=True)
    elif phase == "wait":
        deadline = time.monotonic() + 540
        while time.monotonic() < deadline:
            seen = refs()
            if seen == expected["expected"]:
                break
            assert seen == expected["before"], "Unexpected change in retained references"
            time.sleep(20)
        else:
            raise TimeoutError("No confirmed publication within the observation budget")
    elif phase == "verify":
        mirror = root / "public.git"
        git("clone", "--mirror", expected["source"], str(mirror), timeout=600)
        assert git("symbolic-ref", "HEAD", cwd=mirror) == "refs/heads/master"
        actual = {r:o for line in git("show-ref", "--head", cwd=mirror).splitlines() for o,r in [line.split()]}
        assert actual == expected["expected"], "Cold-cloned refs differ"
        git("fsck", "--strict", "--no-reflogs", "--no-dangling", cwd=mirror)
        oids = sorted({line.split(" ",1)[0] for line in git("rev-list", "--objects", "--missing=error", "--all", cwd=mirror).splitlines()})
        assert len(oids) == expected["objects"]
        proc = subprocess.Popen(["git", "cat-file", "--batch"], env=env, cwd=mirror,
                                stdin=subprocess.PIPE, stdout=subprocess.PIPE)
        manifest = hashlib.sha256()
        for oid in oids:
            proc.stdin.write(oid.encode() + b"\n"); proc.stdin.flush()
            obj, kind, size = proc.stdout.readline().rstrip().split()
            assert obj.decode() == oid
            digest = hashlib.sha256(kind + b" " + size + b"\0")
            left = int(size)
            while left:
                chunk = proc.stdout.read(min(left, 2**20))
                assert chunk
                digest.update(chunk); left -= len(chunk)
            assert proc.stdout.read(1) == b"\n"
            manifest.update(obj + b" " + digest.digest())
        proc.stdin.close(); assert proc.wait(timeout=30) == 0
        assert manifest.hexdigest() == expected["manifest_sha256"], "Cold Git object contents differ"
        assert shutil.disk_usage(path).free >= floor
        receipt = dict(commit=expected["commit"], refs=len(actual), objects=len(oids),
                       manifest_sha256=manifest.hexdigest(), helper_sha256=helper_sha,
                       strict_fsck="pass", cold_public_contents="identical",
                       runner_free_before_bytes=free, runner_free_after_bytes=shutil.disk_usage(path).free)
        (path / "source-receipt.json").write_text(json.dumps(receipt,indent=2)+"\n")
        print(json.dumps(receipt))
    else:
        raise ValueError("Unknown phase")
