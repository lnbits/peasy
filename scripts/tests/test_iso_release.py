import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import join_iso
import release_isos

TAG = "v1.2.3"
COMMIT = "a" * 40
REPO = "example/peasy"


class GitHub:
    def __init__(self):
        self.release = None
        self.assets = []
        self.calls = []
        self.fail_upload = False
        self.commit = COMMIT
        self.corrupt_upload = False
        self.annotated = False
        self.fail_after = None
        self.move_after_upload = False

    def __call__(self, *args):
        self.calls.append(args)
        if args[0] == "api":
            endpoint = args[1]
            if "/git/ref/tags/" in endpoint:
                return json.dumps({"object": {"type": "tag" if self.annotated else "commit", "sha": self.commit}})
            if "/git/tags/" in endpoint:
                return json.dumps({"object": {"type": "commit", "sha": self.commit}})
            if endpoint.endswith("/assets"):
                return json.dumps([self.assets])
            if endpoint.endswith("/releases"):
                return json.dumps([[self.release] if self.release else []])
            return json.dumps(self.release)
        if args[:2] == ("release", "create"):
            assert "--draft" in args and "--verify-tag" in args
            assert args[args.index("--target") + 1] == COMMIT
            self.release = {"id": 7, "tag_name": TAG, "draft": True,
                            "body": Path(args[args.index("--notes-file") + 1]).read_text()}
        elif args[:2] == ("release", "upload"):
            assert self.release["draft"] is True
            assert "--clobber" not in args
            if self.fail_upload or self.fail_after == len(self.assets):
                raise subprocess.CalledProcessError(1, "gh")
            path = Path(args[3])
            self.assets.append({"name": path.name, "size": path.stat().st_size, "state": "uploaded",
                                "digest": "sha256:" + ("0" * 64 if self.corrupt_upload else join_iso.digest(path))})
            if self.move_after_upload:
                self.commit = "b" * 40
        elif args[:2] == ("release", "edit"):
            assert "--draft=false" in args
            self.release["draft"] = False
        else:
            raise AssertionError(args)
        return ""


