#!/usr/bin/env python3
"""LumaGate release lifecycle. Python 3.11+, standard library only."""

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import Request, urlopen
from datetime import datetime, timezone
from signatures import verify as verify_signature
import tempfile

REPOSITORY = "Restry/LumaGate"
PREFIX = "lumagate-v"
LEGACY_PLATFORMS = {
    "macos-arm64": ("aarch64-apple-darwin", {"dmg": "dmg/*.dmg"}),
    "macos-x64": ("x86_64-apple-darwin", {"dmg": "dmg/*.dmg"}),
    "windows-x64": ("x86_64-pc-windows-msvc", {"exe": "nsis/*.exe"}),
    "windows-arm64": ("aarch64-pc-windows-msvc", {"exe": "nsis/*.exe"}),
    "linux-x64": (
        "x86_64-unknown-linux-gnu",
        {
            "AppImage": "appimage/*.AppImage",
            "deb": "deb/*.deb",
        },
    ),
}
UPDATER_TARGETS = {
    "macos-arm64": ("darwin-aarch64", "app.tar.gz", "macos/*.app.tar.gz"),
    "macos-x64": ("darwin-x86_64", "app.tar.gz", "macos/*.app.tar.gz"),
    "windows-x64": ("windows-x86_64", "exe", "nsis/*.exe"),
    "windows-arm64": ("windows-aarch64", "exe", "nsis/*.exe"),
    "linux-x64": ("linux-x86_64", "AppImage", "appimage/*.AppImage"),
}
PLATFORMS = {
    platform: (target, {**patterns, UPDATER_TARGETS[platform][1]: UPDATER_TARGETS[platform][2],
                       UPDATER_TARGETS[platform][1] + ".sig": UPDATER_TARGETS[platform][2] + ".sig"})
    for platform, (target, patterns) in LEGACY_PLATFORMS.items()
}
ENDPOINT = f"https://github.com/{REPOSITORY}/releases/latest/download/latest.json"
PUBLIC_KEY = json.loads((Path(__file__).resolve().parents[2] / "src-tauri/tauri.conf.json").read_text())["plugins"]["updater"]["pubkey"]
WINDOWS_MACHINES = {"windows-x64": 0x8664, "windows-arm64": 0xAA64}
INVENTORY_SCHEMA = 3


def inventory_platforms(info, version):
    schema = info.get("schema", 1)
    if schema == 1:
        # Existing public assets are immutable, not incomplete new releases.
        require(
            semver(version) <= (3, 24, 2),
            "New releases require the ARM64 inventory schema",
        )
        return {p: spec for p, spec in LEGACY_PLATFORMS.items() if p != "windows-arm64"}
    if schema == 2:
        require(semver(version) <= (3, 24, 6), "New releases require signed updater artifacts")
        return LEGACY_PLATFORMS
    require(schema == INVENTORY_SCHEMA, "Unsupported release inventory schema")
    return PLATFORMS


def validate_windows_payload(build):
    if build["platform"] not in WINDOWS_MACHINES:
        return
    receipt = build.get("windows_payload", {})
    payload = receipt.get("executable", {})
    installer = receipt.get("installer", {})
    asset = next(a for a in build["assets"] if a["name"].endswith(".exe"))
    require(
        installer.get("sha256") == asset["sha256"]
        and installer.get("size") == asset["size"]
        and payload.get("machine") == WINDOWS_MACHINES[build["platform"]]
        and payload.get("version") == build["version"]
        and payload.get("product") == "LumaGate"
        and payload.get("size", 0) > 0
        and re.fullmatch(r"[0-9a-f]{64}", payload.get("sha256", ""))
        and Path(payload.get("name", "")).suffix == ".exe",
        f"Missing or mismatched Windows payload receipt: {build['platform']}",
    )


def require(condition, message):
    if not condition:
        raise ValueError(message)


def semver(value):
    require(
        isinstance(value, str)
        and re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", value),
        f"Expected stable MAJOR.MINOR.PATCH, got {value!r}",
    )
    parts = tuple(map(int, value.split(".")))
    require(
        parts[0] <= 255 and parts[1] <= 255 and parts[2] <= 65535,
        "Version exceeds Windows installer limits; explicitly bump minor/major",
    )
    return parts


def read_json(path):
    return json.loads(Path(path).read_text(encoding="utf-8"))


def write_json(path, value):
    Path(path).write_text(
        json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )


