#!/usr/bin/env python3
"""Bridge cargo-xwin's clang-cl include flags for ring's GNU clang ARM64 assembler.

ring selects the `clang` driver for Windows ARM64 .S files while inheriting /imsvc
flags from the surrounding clang-cl build. Translate only that driver's target
invocations; do not patch ring, disable crypto assembly, or change app code.
"""
import os
import sys

args = sys.argv[1:]
real = os.environ.get("LUMAGATE_REAL_CLANG", "/usr/lib/llvm-19/bin/clang")
if any("aarch64-pc-windows-msvc" in arg for arg in args) and "--driver-mode=cl" not in args:
    translated = []
    index = 0
    while index < len(args):
        arg = args[index]
        if arg == "/imsvc" and index + 1 < len(args):
            translated.extend(["-isystem", args[index + 1]])
            index += 2
        elif arg.startswith("/imsvc"):
            translated.extend(["-isystem", arg[len("/imsvc"):]])
            index += 1
        else:
            translated.append(arg)
            index += 1
    args = translated
os.execv(real, [real, *args])