class Releases(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / "source"
        self.output = self.root / "output"
        self.source.mkdir()
        self.output.mkdir()
        for desktop, size in (("gnome", 100), ("plasma", 9000)):
            name = f"peasy-nixos-{TAG}-{desktop}-x86_64.iso"
            path = self.source / name
            path.write_bytes(bytes(range(250)) * (size // 250) + b"x" * (size % 250))
            (self.source / f"{name}.sha256").write_text(f"{join_iso.digest(path)}  {name}\n")
            (self.source / f"{desktop}-iso-status.json").write_text(json.dumps({
                "releaseReady": True, "installedTargetHasPeasy": True, "installedBootVerified": True}))

    def prepare(self):
        return release_isos.prepare(self.source, self.output, TAG, COMMIT, 4096, 2048)

    def publish(self, assets, github):
        release_isos.publish(assets, TAG, COMMIT, REPO, self.output, run=github)

    def test_direct_and_split_round_trip(self):
        assets = self.prepare()
        self.assertTrue(any(asset.name.endswith("gnome-x86_64.iso") for asset in assets))
        manifest = next(self.output.glob("*.parts.json"))
        reconstructed = join_iso.join(manifest)
        self.assertEqual(reconstructed.read_bytes(), (self.source / reconstructed.name).read_bytes())
        with self.assertRaises(FileExistsError):
            join_iso.join(manifest)

    def test_missing_part_and_corruption_never_leave_iso(self):
        self.prepare()
        manifest = next(self.output.glob("*.parts.json"))
        data = json.loads(manifest.read_text())
        part = self.output / data["parts"][0]["name"]
        original = part.read_bytes()
        part.unlink()
        with self.assertRaises(ValueError):
            join_iso.join(manifest)
        part.write_bytes(b"z" * len(original))
        with self.assertRaises(ValueError):
            join_iso.join(manifest)
        self.assertFalse((self.output / data["name"]).exists())

    def test_traversal_and_symlink_rejected(self):
        self.prepare()
        manifest = next(self.output.glob("*.parts.json"))
        data = json.loads(manifest.read_text())
        part = self.output / data["parts"][0]["name"]
        part.unlink()
        part.symlink_to(self.source / data["name"])
        with self.assertRaises(ValueError):
            join_iso.join(manifest)
        data["name"] = "../outside.iso"
        manifest.write_text(json.dumps(data))
        with self.assertRaises(ValueError):
            join_iso.join(manifest)

    def test_bad_checksum_or_missing_image_blocks_packaging(self):
        next(self.source.glob("*plasma*.iso")).write_bytes(b"corrupt")
        with self.assertRaises(ValueError):
            self.prepare()
        self.assertEqual(list(self.output.iterdir()), [])
        next(self.source.glob("*plasma*.iso")).unlink()
        with self.assertRaises(ValueError):
            self.prepare()

    def test_unverified_status_blocks_packaging(self):
        (self.source / "plasma-iso-status.json").write_text('{"releaseReady": true}')
        with self.assertRaises(ValueError):
            self.prepare()

    def test_publish_only_after_verification_and_idempotent_retry(self):
        assets = self.prepare()
        github = GitHub()
        github.annotated = True
        self.publish(assets, github)
        self.assertIs(github.release["draft"], False)
        self.assertEqual(len(github.assets), len(assets))
        edits = [call for call in github.calls if call[:2] == ("release", "edit")]
        self.assertEqual(len(edits), 1)
        github.calls.clear()
        self.publish(assets, github)
        self.assertFalse(any(call[0] == "release" for call in github.calls))

    def test_failed_upload_stays_private_and_resumes(self):
        assets = self.prepare()
        github = GitHub()
        github.fail_after = 2
        with self.assertRaises(subprocess.CalledProcessError):
            self.publish(assets, github)
        self.assertIs(github.release["draft"], True)
        self.assertEqual(len(github.assets), 2)
        github.fail_after = None
        self.publish(assets, github)
        self.assertIs(github.release["draft"], False)
        self.assertEqual(len(github.assets), len(assets))

    def test_tag_moved_during_upload_never_publishes(self):
        assets = self.prepare()
        github = GitHub()
        github.move_after_upload = True
        with self.assertRaises(ValueError):
            self.publish(assets, github)
        self.assertIs(github.release["draft"], True)
        self.assertFalse(any(call[:2] == ("release", "edit") for call in github.calls))

    def test_incomplete_published_release_is_not_modified(self):
        assets = self.prepare()
        github = GitHub()
        self.publish(assets, github)
        github.assets.pop()
        github.calls.clear()
        with self.assertRaises(ValueError):
            self.publish(assets, github)
        self.assertFalse(any(call[0] == "release" for call in github.calls))

    def test_api_failure_is_not_treated_as_absent_release(self):
        assets = self.prepare()
        github = GitHub()

        def failing(*args):
            if args[:2] == ("api", f"repos/{REPO}/releases"):
                raise subprocess.CalledProcessError(1, "gh")
            return github(*args)

        with self.assertRaises(subprocess.CalledProcessError):
            release_isos.publish(assets, TAG, COMMIT, REPO, self.output, run=failing)
        self.assertIsNone(github.release)

    def test_exact_size_limit_splits_and_below_limit_is_direct(self):
        for desktop, size in (("gnome", 4095), ("plasma", 4096)):
            path = next(self.source.glob(f"*{desktop}*.iso"))
            path.write_bytes(b"x" * size)
            path.with_name(path.name + ".sha256").write_text(f"{join_iso.digest(path)}  {path.name}\n")
        assets = self.prepare()
        self.assertTrue(any(asset.name.endswith("gnome-x86_64.iso") for asset in assets))
        self.assertEqual(len(list(self.output.glob("*.part*"))), 3)  # Two parts and manifest.

    def test_whole_checksum_and_part_order_validated(self):
        self.prepare()
        manifest = next(self.output.glob("*.parts.json"))
        data = json.loads(manifest.read_text())
        data["sha256"] = "0" * 64
        manifest.write_text(json.dumps(data))
        with self.assertRaises(ValueError):
            join_iso.join(manifest)
        self.assertFalse((self.output / data["name"]).exists())
        data["parts"].reverse()
        manifest.write_text(json.dumps(data))
        with self.assertRaises(ValueError):
            join_iso.join(manifest)

    def test_remote_corruption_never_publishes_or_overwrites(self):
        assets = self.prepare()
        github = GitHub()
        github.corrupt_upload = True
        with self.assertRaises(ValueError):
            self.publish(assets, github)
        self.assertIs(github.release["draft"], True)
        with self.assertRaises(ValueError):
            self.publish(assets, github)

    def test_unrelated_release_and_moved_tag_untouched(self):
        assets = self.prepare()
        github = GitHub()
        github.release = {"id": 7, "tag_name": TAG, "draft": False, "body": "user release"}
        with self.assertRaises(ValueError):
            self.publish(assets, github)
        self.assertFalse(any(call[0] == "release" for call in github.calls))
        github.commit = "b" * 40
        with self.assertRaises(ValueError):
            self.publish(assets, github)

    def test_bad_identifiers_rejected(self):
        for tag in ("v1/other", "v1;echo", "--draft", "1.0"):
            with self.assertRaises(ValueError):
                release_isos.prepare(self.source, self.output, tag, COMMIT)


if __name__ == "__main__":
    unittest.main()