def run(*args, cwd=None):
    return subprocess.check_output(args, cwd=cwd, text=True).strip()


def manifests(root):
    package = read_json(root / "package.json")
    tauri = read_json(root / "src-tauri/tauri.conf.json")
    cargo = tomllib.loads((root / "src-tauri/Cargo.toml").read_text(encoding="utf-8"))
    lock = tomllib.loads((root / "src-tauri/Cargo.lock").read_text(encoding="utf-8"))
    own = [
        p
        for p in lock["package"]
        if p["name"] == cargo["package"]["name"] and "source" not in p
    ]
    require(len(own) == 1, "Cargo.lock must contain exactly one local app package")
    versions = [
        package["version"],
        tauri["version"],
        cargo["package"]["version"],
        own[0]["version"],
    ]
    require(len(set(versions)) == 1, f"Unsynchronized source versions: {versions}")
    semver(versions[0])
    require(
        tauri["productName"] == "LumaGate",
        "App owner must finish LumaGate product branding",
    )
    require(tauri.get("bundle", {}).get("createUpdaterArtifacts") is True, "Signed updates require createUpdaterArtifacts=true")
    updater = tauri.get("plugins", {}).get("updater", {})
    require(updater.get("pubkey") == PUBLIC_KEY and updater.get("endpoints") == [ENDPOINT], "Updater key/source must be pinned to LumaGate")
    return package, tauri, cargo


def replace_toml_version(text, table, name, version):
    # Parse first, then replace only the validated table's scalar; preserve comments.
    tomllib.loads(text)
    chunks = re.split(r"(?m)(?=^\[)", text)
    found = 0
    for i, chunk in enumerate(chunks):
        if not chunk.startswith(table + "\n"):
            continue
        parsed = tomllib.loads(chunk)
        entry = parsed["package"] if table == "[package]" else parsed["package"][0]
        if entry.get("name") != name or "source" in entry:
            continue
        chunks[i], count = re.subn(
            r'(?m)^(version\s*=\s*)"[^"\n]+"',
            lambda m: m[1] + json.dumps(version),
            chunk,
        )
        require(count == 1, "Expected one app version assignment")
        found += 1
    require(found == 1, "Expected one local app package table")
    result = "".join(chunks)
    tomllib.loads(result)
    return result


def stamp(root, version):
    semver(version)
    package, tauri, cargo = manifests(root)
    name = cargo["package"]["name"]
    replacements = {}
    for relative, table in [
        ("src-tauri/Cargo.toml", "[package]"),
        ("src-tauri/Cargo.lock", "[[package]]"),
    ]:
        path = root / relative
        replacements[path] = replace_toml_version(
            path.read_text(encoding="utf-8"), table, name, version
        )
    package["version"] = tauri["version"] = version
    for path, content in replacements.items():
        path.write_text(content, encoding="utf-8")
    write_json(root / "package.json", package)
    write_json(root / "src-tauri/tauri.conf.json", tauri)
    manifests(root)
    print(f"Stamped all four build-checkout manifests: {version}")


class GitHub:
    def request(self, method, path, data=None, missing=False):
        require(path.startswith("/"), "Expected repository-relative API path")
        request = Request(
            f"https://api.github.com/repos/{REPOSITORY}{path}",
            data=None if data is None else json.dumps(data).encode(),
            method=method,
            headers={
                "Authorization": f"Bearer {os.environ['GH_TOKEN']}",
                "Accept": "application/vnd.github+json",
                "X-GitHub-Api-Version": "2022-11-28",
                "Content-Type": "application/json",
            },
        )
        try:
            with urlopen(request, timeout=60) as response:
                body = response.read()
                return json.loads(body) if body else None
        except HTTPError as error:
            if missing and error.code == 404:
                return None
            # Never log request headers or server echoes of credentials.
            raise RuntimeError(f"GitHub {method} {path}: HTTP {error.code}") from None

    def pages(self, path):
        result = []
        page = 1
        while True:
            batch = self.request("GET", f"{path}?per_page=100&page={page}")
            require(isinstance(batch, list), "Expected paginated GitHub array")
            result.extend(batch)
            if len(batch) < 100:
                return result
            page += 1

    def upload(self, release_id, path):
        # gh streams the binary and rejects existing names; never use --clobber.
        subprocess.run(
            ["gh", "release", "upload", self.tag, str(path), "--repo", REPOSITORY],
            check=True,
        )


