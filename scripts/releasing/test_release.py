"""Release correctness boundaries; no network, app data, or app compilation."""

import copy
import base64
import hashlib
from unittest.mock import patch
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
import struct
import tempfile
import unittest
from pathlib import Path

import release
from verify_bundle import pe_machine

SHA = "a" * 40
OTHER = "b" * 40
VERSION = "3.23.1"

# Test-only deterministic key, never used by production builds.
TEST_KEY = Ed25519PrivateKey.from_private_bytes(bytes(range(32)))
KEY_ID = b"fixture!"
TEST_PUBLIC = base64.b64encode(b"untrusted comment: test key\n" + base64.b64encode(b"Ed" + KEY_ID + TEST_KEY.public_key().public_bytes_raw()) + b"\n").decode()


def sign_fixture(data):
    signature = TEST_KEY.sign(hashlib.blake2b(data).digest())
    comment = b"test-only fixture"
    return base64.b64encode(b"untrusted comment: test\n" + base64.b64encode(b"ED" + KEY_ID + signature) + b"\ntrusted comment: " + comment + b"\n" + base64.b64encode(TEST_KEY.sign(signature + comment)) + b"\n").decode()


def fixture(root, name="lumagate"):
    (root / "src-tauri").mkdir(parents=True)
    release.write_json(root / "package.json", {"name": "lumagate", "version": VERSION})
    release.write_json(
        root / "src-tauri/tauri.conf.json",
        {
            "productName": "LumaGate",
            "version": VERSION,
            "bundle": {"createUpdaterArtifacts": True},
            "plugins": {"updater": {"pubkey": release.PUBLIC_KEY, "endpoints": [release.ENDPOINT]}},
        },
    )
    (root / "src-tauri/Cargo.toml").write_text(
        f'[package]\nname = "{name}"\nversion = "{VERSION}" # app\n\n'
        '[dependencies]\nserde = "1"\n',
        encoding="utf-8",
    )
    (root / "src-tauri/Cargo.lock").write_text(
        f'version = 4\n\n[[package]]\nname = "{name}"\nversion = "{VERSION}"\n\n'
        f'[[package]]\nname = "other"\nversion = "{VERSION}"\nsource = "registry+https://example.invalid"\n',
        encoding="utf-8",
    )


def artifacts(root, version=VERSION, sha=SHA):
    source = root / "artifacts"
    source.mkdir()
    for platform, (target, _) in release.PLATFORMS.items():
        directory = source / f"installers-{platform}"
        directory.mkdir()
        records = []
        for name in sorted(release.names(version, platform)):
            path = directory / name
            if name.endswith(".sig"):
                path.write_text(sign_fixture(directory.joinpath(name[:-4]).read_bytes()))
            else:
                path.write_bytes(f"fixture installer: {name}".encode())
            records.append(release.file_record(path))
        payload = {}
        if platform in release.WINDOWS_MACHINES:
            payload["windows_payload"] = {
                "installer": records[0],
                "executable": {
                    "name": "lumagate.exe",
                    "size": 128,
                    "sha256": "c" * 64,
                    "machine": release.WINDOWS_MACHINES[platform],
                    "version": version,
                    "product": "LumaGate",
                },
            }
        release.write_json(
            directory / "build.json",
            {
                "platform": platform,
                "target": target,
                "version": version,
                "sha": sha,
                "assets": records,
                **payload,
            },
        )
    return source


