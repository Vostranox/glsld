#!/usr/bin/env python3
"""Preserve license texts for locked Rust dependencies and the native compiler.

SPDX-License-Identifier: MIT
"""

import argparse
import json
from pathlib import Path
import re
import subprocess

GLSLANG_SYS_VERSION = "0.9.2+e1b562a"
GLSLANG_SYS_COMMIT = "f8ed92af267f2756f5f8fd388c995e20baa48882"
MIT_EXPRESSIONS = {
    "MIT",
    "MIT OR Apache-2.0",
    "Apache-2.0 OR MIT",
    "MIT/Apache-2.0",
    "Unlicense OR MIT",
    "(MIT OR Apache-2.0) AND Unicode-3.0",
    "Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT",
}

def native_notices(source):
    """Retain complete native licenses and distinct copyright/license headers."""
    native = source / "native/glslang"
    license_text = (native / "LICENSE.txt").read_text(encoding="utf-8")
    sections = [f"--- native/glslang/LICENSE.txt ---\n{license_text}"]
    headers = {}
    directories = (native / "glslang", native / "SPIRV", source / "native/build_info")
    for directory in directories:
        for path in sorted(directory.rglob("*")):
            if not path.is_file() or path.suffix not in {".h", ".hpp", ".c", ".cpp"}:
                continue
            contents = path.read_text(encoding="utf-8-sig")
            header_match = re.match(r"\s*(?:(?://[^\n]*\n|/\*[\s\S]*?\*/)\s*)*", contents)
            header = header_match[0].strip()
            if re.search(r"copyright|SPDX-License-Identifier", header, re.IGNORECASE):
                headers.setdefault(header, []).append(str(path.relative_to(source)))

    has_bison_exception = any("special exception" in header for header in headers)
    if not headers or not has_bison_exception:
        raise SystemExit("Native compiler notices changed; review the pinned build inputs")
    for header, paths in headers.items():
        source_files = ", ".join(paths)
        sections.append(f"--- {source_files} ---\n{header}")
    return "\n\n".join(sections) + "\n"

def package_notice(package, root):
    expression = package["license"] or "UNDECLARED"
    if expression not in MIT_EXPRESSIONS:
        raise SystemExit(f"Review new dependency license: {package['name']} {expression}")

    source = Path(package["manifest_path"]).parent
    notice_names = {"license-mit", "license-unicode", "copyright", "notice", "notice.txt"}
    if expression == "MIT":
        notice_names.update({"license", "license.txt"})
    sections = []
    for path in sorted(source.iterdir()):
        if path.is_file() and path.name.casefold() in notice_names:
            contents = path.read_text(encoding="utf-8")
            sections.append(f"--- {path.name} ---\n{contents}")

    if package["name"] == "glslang-sys":
        vcs_info = json.loads((source / ".cargo_vcs_info.json").read_text())
        commit = vcs_info["git"]["sha1"]
        if package["version"] != GLSLANG_SYS_VERSION or commit != GLSLANG_SYS_COMMIT:
            raise SystemExit("Review glslang-sys native inputs and refresh its pinned license")
        license_file = root / "scripts/licenses/glslang-rs-MIT.txt"
        license_text = license_file.read_text(encoding="utf-8")
        sections.append(f"--- upstream LICENSE.md (UTF-8 transcription) ---\n{license_text}")
        sections.append(native_notices(source))

    if not sections:
        raise SystemExit(f"No license notice found for {package['name']}")
    heading = (
        f"{package['name']} {package['version']}\n"
        f"Declared crate license: {expression}\n\n"
    )
    return heading + "\n".join(sections)

def collect_notices(root, target):
    command = [
        "cargo", "metadata", "--locked", "--offline", "--format-version", "1",
        "--filter-platform", target,
    ]
    metadata = json.loads(subprocess.check_output(command, cwd=root))
    reachable = {node["id"] for node in metadata["resolve"]["nodes"]}
    packages = sorted(
        metadata["packages"], key=lambda package: (package["name"], package["version"])
    )
    generated = {}
    rows = []
    for package in packages:
        if not package["source"] or package["id"] not in reachable:
            continue
        filename = f"{package['name']}-{package['version']}.txt"
        generated[filename] = package_notice(package, root)
        rows.append(
            f"| {package['name']} | {package['version']} | {package['license']} "
            f"| [Notices](third-party/{filename}) |"
        )
    return generated, rows

def notice_index(target, rows):
    header = f"""# Third-party notices

Generated from the locked Cargo graph for `{target}`, including build
packages. Original glsld code is MIT. Dependencies retain their own copyrights
and terms; the MIT option is selected where offered. `unicode-ident` also
requires its Unicode-3.0 notice.

## Statically linked compiler

`glslang-sys` includes Khronos glslang, whose native source has additional BSD,
MIT, Apache, NVIDIA and other notices. Its generated parser contains the GNU
Bison skeleton under GPL-3.0-or-later **with the Bison exception**, which permits
larger works under other terms. glsld is not a parser generator and does not
modify that skeleton.
The complete upstream license collection and source copyright/license headers
are retained in the glslang-sys notice below. Upstream update scripts and test
programs are not compiled or packaged with glsld.

The binding's missing registry license is preserved from commit
`{GLSLANG_SYS_COMMIT}`.
The Rust standard library and platform runtimes retain their own licenses.
The VS Code JavaScript bundle has a separate `THIRD_PARTY_LICENSES.txt`, generated
from its bundled runtime packages and included alongside these native notices.

## Rust packages

| Package | Version | Declared crate license | Retained text |
| --- | --- | --- | --- |
"""
    return header + "\n".join(rows) + "\n"

def write_notices(output, target, generated, rows):
    notices = output / "third-party"
    notices.mkdir(parents=True, exist_ok=True)
    for filename, contents in generated.items():
        (notices / filename).write_text(contents, encoding="utf-8")
    for path in notices.glob("*.txt"):
        if path.name not in generated:
            path.unlink()
    index = notice_index(target, rows)
    (output / "THIRD_PARTY_NOTICES.md").write_text(index, encoding="utf-8")

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", default="x86_64-unknown-linux-gnu")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    generated, rows = collect_notices(root, args.target)
    output = root / "target/notices" / args.target
    write_notices(output, args.target, generated, rows)
    print(f"Recorded {len(rows)} dependency notices in {output}")

if __name__ == "__main__":
    main()