def guard(root, sha):
    require(
        os.environ.get("GITHUB_REPOSITORY") == REPOSITORY,
        "Publishing repository is not Restry/LumaGate",
    )
    require(
        os.environ.get("GITHUB_REF") == "refs/heads/release",
        "Only release branch may publish",
    )
    require(
        os.environ.get("GITHUB_EVENT_NAME") in ("push", "workflow_dispatch"),
        "Invalid publishing event",
    )
    require(re.fullmatch(r"[0-9a-f]{40}", sha), "Invalid source SHA")
    require(
        os.environ.get("GITHUB_SHA") == sha, "Source SHA differs from workflow event"
    )
    require(
        run("git", "rev-parse", "HEAD", cwd=root) == sha,
        "Checkout is not the exact source SHA",
    )


def reservations(api):
    refs = api.request("GET", f"/git/matching-refs/tags/{PREFIX}")
    records = []
    for ref in refs:
        tag = ref["ref"].removeprefix("refs/tags/")
        version = tag.removeprefix(PREFIX)
        semver(version)
        require(
            ref["object"]["type"] == "tag",
            f"Ref {tag} is not an annotated release reservation",
        )
        obj = api.request("GET", f"/git/tags/{ref['object']['sha']}")
        require(
            obj["object"]["type"] == "commit",
            f"Ref {tag} does not point directly to a commit",
        )
        sha = obj["object"]["sha"]
        require(
            obj["tag"] == tag
            and obj["message"].strip() == f"LumaGate release {version}\nSource: {sha}",
            f"Unrecognized reservation {tag}; refusing to reuse or overwrite it",
        )
        records.append({"version": version, "sha": sha, "tag": tag})
    return records


def choose_version(base, sha, records):
    base_parts = semver(base)
    same = [r for r in records if r["sha"] == sha]
    require(len(same) <= 1, "Multiple release versions point to this source SHA")
    if same:
        return same[0]["version"]
    if not records:
        return base
    highest = max(semver(r["version"]) for r in records)
    require(
        base_parts[:2] >= highest[:2],
        "Base version is behind current major/minor release series",
    )
    if base_parts > highest:
        return base
    version = f"{highest[0]}.{highest[1]}.{highest[2] + 1}"
    semver(version)
    return version


def assert_tag(api, version, sha):
    matches = [r for r in reservations(api) if r["version"] == version]
    require(
        len(matches) == 1 and matches[0]["sha"] == sha,
        "Reserved tag/source integrity mismatch",
    )


def reserve(api, base, sha):
    records = reservations(api)
    version = choose_version(base, sha, records)
    tag = PREFIX + version
    if not any(r["sha"] == sha for r in records):
        obj = api.request(
            "POST",
            "/git/tags",
            {
                "tag": tag,
                "message": f"LumaGate release {version}\nSource: {sha}",
                "object": sha,
                "type": "commit",
            },
        )
        api.request("POST", "/git/refs", {"ref": f"refs/tags/{tag}", "sha": obj["sha"]})
    assert_tag(api, version, sha)
    return version


def names(version, platform, platforms=None):
    return {f"LumaGate-{version}-{platform}.{ext}" for ext in (platforms or PLATFORMS)[platform][1]}


def file_record(path):
    require(path.is_file() and not path.is_symlink(), f"Not a regular asset: {path}")
    require(path.stat().st_size > 0, f"Empty asset: {path}")
    with path.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    return {"name": path.name, "size": path.stat().st_size, "sha256": digest}


def stage(root, version, sha, platform, output):
    package, _, _ = manifests(root)
    require(
        package["version"] == version, "Build checkout has not been version-stamped"
    )
    target, patterns = PLATFORMS[platform]
    bundle = root / "src-tauri/target" / target / "release/bundle"
    require(not output.exists(), "Stage destination must be new")
    sources = []
    for ext, pattern in patterns.items():
        candidates = list(bundle.glob(pattern))
        require(
            len(candidates) == 1,
            f"Expected exactly one {platform} {ext}, found {len(candidates)}",
        )
        source = candidates[0]
        require(
            platform.startswith("macos-") and ext in ("app.tar.gz", "app.tar.gz.sig") or version in source.name,
            f"Bundle filename lacks embedded build version: {source.name}",
        )
        file_record(source)
        sources.append((source, f"LumaGate-{version}-{platform}.{ext}"))
    output.mkdir(parents=True)
    assets = []
    for source, name in sources:
        shutil.copyfile(source, output / name)
        assets.append(file_record(output / name))
    _, extension, _ = UPDATER_TARGETS[platform]
    asset = output / f"LumaGate-{version}-{platform}.{extension}"
    verify_signature(asset, Path(str(asset) + ".sig").read_text(), PUBLIC_KEY)
    build = {
        "version": version,
        "sha": sha,
        "platform": platform,
        "target": target,
        "assets": assets,
    }
    if platform in WINDOWS_MACHINES:
        build["windows_payload"] = read_json(bundle.parent / "windows-payload.json")
        validate_windows_payload(build)
    write_json(output / "build.json", build)
    print(f"Staged {platform}: {', '.join(a['name'] for a in assets)}")