class FakeGitHub:
    """In-memory remote state, including interrupted uploads and public immutability."""

    def __init__(self):
        self.tags = {}
        self.refs = []
        self.releases = []
        self.assets = {}
        self.writes = []
        self.fail_upload = None
        self.latest = None

    def request(self, method, path, data=None, missing=False):
        if method == "GET":
            if path.startswith("/git/matching-refs/"):
                return copy.deepcopy(self.refs)
            if path.startswith("/git/tags/"):
                return copy.deepcopy(self.tags[path.split("/")[-1]])
            raise AssertionError(path)
        self.writes.append((method, path, copy.deepcopy(data)))
        if path == "/git/tags":
            key = str(len(self.tags) + 1)
            self.tags[key] = {
                "tag": data["tag"],
                "message": data["message"],
                "object": {"type": data["type"], "sha": data["object"]},
            }
            return {"sha": key}
        if path == "/git/refs":
            if any(r["ref"] == data["ref"] for r in self.refs):
                raise RuntimeError("Tag collision")
            self.refs.append(
                {"ref": data["ref"], "object": {"type": "tag", "sha": data["sha"]}}
            )
            return {}
        if path == "/releases":
            item = {**data, "id": len(self.writes)}
            self.releases.append(item)
            self.assets[item["id"]] = []
            return copy.deepcopy(item)
        release_id = int(path.split("/")[-1])
        item = next(r for r in self.releases if r["id"] == release_id)
        if method == "DELETE":
            if not item["draft"]:
                raise AssertionError("Attempted to delete public release")
            self.releases.remove(item)
            del self.assets[release_id]
            return None
        if method == "PATCH":
            item.update(data)
            if data["make_latest"] == "true":
                self.latest = item["tag_name"]
            return copy.deepcopy(item)
        raise AssertionError((method, path))

    def pages(self, path):
        if path == "/releases":
            return copy.deepcopy(self.releases)
        return copy.deepcopy(self.assets[int(path.split("/")[2])])

    def upload(self, release_id, path):
        if self.fail_upload == len(self.assets[release_id]):
            raise RuntimeError("Interrupted upload")
        record = release.file_record(path)
        self.assets[release_id].append(
            {
                "name": record["name"],
                "size": record["size"],
                "digest": "sha256:" + record["sha256"],
                "state": "uploaded",
            }
        )


