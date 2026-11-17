#!/usr/bin/env python3
import re
import shlex
import shutil
import struct
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
VENDOR = ROOT / "vendor" / "ffmpeg"

FORBIDDEN_FLAGS = (
    "--enable-gpl",
    "--enable-nonfree",
)

FORBIDDEN_LIBRARIES = (
    "libx264",
    "libx265",
    "libxvid",
    "libvidstab",
    "librubberband",
    "frei0r",
)

FLAGS_NEEDING_A_DECISION = ("--enable-version3",)


def baked_configure_line(prefix: Path) -> str | None:
    for candidate in sorted(prefix.glob("lib/libavutil*.dylib")) + sorted(
        prefix.glob("lib/libavutil.so*")
    ) + sorted(prefix.glob("bin/avutil*.dll")):
        blob = candidate.read_bytes()
        match = re.search(rb"--prefix=[^\x00]{0,4000}", blob)
        if match:
            return match.group(0).decode("utf-8", "replace")
    return None


def recorded_configure_line(prefix: Path) -> str | None:
    info = prefix / "BUILD_INFO.txt"
    if not info.exists():
        return None
    for line in info.read_text().splitlines():
        if line.startswith("configure="):
            return line.removeprefix("configure=")
    return None


def flags(line: str) -> list[str]:
    try:
        return shlex.split(line)
    except ValueError:
        return line.split()


ELF_SYSTEM_LIBRARIES = (
    "libc",
    "libm",
    "libpthread",
    "libdl",
    "librt",
    "libz",
    "libva",
    "libva-drm",
    "libdrm",
    "libatomic",
    "ld-linux-x86-64",
    "ld-linux-aarch64",
)
FFMPEG_LIBRARIES = (
    "libavcodec",
    "libavdevice",
    "libavfilter",
    "libavformat",
    "libavutil",
    "libswresample",
    "libswscale",
)

DT_NEEDED = 1
DT_RPATH = 15
DT_RUNPATH = 29
SHT_DYNAMIC = 6


def elf_dynamic(blob: bytes) -> dict[int, list[str]] | None:
    if blob[:4] != b"\x7fELF" or blob[4] not in (1, 2) or blob[5] not in (1, 2):
        return None
    wide = blob[4] == 2
    endian = "<" if blob[5] == 1 else ">"
    if wide:
        shoff, = struct.unpack_from(endian + "Q", blob, 0x28)
        shentsize, shnum = struct.unpack_from(endian + "HH", blob, 0x3A)
        header = endian + "IIQQQQIIQQ"
        entry = endian + "qQ"
    else:
        shoff, = struct.unpack_from(endian + "I", blob, 0x20)
        shentsize, shnum = struct.unpack_from(endian + "HH", blob, 0x2E)
        header = endian + "IIIIIIIIII"
        entry = endian + "iI"
    sections = [
        struct.unpack_from(header, blob, shoff + index * shentsize) for index in range(shnum)
    ]
    for section in sections:
        if section[1] != SHT_DYNAMIC:
            continue
        offset, size, link = section[4], section[5], section[6]
        strings = sections[link][4]
        found: dict[int, list[str]] = {}
        step = struct.calcsize(entry)
        for position in range(offset, offset + size, step):
            tag, value = struct.unpack_from(entry, blob, position)
            if tag == 0:
                break
            if tag in (DT_NEEDED, DT_RPATH, DT_RUNPATH):
                start = strings + value
                end = blob.index(b"\x00", start)
                found.setdefault(tag, []).append(blob[start:end].decode("utf-8", "replace"))
        return found
    return None


def library_stem(soname: str) -> str:
    return soname.split(".so", 1)[0]