def assemble(source, output, version, sha):
    require(not output.exists(), "Assembly destination must be new")
    require(
        {p.name for p in source.iterdir()} == {f"installers-{p}" for p in PLATFORMS},
        f"Expected exactly {len(PLATFORMS)} platform artifact directories",
    )
    records = []
    sources = []
    for platform, (target, _) in PLATFORMS.items():
        directory = source / f"installers-{platform}"
        require(not directory.is_symlink(), "Symlink artifact directory")
        expected = names(version, platform)
        require(
            {p.name for p in directory.iterdir()} == expected | {"build.json"},
            f"Unexpected or missing files for {platform}",
        )
        build = read_json(directory / "build.json")
        require(
            (build["version"], build["sha"], build["platform"], build["target"])
            == (version, sha, platform, target),
            f"Mixed source/version/platform in {platform}",
        )
        actual = [file_record(directory / name) for name in sorted(expected)]
        require(
            sorted(build["assets"], key=lambda a: a["name"]) == actual,
            f"Asset checksum/size mismatch in {platform}",
        )
        validate_windows_payload(build)
        records.append(build)
        sources.extend(directory / name for name in sorted(expected))
    output.mkdir(parents=True)
    for path in sources:
        shutil.copyfile(path, output / path.name)
    latest = {"version": version, "notes": update_notes(), "pub_date": datetime.now(timezone.utc).isoformat(), "platforms": {}}
    for platform, (target, extension, _) in UPDATER_TARGETS.items():
        asset = output / f"LumaGate-{version}-{platform}.{extension}"
        signature = Path(str(asset) + ".sig").read_text().strip()
        verify_signature(asset, signature, PUBLIC_KEY)
        latest["platforms"][target] = {"url": f"https://github.com/{REPOSITORY}/releases/download/{PREFIX}{version}/{asset.name}", "signature": signature}
    write_json(output / "latest.json", latest)
    write_json(
        output / "BUILD-INFO.json",
        {
            "schema": INVENTORY_SCHEMA,
            "repository": REPOSITORY,
            "source": sha,
            "version": version,
            "tag": PREFIX + version,
            "signing": "Tauri minisign updater signatures; macOS ad-hoc only, no notarization; Windows no Authenticode",
            "updater_public_key_sha256": hashlib.sha256(PUBLIC_KEY.encode()).hexdigest(),
            "platforms": records,
        },
    )
    checksums = [file_record(path) for path in sorted(output.iterdir())]
    (output / "SHA256SUMS").write_text(
        "".join(f"{a['sha256']}  {a['name']}\n" for a in checksums), encoding="utf-8"
    )
    return validate_assets(output, version, sha)


