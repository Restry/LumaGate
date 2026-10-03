#!/usr/bin/env python3
"""Static release checks; this does NOT substitute for a Windows runtime test.

Requires pefile and an NSIS payload extracted with 7z. Checks the actual payload,
not the x86 installer stub, when determining the application's architecture.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import pefile


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def icon_images(path):
    data = path.read_bytes()
    reserved, kind, count = struct.unpack_from("<HHH", data)
    assert (reserved, kind) == (0, 1), "Invalid source ICO"
    result = set()
    for i in range(count):
        size, offset = struct.unpack_from("<II", data, 6 + i * 16 + 8)
        result.add(data[offset:offset + size])
    return result


def inspect(path, expected_machine, icons):
    pe = pefile.PE(str(path))
    assert pe.FILE_HEADER.Machine == expected_machine, "Wrong PE architecture"
    assert pe.OPTIONAL_HEADER.Subsystem == 2, "Not a Windows GUI executable"
    resources = {}
    for kind in pe.DIRECTORY_ENTRY_RESOURCE.entries:
        for name in kind.directory.entries:
            for lang in name.directory.entries:
                info = lang.data.struct
                resources[(kind.id, name.id)] = pe.get_data(info.OffsetToData, info.Size)
    groups = []
    for (kind, name), data in resources.items():
        if kind != 14:
            continue
        count = struct.unpack_from("<H", data, 4)[0]
        ids = [struct.unpack_from("<H", data, 6 + i * 14 + 12)[0] for i in range(count)]
        groups.append({"id": name, "images": count,
                       "lumagate": bool(ids) and all(resources.get((3, i)) in icons for i in ids)})
    assert any(group["lumagate"] for group in groups), "LumaGate icon not found"
    version = {}
    for entries in getattr(pe, "FileInfo", []):
        for entry in entries:
            for table in getattr(entry, "StringTable", []):
                version.update({k.decode(): v.decode() for k, v in table.entries.items()})
    assert version.get("ProductName") == "LumaGate", version
    assert version.get("ProductVersion", "").startswith("3.23.1"), version
    imports = sorted({entry.dll.decode().lower() for entry in getattr(pe, "DIRECTORY_ENTRY_IMPORT", [])})
    dynamic_crt = [name for name in imports if name.startswith(("vcruntime", "msvcp", "ucrtbase", "api-ms-win-crt"))]
    assert not dynamic_crt, f"Unexpected dynamic C runtime: {dynamic_crt}"
    return {"file": path.name, "bytes": path.stat().st_size, "sha256": sha(path),
            "machine": hex(pe.FILE_HEADER.Machine), "subsystem": "Windows GUI",
            "version": version, "iconGroups": groups, "imports": imports,
            "signed": bool(pe.OPTIONAL_HEADER.DATA_DIRECTORY[4].Size)}


def main():
    parser = argparse.ArgumentParser()
    for arg in ("installer", "payload", "original", "icon", "report"):
        parser.add_argument("--" + arg, type=Path, required=True)
    parser.add_argument("--arch", choices=["x64", "arm64"], required=True)
    args = parser.parse_args()
    source = args.original.read_bytes()
    marker = b"TAURI_BUNDLE_TYPE_VAR_UNK"
    assert source.count(marker) == 1, "Missing/ambiguous Tauri bundle marker"
    expected = source.replace(marker, b"TAURI_BUNDLE_TYPE_VAR_NSS")
    assert args.payload.read_bytes() == expected, "Installer payload differs beyond Tauri's documented bundle marker"
    icons = icon_images(args.icon)
    result = {"version": "3.23.1", "architecture": args.arch,
              "installer": inspect(args.installer, 0x14c, icons),
              "application": inspect(args.payload, 0x8664 if args.arch == "x64" else 0xaa64, icons),
              "sourceIconSha256": sha(args.icon), "payloadMatchesCompiledApplication": True,
              "bundleMarker": "UNK → NSS (Tauri NSIS packaging)",
              "windowsRuntimeTested": False}
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"architecture": args.arch, "passed": True,
                      "installerSha256": result["installer"]["sha256"],
                      "windowsRuntimeTested": False}))


if __name__ == "__main__":
    main()