def elf_problems(prefix: Path) -> list[str]:
    problems = []
    for library in sorted(prefix.glob("lib/lib*.so.*")):
        if library.is_symlink():
            continue
        dynamic = elf_dynamic(library.read_bytes())
        if dynamic is None:
            problems.append(f"{library.name}: no ELF dynamic section; not a shared object of this prefix")
            continue
        for tag in (DT_RPATH, DT_RUNPATH):
            for value in dynamic.get(tag, []):
                for entry in value.split(":"):
                    if entry and not entry.startswith(("$ORIGIN", "${ORIGIN}")):
                        kind = "RPATH" if tag == DT_RPATH else "RUNPATH"
                        problems.append(f"{library.name}: {kind} {entry} points into the build machine")
        for needed in dynamic.get(DT_NEEDED, []):
            stem = library_stem(needed)
            if stem in FFMPEG_LIBRARIES or stem in ELF_SYSTEM_LIBRARIES:
                continue
            problems.append(f"{library.name}: loads {needed}, which a user's Linux does not have")
    return problems


PE_SYSTEM_LIBRARIES = (
    "advapi32",
    "bcrypt",
    "crypt32",
    "d3d11",
    "dxgi",
    "gdi32",
    "kernel32",
    "kernelbase",
    "mf",
    "mfplat",
    "mfreadwrite",
    "msvcrt",
    "ncrypt",
    "ntdll",
    "ole32",
    "oleaut32",
    "psapi",
    "secur32",
    "shell32",
    "shlwapi",
    "ucrtbase",
    "user32",
    "userenv",
    "ws2_32",
)
PE_FFMPEG_DLL = re.compile(
    r"^(avcodec|avdevice|avfilter|avformat|avutil|swresample|swscale)-[0-9]+\.dll$"
)
IMAGE_DIRECTORY_ENTRY_IMPORT = 1


def pe_imports(blob: bytes) -> list[str] | None:
    if len(blob) < 0x40 or blob[:2] != b"MZ":
        return None
    (pe_offset,) = struct.unpack_from("<I", blob, 0x3C)
    if pe_offset + 24 > len(blob) or blob[pe_offset : pe_offset + 4] != b"PE\x00\x00":
        return None
    coff = pe_offset + 4
    (sections_count,) = struct.unpack_from("<H", blob, coff + 2)
    (optional_size,) = struct.unpack_from("<H", blob, coff + 16)
    optional = coff + 20
    if optional + optional_size > len(blob) or optional_size < 2:
        return None
    (magic,) = struct.unpack_from("<H", blob, optional)
    if magic == 0x10B:
        count_at, directories_at = 92, 96
    elif magic == 0x20B:
        count_at, directories_at = 108, 112
    else:
        return None
    if directories_at > optional_size:
        return None
    (directories_count,) = struct.unpack_from("<I", blob, optional + count_at)
    if directories_count <= IMAGE_DIRECTORY_ENTRY_IMPORT:
        return []
    entry = optional + directories_at + 8 * IMAGE_DIRECTORY_ENTRY_IMPORT
    if entry + 8 > optional + optional_size:
        return None
    import_rva, import_size = struct.unpack_from("<II", blob, entry)
    if import_rva == 0 or import_size == 0:
        return []

    sections = []
    table = optional + optional_size
    for index in range(sections_count):
        at = table + 40 * index
        if at + 40 > len(blob):
            return None
        virtual_size, virtual_address, raw_size, raw_pointer = struct.unpack_from(
            "<IIII", blob, at + 8
        )
        sections.append((virtual_address, max(virtual_size, raw_size), raw_pointer))

    def offset_of(rva: int) -> int | None:
        for virtual_address, size, raw_pointer in sections:
            if virtual_address <= rva < virtual_address + size:
                return rva - virtual_address + raw_pointer
        return None

    descriptor = offset_of(import_rva)
    if descriptor is None:
        return None
    names = []
    while descriptor + 20 <= len(blob):
        fields = struct.unpack_from("<IIIII", blob, descriptor)
        if not any(fields):
            return names
        name_offset = offset_of(fields[3])
        if name_offset is None:
            return None
        end = blob.find(b"\x00", name_offset)
        if end < 0:
            return None
        names.append(blob[name_offset:end].decode("ascii", "replace"))
        descriptor += 20
    return None