def validate_assets(directory, version, sha):
    info = read_json(directory / "BUILD-INFO.json")
    platforms = inventory_platforms(info, version)
    expected = set().union(*(names(version, p, platforms) for p in platforms)) | {"BUILD-INFO.json", "SHA256SUMS"}
    if info.get("schema") == INVENTORY_SCHEMA:
        expected.add("latest.json")
        latest = read_json(directory / "latest.json")
        require(latest["version"] == version and set(latest["platforms"]) == {t[0] for t in UPDATER_TARGETS.values()}, "Updater manifest version/targets mismatch")
        require(bool(latest.get("notes")) and datetime.fromisoformat(latest["pub_date"]).tzinfo is not None, "Updater notes/date missing")
        require(info.get("updater_public_key_sha256") == hashlib.sha256(PUBLIC_KEY.encode()).hexdigest(), "Updater key provenance mismatch")
        for platform, (target, extension, _) in UPDATER_TARGETS.items():
            asset = directory / f"LumaGate-{version}-{platform}.{extension}"
            entry = latest["platforms"][target]
            signature = Path(str(asset) + ".sig").read_text().strip()
            require(entry == {"signature": signature, "url": f"https://github.com/{REPOSITORY}/releases/download/{PREFIX}{version}/{asset.name}"}, f"Updater asset mapping mismatch: {target}")
            verify_signature(asset, signature, PUBLIC_KEY)
    require(
        {p.name for p in directory.iterdir()} == expected,
        f"Release must contain exactly {len(expected)} assets",
    )
    require(
        (info["source"], info["version"], info["tag"], info["repository"])
        == (sha, version, PREFIX + version, REPOSITORY),
        "Release provenance mismatch",
    )
    records = [file_record(directory / name) for name in sorted(expected)]
    require(
        len(info["platforms"]) == len(platforms)
        and {build["platform"] for build in info["platforms"]} == set(platforms),
        "Release platform provenance is incomplete",
    )
    by_name = {record["name"]: record for record in records}
    for build in info["platforms"]:
        platform = build["platform"]
        require(
            (build["sha"], build["version"], build["target"])
            == (sha, version, PLATFORMS[platform][0])
            and sorted(build["assets"], key=lambda asset: asset["name"])
            == [by_name[name] for name in sorted(names(version, platform, platforms))],
            f"Release platform provenance mismatch: {platform}",
        )
        if info.get("schema", 1) >= 2:
            validate_windows_payload(build)
    expected_sums = "".join(
        f"{r['sha256']}  {r['name']}\n" for r in records if r["name"] != "SHA256SUMS"
    )
    require(
        (directory / "SHA256SUMS").read_text(encoding="utf-8") == expected_sums,
        "SHA256SUMS mismatch",
    )
    return records


def remote_assets(api, release, expected):
    actual = api.pages(f"/releases/{release['id']}/assets")
    require(len(actual) == len(expected), "Remote release asset count mismatch")
    by_name = {a["name"]: a for a in actual}
    require(
        set(by_name) == {a["name"] for a in expected},
        "Remote release asset inventory mismatch",
    )
    for asset in expected:
        remote = by_name[asset["name"]]
        require(
            remote["state"] == "uploaded"
            and remote["size"] == asset["size"]
            and remote.get("digest") == "sha256:" + asset["sha256"],
            f"Remote digest/size/state mismatch: {asset['name']}",
        )


def update_notes():
    return (
        "Provider 新增参与费用估算开关，所有来源默认开启；关闭不影响路由或 Token 统计。\n"
        "修复原生 GitHub Copilot 已知模型未计价，按对应模型标准价估算。\n"
        "历史费用按最终来源设置重算，区分主动关闭与缺少价格；保留日志、凭据及客户端配置。"
    )


def release_notes(version, sha, assets):
    tag = PREFIX + version
    links = "\n".join(
        f"- [{a['name']}](https://github.com/{REPOSITORY}/releases/download/{tag}/{a['name']})"
        for a in assets
    )
    return (
        f"## LumaGate {version}\n\nSource: `{sha}`\n\n{update_notes()}\n\n{links}\n\n"
        "Verify downloads against SHA256SUMS. BUILD-INFO.json records source, targets, and Windows NSIS payload PE metadata.\n\n"
        f"Changes and verification boundaries: https://github.com/{REPOSITORY}/blob/{sha}/CHANGELOG.md\n\n"
        "macOS uses ad-hoc signing, NOT Apple notarization. Windows has no Authenticode signature and may show SmartScreen warnings. "
        "Windows ARM64 runtime upgrade has not been tested on this Mac. Linux AppImage may require FUSE; the build baseline is Ubuntu 22.04. "
        "The dedicated Tauri updater signature is mandatory and independent of platform code signing.\n"
    )


