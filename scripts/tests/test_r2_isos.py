import copy
import io
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch
from urllib.error import HTTPError

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import r2_isos
import release_isos
import test_iso_release
from test_iso_release import GitHub, TAG, COMMIT, REPO


class Missing(Exception):
    response = {"Error": {"Code": "404"}}


class Storage:
    def __init__(self):
        self.objects = {}
        self.uploads = []
        self.deleted = []
        self.corrupt = False
        self.fail_desktop = None

    def head_object(self, Bucket, Key):
        if Key not in self.objects:
            raise Missing()
        data, metadata = self.objects[Key]
        return {"ContentLength": len(data), "Metadata": metadata, "ETag": '"etag"'}

    def get_object(self, Bucket, Key, IfMatch):
        assert IfMatch == '"etag"'
        data = self.objects[Key][0]
        return {"Body": io.BytesIO(b"!" + data[1:] if self.corrupt else data)}

    def upload(self, s3, bucket, key, path, metadata):
        assert s3 is self
        if self.fail_desktop and self.fail_desktop in path.name:
            raise RuntimeError("Upload interrupted")
        self.uploads.append(key)
        self.objects[key] = (path.read_bytes(), metadata)

    def get_paginator(self, name):
        assert name == "list_objects_v2"
        return self

    def paginate(self, Bucket, Prefix):
        for key in list(self.objects):
            if key.startswith(Prefix):
                yield {"Contents": [{"Key": key}]}

    def delete_object(self, Bucket, Key):
        self.deleted.append(Key)
        del self.objects[Key]


