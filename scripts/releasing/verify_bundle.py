#!/usr/bin/env python3
"""Check embedded versions and native architectures without launching the app."""

import argparse
import json
import os
import plistlib
import struct
import tempfile
from pathlib import Path

from release import (
    PLATFORMS,
    WINDOWS_MACHINES,
    file_record,
    manifests,
    require,
    run,
    semver,
    write_json,
)


def one(directory, pattern):
    paths = list(directory.glob(pattern))
    require(
        len(paths) == 1, f"Expected one {pattern} under {directory}, found {len(paths)}"
    )
    return paths[0]


def pe_machine(executable):
    with executable.open("rb") as stream:
        require(stream.read(2) == b"MZ", "Not a Windows executable")
        stream.seek(0x3C)
        offset = stream.read(4)
        require(len(offset) == 4, "Truncated DOS header")
        stream.seek(struct.unpack("<I", offset)[0])
        header = stream.read(6)
    require(len(header) == 6 and header[:4] == b"PE\0\0", "Invalid PE header")
    return struct.unpack("<H", header[4:])[0]


def windows_payload(installer, executable, platform, version):
    # NSIS's bootstrap PE may be x86 even when the installed app is ARM64.
    # Extract without executing either installer or app; verify the actual payload.
    with tempfile.TemporaryDirectory(prefix="lumagate-nsis-") as tmp:
        run("7z", "x", "-y", f"-o{tmp}", str(installer.resolve()))
        packaged = one(Path(tmp), f"**/{executable.name}")
        machine = pe_machine(packaged)
        require(
            machine == WINDOWS_MACHINES[platform],
            "Windows payload architecture mismatch",
        )
        record = file_record(packaged)
        require(
            record == file_record(executable),
            "NSIS payload differs from built executable",
        )
        os.environ["LUMAGATE_VERIFY_EXE"] = str(packaged.resolve())
        info = json.loads(
            run(
                "powershell",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "(Get-Item -LiteralPath $env:LUMAGATE_VERIFY_EXE).VersionInfo | "
                "Select-Object ProductVersion,ProductName | ConvertTo-Json -Compress",
            )
        )
        require(
            info["ProductVersion"] == version and info["ProductName"] == "LumaGate",
            "Windows packaged executable version/product mismatch",
        )
    receipt = {
        "installer": file_record(installer),
        "executable": {
            **record,
            "machine": machine,
            "version": version,
            "product": "LumaGate",
        },
    }
    print(f"Verified NSIS payload: {json.dumps(receipt, sort_keys=True)}")
    return receipt


def verify(root, platform, version):
    semver(version)
    _, config, _ = manifests(root)
    require(config["version"] == version, "Unstamped build checkout")
    directory = root / "src-tauri/target" / PLATFORMS[platform][0] / "release"
    if platform.startswith("macos-"):
        app = one(directory / "bundle/macos", "*.app")
        with (app / "Contents/Info.plist").open("rb") as stream:
            info = plistlib.load(stream)
        require(
            info["CFBundleShortVersionString"] == version,
            "macOS embedded version mismatch",
        )
        require(app.name == "LumaGate.app", "macOS app is not LumaGate-branded")
        executable = app / "Contents/MacOS" / info["CFBundleExecutable"]
        arch = "arm64" if platform == "macos-arm64" else "x86_64"
        require(
            run("lipo", "-archs", str(executable)) == arch,
            "macOS binary architecture mismatch",
        )
        run("codesign", "--verify", "--deep", "--strict", str(app))
        # This is structural signature verification, NOT Gatekeeper/notarization approval.
        dmg = one(directory / "bundle/dmg", "*.dmg")
        run("hdiutil", "verify", str(dmg))
    elif platform in WINDOWS_MACHINES:
        executable = one(directory, "*.exe")
        installer = one(directory / "bundle/nsis", "*.exe")
        write_json(
            directory / "windows-payload.json",
            windows_payload(installer, executable, platform, version),
        )
    else:
        deb = one(directory / "bundle/deb", "*.deb")
        require(
            run("dpkg-deb", "--field", str(deb), "Version") == version,
            "Debian embedded version mismatch",
        )
        require(
            run("dpkg-deb", "--field", str(deb), "Architecture") == "amd64",
            "Debian architecture mismatch",
        )
        appimage = one(directory / "bundle/appimage", "*.AppImage")
        with appimage.open("rb") as stream:
            header = stream.read(20)
        require(
            header[:6] == b"\x7fELF\x02\x01" and header[18:20] == b"\x3e\x00",
            "AppImage is not x64 ELF",
        )
    print(f"Verified native bundle metadata: {platform} {version}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", choices=PLATFORMS, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--root", type=Path, default=Path.cwd())
    args = parser.parse_args()
    verify(args.root, args.platform, args.version)