def publish(api, directory, version, sha):
    expected = validate_assets(directory, version, sha)
    assert_tag(api, version, sha)
    tag = PREFIX + version
    api.tag = tag
    releases = api.pages("/releases")
    matches = [r for r in releases if r["tag_name"] == tag]
    require(len(matches) <= 1, "Duplicate releases for tag")
    release = matches[0] if matches else None
    marker = f"Source: `{sha}`"
    if release:
        require(
            marker in (release.get("body") or ""),
            "Existing release has different provenance",
        )
        if not release["draft"]:
            require(
                not release["prerelease"], "Existing release is unexpectedly prerelease"
            )
            remote_assets(api, release, expected)
            print(f"Already published, unchanged: {tag}")
            return
        # Only our unpublished draft can be discarded. Never delete tags or public assets.
        api.request("DELETE", f"/releases/{release['id']}")
    # The existing annotated tag is the source authority (asserted above and below).
    # Explicit historical target_commitish triggers GitHub's workflow-write check
    # when dev has advanced its workflow; GITHUB_TOKEN cannot have that permission.
    release = api.request(
        "POST",
        "/releases",
        {
            "tag_name": tag,
            "name": f"LumaGate {version}",
            "body": release_notes(version, sha, expected),
            "draft": True,
            "prerelease": False,
            "make_latest": "false",
        },
    )
    for record in expected:
        api.upload(release["id"], directory / record["name"])
    remote_assets(api, release, expected)
    assert_tag(api, version, sha)
    # Backfilling an older failed source must not move Latest backwards.
    newer = any(
        r["tag_name"].startswith(PREFIX)
        and not r["draft"]
        and not r["prerelease"]
        and semver(r["tag_name"][len(PREFIX) :]) > semver(version)
        for r in releases
    )
    api.request(
        "PATCH",
        f"/releases/{release['id']}",
        {
            "draft": False,
            "prerelease": False,
            "make_latest": "false" if newer else "true",
        },
    )
    print(
        f"Published complete release: https://github.com/{REPOSITORY}/releases/tag/{tag}"
    )


def verify_public(directory, version, sha):
    expected = validate_assets(directory, version, sha)
    with tempfile.TemporaryDirectory(prefix="lumagate-public-") as temporary:
        downloaded = Path(temporary)
        for record in expected:
            url = f"https://github.com/{REPOSITORY}/releases/download/{PREFIX}{version}/{record['name']}"
            with urlopen(Request(url, headers={"User-Agent": "LumaGate-release-verification"}), timeout=120) as response:
                require(response.status == 200, "Anonymous asset GET failed")
                with (downloaded / record["name"]).open("wb") as stream:
                    shutil.copyfileobj(response, stream)
            require(file_record(downloaded / record["name"]) == record, f"Public download mismatch: {record['name']}")
        validate_assets(downloaded, version, sha)
    with urlopen(Request(ENDPOINT, headers={"User-Agent": "LumaGate-release-verification"}), timeout=60) as response:
        latest = json.load(response)
    require(semver(latest["version"]) >= semver(version), "Public latest endpoint points backwards")
    if latest["version"] == version:
        require(latest == read_json(directory / "latest.json"), "Anonymous latest manifest mismatch")
    print(f"Verified anonymous latest.json and {len(expected)} public downloads, hashes and updater signatures: {version}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "command",
        choices=["check", "reserve", "stamp", "stage", "assemble", "publish", "verify", "public"],
    )
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--version")
    parser.add_argument("--sha", default=os.environ.get("GITHUB_SHA"))
    parser.add_argument("--platform", choices=PLATFORMS)
    parser.add_argument("--input", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.command == "check":
        print(f"Validated base version: {manifests(args.root)[0]['version']}")
        return
    if args.command in ("reserve", "publish"):
        guard(args.root, args.sha)
        api = GitHub()
    if args.command == "reserve":
        version = reserve(api, manifests(args.root)[0]["version"], args.sha)
        # Published reruns verify retained assets instead of rebuilding non-reproducible installers.
        release = api.request("GET", f"/releases/tags/{PREFIX}{version}", missing=True)
        published = release is not None and not release["draft"]
        if published:
            require(
                not release["prerelease"]
                and f"Source: `{args.sha}`" in (release.get("body") or ""),
                "Existing public release has invalid provenance/state",
            )
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
            output.write(
                f"version={version}\ntag={PREFIX}{version}\npublished={str(published).lower()}\n"
            )
        print(f"Reserved {PREFIX}{version} at {args.sha}; published={published}")
        return
    semver(args.version)
    if args.command == "stamp":
        stamp(args.root, args.version)
    elif args.command == "stage":
        stage(args.root, args.version, args.sha, args.platform, args.output)
    elif args.command == "assemble":
        assemble(args.input, args.output, args.version, args.sha)
    elif args.command == "publish":
        publish(api, args.input, args.version, args.sha)
    elif args.command == "verify":
        records = validate_assets(args.input, args.version, args.sha)
        print(f"Verified all {len(records)} release assets and checksums")
    elif args.command == "public":
        verify_public(args.input, args.version, args.sha)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, RuntimeError) as error:
        sys.exit(str(error))