class R2Releases(unittest.TestCase):
    setUp = test_iso_release.Releases.setUp

    def prepare(self):
        return r2_isos.prepare(self.source, self.output, TAG, COMMIT, "https://downloads.askpeasy.com")

    def deliver(self, storage, manifest, published=False):
        r2_isos.deliver(storage, "peasy-releases", self.source, manifest, published,
                        upload=storage.upload, public_check=lambda item: None)

    def publish(self, manifest, assets, storage, github):
        release_isos.publish(assets, TAG, COMMIT, REPO, self.output, run=github, manifest=manifest,
                             deliver=lambda published: self.deliver(storage, manifest, published))

    def latest(self, manifest):
        return {"tag_name": TAG, "draft": False, "prerelease": False, "body": r2_isos.notes(manifest)}

    def old_key(self, storage, tag="v0.1.0", desktop="gnome"):
        key = f"{r2_isos.PREFIX}{tag}/{COMMIT}/peasy-nixos-{tag}-{desktop}-x86_64.iso"
        storage.objects[key] = (b"old", {"peasy-tag": tag, "peasy-commit": COMMIT, "sha256": "b" * 64})
        return key

    def test_prepare_only_small_github_assets_with_whole_image_hashes(self):
        manifest, assets = self.prepare()
        self.assertEqual([item["desktop"] for item in manifest["images"]], ["gnome"])
        self.assertEqual({item.name for item in assets}, {
            "SHA256SUMS", "iso-downloads.json", *[f"{item['name']}.sha256" for item in manifest["images"]]})
        self.assertEqual(json.loads((self.output / "iso-downloads.json").read_text()), manifest)
        for item in manifest["images"]:
            self.assertIn(f"{item['sha256']}  {item['name']}\n", (self.output / "SHA256SUMS").read_text())
        self.assertFalse(list(self.output.glob("*.iso*part*")))

    def test_gnome_release_does_not_require_plasma_artifacts(self):
        for path in self.source.iterdir():
            if "plasma" in path.name:
                path.unlink()
        manifest, assets = self.prepare()
        self.assertEqual(len(manifest["images"]), 1)
        self.assertEqual(len(assets), 3)
        notes = r2_isos.notes(manifest)
        self.assertIn("XFCE installation requires an Internet connection", notes)
        self.assertNotIn("Plasma ISO", notes)

    def test_bad_input_never_produces_release_assets(self):
        for tag, commit in (("v1/path", COMMIT), (TAG, "short")):
            with self.assertRaises(ValueError):
                r2_isos.prepare(self.source, self.output, tag, commit, "https://downloads.askpeasy.com")
        (self.source / f"peasy-nixos-{TAG}-gnome-x86_64.iso.sha256").write_text("wrong")
        with self.assertRaises(ValueError):
            self.prepare()
        self.assertEqual(list(self.output.iterdir()), [])

    def test_invalid_verification_status_and_symlink(self):
        status = self.source / "gnome-iso-status.json"
        original = status.read_text()
        status.write_text('{"releaseReady": true}')
        with self.assertRaises(ValueError):
            self.prepare()
        status.write_text(original)
        image = self.source / f"peasy-nixos-{TAG}-gnome-x86_64.iso"
        image.rename(self.source / "real.iso")
        image.symlink_to(self.source / "real.iso")
        with self.assertRaises(ValueError):
            self.prepare()

    def test_r2_and_github_verified_before_publication_and_idempotent_retry(self):
        manifest, assets = self.prepare()
        storage, github = Storage(), GitHub()
        self.publish(manifest, assets, storage, github)
        self.assertFalse(github.release["draft"])
        self.assertIn(r2_isos.marker(manifest), github.release["body"])
        self.assertEqual(len(storage.uploads), 1)
        self.assertTrue(any(call[:2] == ("release", "edit") and "--latest" in call for call in github.calls))
        self.publish(manifest, assets, storage, github)
        self.assertEqual(len(storage.uploads), 1)

    def test_interrupted_upload_stays_private_then_resumes(self):
        manifest, assets = self.prepare()
        storage, github = Storage(), GitHub()
        storage.fail_desktop = "gnome"
        with self.assertRaises(RuntimeError):
            self.publish(manifest, assets, storage, github)
        self.assertTrue(github.release["draft"])
        storage.fail_desktop = None
        self.publish(manifest, assets, storage, github)
        self.assertEqual(len(storage.uploads), 1)

    def test_corrupt_remote_bytes_stay_private_even_with_correct_metadata(self):
        manifest, assets = self.prepare()
        storage, github = Storage(), GitHub()
        storage.corrupt = True
        with self.assertRaisesRegex(ValueError, "SHA-256"):
            self.publish(manifest, assets, storage, github)
        self.assertTrue(github.release["draft"])

    def test_metadata_conflict_never_overwritten(self):
        manifest, _ = self.prepare()
        storage = Storage()
        self.deliver(storage, manifest)
        key = storage.uploads[0]
        storage.objects[key][1]["sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "refusing to overwrite"):
            self.deliver(storage, manifest)
        self.assertEqual(len(storage.uploads), 1)

    def test_published_expired_iso_never_restored(self):
        manifest, _ = self.prepare()
        storage = Storage()
        with self.assertRaisesRegex(ValueError, "refusing to restore"):
            self.deliver(storage, manifest, published=True)
        self.assertFalse(storage.uploads)

    def test_public_access_failure_prevents_publication(self):
        manifest, assets = self.prepare()
        storage, github = Storage(), GitHub()
        def deliver(published):
            r2_isos.deliver(storage, "bucket", self.source, manifest, published, upload=storage.upload,
                            public_check=lambda item: (_ for _ in ()).throw(ValueError("Public access disabled")))
        with self.assertRaisesRegex(ValueError, "Public access"):
            release_isos.publish(assets, TAG, COMMIT, REPO, self.output, run=github,
                                 manifest=manifest, deliver=deliver)
        self.assertTrue(github.release["draft"])

    def test_changed_manifest_or_tag_rejected(self):
        manifest, assets = self.prepare()
        storage, github = Storage(), GitHub()
        self.publish(manifest, assets, storage, github)
        altered = copy.deepcopy(manifest)
        altered["images"][0]["url"] += "changed"
        with self.assertRaisesRegex(ValueError, "metadata differs"):
            self.publish(altered, assets, storage, github)
        github.commit = "b" * 40
        with self.assertRaisesRegex(ValueError, "tested commit"):
            self.publish(manifest, assets, storage, github)

    def test_api_failure_is_not_absence(self):
        with patch.object(Storage, "head_object", side_effect=RuntimeError("Access denied")):
            with self.assertRaisesRegex(RuntimeError, "Access denied"):
                r2_isos.head(Storage(), "bucket", "key")

    def test_public_check_identifies_peasy_without_credentials_or_redirects(self):
        manifest, _ = self.prepare()
        item = manifest["images"][0]
        with patch.object(r2_isos, "build_opener") as factory:
            opener = factory.return_value
            response = opener.open.return_value.__enter__.return_value
            response.status = 200
            response.headers = {"Content-Length": str(item["size"])}
            r2_isos.verify_public(item)
            factory.assert_called_once_with(r2_isos.NoRedirect)
            request = opener.open.call_args.args[0]
            self.assertEqual(request.full_url, item["url"])
            self.assertEqual(request.get_method(), "HEAD")
            self.assertEqual(dict(request.header_items()), {
                "User-agent": "Peasy-ReleaseVerifier/1.0 (+https://github.com/lnbits/peasy)"})
            opener.open.assert_called_once_with(request, timeout=60)
            self.assertIsNone(r2_isos.NoRedirect().redirect_request(
                request, None, 302, "Found", {}, "https://other.example/iso"))

    def test_public_check_still_rejects_wrong_status_or_size(self):
        manifest, _ = self.prepare()
        item = manifest["images"][0]
        for status, length in ((403, item["size"]), (206, item["size"]),
                               (200, item["size"] - 1), (200, None)):
            with self.subTest(status=status, length=length), patch.object(r2_isos, "build_opener") as factory:
                response = factory.return_value.open.return_value.__enter__.return_value
                response.status = status
                response.headers = {} if length is None else {"Content-Length": str(length)}
                with self.assertRaisesRegex(ValueError, "complete image"):
                    r2_isos.verify_public(item)

    def test_public_http_denial_is_not_bypassed(self):
        manifest, _ = self.prepare()
        item = manifest["images"][0]
        with patch.object(r2_isos, "build_opener") as factory:
            factory.return_value.open.side_effect = HTTPError(item["url"], 403, "Forbidden", {}, None)
            with self.assertRaises(HTTPError):
                r2_isos.verify_public(item)
            factory.return_value.open.assert_called_once()

    def test_prune_only_owned_older_images_after_latest_confirmation(self):
        manifest, _ = self.prepare()
        storage = Storage()
        self.deliver(storage, manifest)
        old = self.old_key(storage)
        old_plasma = self.old_key(storage, desktop="plasma")
        unrelated = self.old_key(storage, "v0.2.0")
        storage.objects[unrelated] = (b"not ours", {})
        storage.objects["unrelated.iso"] = (b"keep", {})
        r2_isos.prune(storage, "bucket", manifest, REPO, lambda *args: json.dumps(self.latest(manifest)))
        self.assertEqual(storage.deleted, [old, old_plasma])
        self.assertIn(unrelated, storage.objects)
        self.assertIn("unrelated.iso", storage.objects)
        self.assertTrue(all(key in storage.objects for key in storage.uploads))

    def test_cleanup_skips_non_latest_draft_and_prerelease(self):
        manifest, _ = self.prepare()
        for field, value in (("tag_name", "v2"), ("draft", True), ("prerelease", True), ("body", "")):
            storage = Storage()
            self.old_key(storage)
            release = self.latest(manifest)
            release[field] = value
            r2_isos.prune(storage, "bucket", manifest, REPO, lambda *args: json.dumps(release))
            self.assertFalse(storage.deleted)

    def test_cleanup_stops_when_latest_changes(self):
        manifest, _ = self.prepare()
        storage = Storage()
        self.old_key(storage)
        replies = iter([json.dumps(self.latest(manifest)), '{}'])
        with self.assertRaisesRegex(ValueError, "Latest release changed"):
            r2_isos.prune(storage, "bucket", manifest, REPO, lambda *args: next(replies))
        self.assertFalse(storage.deleted)

    def test_configuration_rejects_wrong_endpoint_and_credentials_in_url(self):
        env = {"R2_BUCKET": "peasy-releases", "R2_PUBLIC_URL": "https://downloads.askpeasy.com",
               "R2_ENDPOINT_URL": "https://" + "a" * 32 + ".r2.cloudflarestorage.com"}
        self.assertEqual(r2_isos.configuration(env)[1], "peasy-releases")
        for value in ("http://example.com", "https://example.com", env["R2_ENDPOINT_URL"] + "/peasy-releases",
                      "https://secret@example.com", "https://example.com?secret=abc"):
            with self.assertRaises(ValueError):
                r2_isos.configuration({**env, "R2_ENDPOINT_URL": value})


if __name__ == "__main__":
    unittest.main()