def pe_problems(prefix: Path) -> list[str]:
    problems = []
    for library in sorted(prefix.glob("bin/*.dll")):
        imports = pe_imports(library.read_bytes())
        if imports is None:
            problems.append(f"{library.name}: no PE import table; not a DLL of this prefix")
            continue
        for name in imports:
            lowered = name.lower()
            if PE_FFMPEG_DLL.match(lowered) or lowered.startswith("api-ms-win-"):
                continue
            if lowered.removesuffix(".dll") in PE_SYSTEM_LIBRARIES:
                continue
            problems.append(f"{library.name}: imports {name}, which a user's Windows does not have")
        match = PE_FFMPEG_DLL.match(library.name.lower())
        if match and not (prefix / "lib" / f"{match.group(1)}.lib").exists():
            problems.append(
                f"{library.name}: no lib/{match.group(1)}.lib import library; "
                "rebuild with scripts/build-ffmpeg-lgpl.sh"
            )
    return problems


def portability_problems(prefix: Path) -> list[str]:
    problems = []
    for pc in sorted(prefix.glob("lib/pkgconfig/*.pc")):
        for line in pc.read_text().splitlines():
            if line.startswith("prefix=") and "${pcfiledir}" not in line:
                problems.append(f"{pc}: absolute {line}; rebuild with scripts/build-ffmpeg-lgpl.sh")
                break
    if sys.platform == "darwin" and shutil.which("otool"):
        for dylib in sorted(prefix.glob("lib/lib*.*.dylib")):
            if dylib.is_symlink():
                continue
            out = subprocess.run(
                ["otool", "-L", str(dylib)], capture_output=True, text=True, check=False
            ).stdout
            for entry in out.splitlines()[1:]:
                name = entry.strip().split(" (", 1)[0]
                if name.startswith(("/usr/lib/", "/System/", "@rpath/")):
                    continue
                problems.append(f"{dylib.name}: loads {name}, which a user's Mac does not have")
    problems.extend(elf_problems(prefix))
    problems.extend(pe_problems(prefix))
    return problems


def check(prefix: Path) -> list[str]:
    line = baked_configure_line(prefix)
    if line is None:
        return [f"{prefix}: no configure string in libavutil; the licence cannot be derived"]

    problems = []

    recorded = recorded_configure_line(prefix)
    if recorded is not None and flags(recorded) != flags(line):
        problems.append(
            f"{prefix}: BUILD_INFO.txt configure line disagrees with libavutil; "
            "the prefix was not rebuilt after the script changed"
        )

    for flag in FORBIDDEN_FLAGS:
        if flag in line:
            problems.append(f"{prefix}: {flag} relicenses the product under the GPL")

    for library in FORBIDDEN_LIBRARIES:
        if library in line:
            problems.append(f"{prefix}: links {library}, which is GPL")

    for flag in FLAGS_NEEDING_A_DECISION:
        if flag in line:
            problems.append(
                f"{prefix}: {flag} makes this LGPLv3, not LGPLv2.1 — a licensing decision"
            )

    if "--disable-shared" in line or "--enable-static" in line:
        problems.append(f"{prefix}: static linking removes the LGPL substitution right")

    problems.extend(portability_problems(prefix))
    return problems


def main() -> int:
    prefixes = (
        [Path(argument) for argument in sys.argv[1:]]
        if len(sys.argv) > 1
        else sorted(p for p in VENDOR.glob("*-*") if p.is_dir())
    )

    if not prefixes:
        print("ffmpeg-license: no prefix built; nothing to check")
        print("  build one with ./scripts/build-ffmpeg-lgpl.sh")
        return 0

    problems = []
    for prefix in prefixes:
        problems.extend(check(prefix))

    if problems:
        print(f"ffmpeg-license: {len(problems)} problem(s)\n")
        for problem in problems:
            print(f"  {problem}")
        return 1

    for prefix in prefixes:
        print(f"ffmpeg-license: {prefix.name} is LGPL-2.1, dynamically linked")
    return 0


if __name__ == "__main__":
    sys.exit(main())