class Versions(unittest.TestCase):
    def test_reservation_is_unique_and_retry_reuses_older_sha(self):
        api = FakeGitHub()
        self.assertEqual(release.reserve(api, VERSION, SHA), VERSION)
        self.assertEqual(release.reserve(api, VERSION, OTHER), "3.23.2")
        writes = copy.deepcopy(api.writes)
        self.assertEqual(release.reserve(api, "4.0.0", SHA), VERSION)
        self.assertEqual(api.writes, writes)

    def test_numeric_sequence_and_explicit_minor_major(self):
        history = [
            {"version": "3.23.9", "sha": OTHER},
            {"version": "3.23.10", "sha": "c" * 40},
        ]
        self.assertEqual(release.choose_version(VERSION, SHA, history), "3.23.11")
        self.assertEqual(release.choose_version("3.24.0", SHA, history), "3.24.0")
        self.assertEqual(release.choose_version("4.0.0", SHA, history), "4.0.0")
        with self.assertRaises(ValueError):
            release.choose_version("3.22.0", SHA, history)

    def test_malformed_and_overflow_versions_fail_closed(self):
        for value in ["3.23.1-beta", "03.23.1", "3.23.1+sha", "256.0.0", "1.2.65536"]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                release.semver(value)
        with self.assertRaises(ValueError):
            release.choose_version(
                VERSION, SHA, [{"version": "3.23.65535", "sha": OTHER}]
            )

    def test_foreign_or_moved_tag_cannot_publish(self):
        api = FakeGitHub()
        release.reserve(api, VERSION, SHA)
        api.tags["1"]["object"]["sha"] = OTHER
        with self.assertRaises(ValueError):
            release.assert_tag(api, VERSION, SHA)

    def test_stamp_updates_renamed_package_not_dependency(self):
        for name in ["cc-switch", "lumagate"]:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                fixture(root, name)
                release.stamp(root, "3.23.2")
                self.assertEqual(release.manifests(root)[0]["version"], "3.23.2")
                lock = release.tomllib.loads(
                    (root / "src-tauri/Cargo.lock").read_text()
                )
                self.assertEqual(lock["package"][1]["version"], VERSION)
                self.assertIn("# app", (root / "src-tauri/Cargo.toml").read_text())

    def test_divergent_manifests_rejected_before_write(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            fixture(root)
            release.write_json(root / "package.json", {"version": "4.0.0"})
            original = (root / "src-tauri/Cargo.toml").read_bytes()
            with self.assertRaises(ValueError):
                release.stamp(root, "4.0.1")
            self.assertEqual((root / "src-tauri/Cargo.toml").read_bytes(), original)


class AssetsAndPublication(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        signing_key = patch.object(release, "PUBLIC_KEY", TEST_PUBLIC)
        signing_key.start()
        self.addCleanup(signing_key.stop)
        self.source = artifacts(self.root)
        self.output = self.root / "release"
        self.api = FakeGitHub()
        release.reserve(self.api, VERSION, SHA)

    def assemble(self):
        return release.assemble(self.source, self.output, VERSION, SHA)

    def test_complete_inventory_and_checksum_corruption(self):
        records = self.assemble()
        self.assertEqual(
            {record["name"] for record in records},
            set().union(*(release.names(VERSION, p) for p in release.PLATFORMS))
            | {"BUILD-INFO.json", "SHA256SUMS", "latest.json"},
        )
        self.assertIn(
            f"LumaGate-{VERSION}-windows-arm64.exe", {r["name"] for r in records}
        )
        path = self.output / f"LumaGate-{VERSION}-linux-x64.deb"
        path.write_bytes(b"corrupted")
        with self.assertRaises(ValueError):
            release.validate_assets(self.output, VERSION, SHA)

    def test_missing_platform_blocks_all_publication(self):
        release.shutil.rmtree(self.source / "installers-windows-arm64")
        with self.assertRaises(ValueError):
            self.assemble()
        self.assertFalse(self.output.exists())
        self.assertEqual(self.api.releases, [])

    def test_mixed_source_and_unknown_assets_rejected(self):
        directory = self.source / "installers-linux-x64"
        build = release.read_json(directory / "build.json")
        build["sha"] = OTHER
        release.write_json(directory / "build.json", build)
        with self.assertRaises(ValueError):
            self.assemble()
        build["sha"] = SHA
        release.write_json(directory / "build.json", build)
        (directory / "unexpected.sig").write_bytes(b"not an installer")
        with self.assertRaises(ValueError):
            self.assemble()

    def test_partial_upload_stays_draft_then_retry_completes(self):
        self.assemble()
        self.api.fail_upload = 2
        with self.assertRaises(RuntimeError):
            release.publish(self.api, self.output, VERSION, SHA)
        self.assertTrue(self.api.releases[0]["draft"])
        self.assertIsNone(self.api.latest)
        self.api.fail_upload = None
        release.publish(self.api, self.output, VERSION, SHA)
        self.assertEqual(len(self.api.releases), 1)
        self.assertFalse(self.api.releases[0]["draft"])
        self.assertFalse(self.api.releases[0]["prerelease"])
        self.assertEqual(self.api.latest, release.PREFIX + VERSION)
        self.assertEqual(
            {a["name"] for a in self.api.assets[self.api.releases[0]["id"]]},
            {p.name for p in self.output.iterdir()},
        )

    def test_public_retry_does_not_write_or_replace_assets(self):
        self.assemble()
        release.publish(self.api, self.output, VERSION, SHA)
        writes = copy.deepcopy(self.api.writes)
        release.publish(self.api, self.output, VERSION, SHA)
        self.assertEqual(self.api.writes, writes)
        self.api.assets[self.api.releases[0]["id"]][0]["digest"] = "sha256:" + "0" * 64
        with self.assertRaises(ValueError):
            release.publish(self.api, self.output, VERSION, SHA)
        self.assertEqual(self.api.writes, writes)

    def test_older_retry_does_not_steal_latest(self):
        self.assemble()
        self.api.releases.append(
            {
                "id": 999,
                "tag_name": "lumagate-v3.23.2",
                "draft": False,
                "prerelease": False,
            }
        )
        self.api.latest = "lumagate-v3.23.2"
        release.publish(self.api, self.output, VERSION, SHA)
        self.assertEqual(self.api.latest, "lumagate-v3.23.2")

    def test_duplicate_discovered_installer_is_not_arbitrarily_selected(self):
        root = self.root / "checkout"
        fixture(root)
        directory = root / "src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis"
        directory.mkdir(parents=True)
        for suffix in ["one", "two"]:
            (directory / f"LumaGate_{VERSION}_{suffix}.exe").write_bytes(b"fixture")
        with self.assertRaises(ValueError):
            release.stage(root, VERSION, SHA, "windows-x64", self.root / "staged")

    def test_arm64_receipt_rejects_rebranded_x64_and_changed_installer(self):
        path = self.source / "installers-windows-arm64/build.json"
        original = release.read_json(path)
        for field, value in [("machine", 0x8664), ("version", "9.0.0")]:
            build = copy.deepcopy(original)
            build["windows_payload"]["executable"][field] = value
            release.write_json(path, build)
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.assemble()
        build = copy.deepcopy(original)
        build["windows_payload"]["installer"]["sha256"] = "0" * 64
        release.write_json(path, build)
        with self.assertRaises(ValueError):
            self.assemble()

    def test_historical_inventory_verifies_without_rewriting_assets(self):
        self.assemble()
        info_path = self.output / "BUILD-INFO.json"
        info = release.read_json(info_path)
        del info["schema"]
        info["platforms"] = [
            p for p in info["platforms"] if p["platform"] != "windows-arm64"
        ]
        for build in info["platforms"]:
            build.pop("windows_payload", None)
            build["assets"] = [a for a in build["assets"] if a["name"] in release.names(VERSION, build["platform"], release.LEGACY_PLATFORMS)]
        release.write_json(info_path, info)
        expected = set().union(*(release.names(VERSION, p, release.LEGACY_PLATFORMS) for p in release.LEGACY_PLATFORMS if p != "windows-arm64")) | {"BUILD-INFO.json", "SHA256SUMS"}
        for path in self.output.iterdir():
            if path.name not in expected:
                path.unlink()
        records = [
            release.file_record(p)
            for p in sorted(self.output.iterdir())
            if p.name != "SHA256SUMS"
        ]
        (self.output / "SHA256SUMS").write_text(
            "".join(f"{r['sha256']}  {r['name']}\n" for r in records)
        )
        before = {p.name: p.read_bytes() for p in self.output.iterdir()}
        release.validate_assets(self.output, VERSION, SHA)
        self.assertEqual(
            before, {p.name: p.read_bytes() for p in self.output.iterdir()}
        )
        with self.assertRaises(ValueError):
            release.inventory_platforms(info, "3.24.3")

    def test_signature_rejects_tampered_payload(self):
        self.assemble()
        asset = self.output / f"LumaGate-{VERSION}-windows-arm64.exe"
        signature = Path(str(asset) + ".sig").read_text()
        release.verify_signature(asset, signature, release.PUBLIC_KEY)
        asset.write_bytes(asset.read_bytes() + b"tampered")
        with self.assertRaises(ValueError):
            release.verify_signature(asset, signature, release.PUBLIC_KEY)

    def test_manifest_cannot_redirect_one_architecture_to_another(self):
        self.assemble()
        path = self.output / "latest.json"
        latest = release.read_json(path)
        latest["platforms"]["windows-aarch64"] = latest["platforms"]["windows-x86_64"]
        release.write_json(path, latest)
        with self.assertRaises(ValueError):
            release.validate_assets(self.output, VERSION, SHA)


class WindowsPE(unittest.TestCase):
    def test_reads_payload_machine_not_filename(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "LumaGate-windows-arm64.exe"
            header = bytearray(134)
            header[:2] = b"MZ"
            struct.pack_into("<I", header, 0x3C, 128)
            for machine in (0xAA64, 0x8664, 0x14C):
                header[128:] = b"PE\0\0" + struct.pack("<H", machine)
                path.write_bytes(header)
                self.assertEqual(pe_machine(path), machine)
            path.write_bytes(header[:65])
            with self.assertRaises(ValueError):
                pe_machine(path)


if __name__ == "__main__":
    unittest.main()
