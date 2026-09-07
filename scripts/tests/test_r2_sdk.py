"""Exercise the real pinned S3 SDK without network calls or real credentials."""
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import r2_isos

try:
    from botocore.stub import ANY, Stubber
except ImportError:
    Stubber = None


@unittest.skipIf(Stubber is None, "Run with iso-release-tools Python; also checked in the publish job")
class LockedSDK(unittest.TestCase):
    def test_managed_multipart_upload_and_missing_object(self):
        with patch.dict("os.environ", {"R2_ACCESS_KEY_ID": "test-only-placeholder",
                                      "R2_SECRET_ACCESS_KEY": "test-only-placeholder"}):
            storage = r2_isos.client("https://" + "a" * 32 + ".r2.cloudflarestorage.com")
        self.assertEqual(storage.meta.config.request_checksum_calculation, "when_required")
        self.assertEqual(storage.meta.config.response_checksum_validation, "when_required")
        with tempfile.TemporaryDirectory(prefix="peasy-r2-sdk-") as temporary:
            path = Path(temporary) / "peasy.iso"
            # A sparse fixture crosses the real 64 MiB multipart threshold without
            # filling local disk. Stubber prevents any request reaching the network.
            with path.open("wb") as stream:
                stream.truncate(65 * 1024**2)
            metadata = {"sha256": "b" * 64, "peasy-commit": "a" * 40, "peasy-tag": "v1.0.0"}
            base = {"Bucket": "peasy-releases", "Key": "test/key"}
            with Stubber(storage) as stub:
                stub.add_response("create_multipart_upload", {"UploadId": "test-upload"}, {
                    **base, "Metadata": metadata, "ContentType": "application/octet-stream",
                    "ContentDisposition": 'attachment; filename="peasy.iso"',
                    "CacheControl": "public, max-age=31536000, immutable"})
                for _ in range(2):
                    stub.add_response("upload_part", {"ETag": '"part-etag"'}, {
                        **base, "UploadId": "test-upload", "PartNumber": ANY, "Body": ANY})
                stub.add_response("complete_multipart_upload", {"ETag": '"complete-etag"'}, {
                    **base, "UploadId": "test-upload", "MultipartUpload": {"Parts": [
                        {"ETag": '"part-etag"', "PartNumber": 1},
                        {"ETag": '"part-etag"', "PartNumber": 2}]}})
                r2_isos.upload_file(storage, "peasy-releases", "test/key", path, metadata)
                stub.add_client_error("head_object", service_error_code="404", http_status_code=404,
                                      expected_params={**base})
                self.assertIsNone(r2_isos.head(storage, "peasy-releases", "test/key"))
                stub.assert_no_pending_responses()
