#!/usr/bin/env python3
"""Check metadata, package native releases, and hash all six platform artifacts.

Packaging generates dependency notices for the selected target.
SPDX-License-Identifier: MIT
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parent.parent
EXTENSION = ROOT / "editors/vscode"
TARGETS = {
    "x86_64-unknown-linux-gnu": "linux-x64",
    "aarch64-unknown-linux-gnu": "linux-arm64",
    "x86_64-apple-darwin": "darwin-x64",
    "aarch64-apple-darwin": "darwin-arm64",
    "x86_64-pc-windows-msvc": "win32-x64",
    "aarch64-pc-windows-msvc": "win32-arm64",
}

def check_marketplace_identity(extension):
    publisher = extension.get("publisher", "")
    repository = extension.get("repository", {})
    if not isinstance(repository, str):
        repository = repository.get("url", "")

    identity = f"{publisher} {repository}".lower()
    has_placeholder = any(
        marker in identity
        for marker in ("replace-me", "replace_me", "example.com")
    )
    if not publisher or not repository or has_placeholder:
        raise SystemExit(
            "Replace the temporary publisher and repository in "
            "editors/vscode/package.json before publishing"
        )

def check(tag="", marketplace=False) -> str:
    """Require matching package versions before building release artifacts."""
    cargo = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["package"]
    cargo_lock = tomllib.loads((ROOT / "Cargo.lock").read_text(encoding="utf-8"))
    extension = json.loads((EXTENSION / "package.json").read_text(encoding="utf-8"))
    npm_lock = json.loads((EXTENSION / "package-lock.json").read_text(encoding="utf-8"))

    version = cargo["version"]
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version):
        raise SystemExit("Release versions must use major.minor.patch for VS Code Marketplace")

    locked_version = next(
        (
            package["version"]
            for package in cargo_lock["package"]
            if package["name"] == cargo["name"] and "source" not in package
        ),
        None,
    )
    versions = [
        locked_version,
        extension["version"],
        npm_lock["version"],
        npm_lock["packages"][""]["version"],
    ]
    if any(candidate != version for candidate in versions):
        raise SystemExit(
            "Cargo.toml, Cargo.lock, extension package.json and "
            "package-lock.json versions must match"
        )
    if tag and tag != f"v{version}":
        raise SystemExit(f"Tag {tag!r} does not match package version v{version}")
    if marketplace:
        check_marketplace_identity(extension)
    return version

def generate_notices(target: str) -> Path:
    command = [
        sys.executable,
        str(ROOT / "scripts/dependency-notices.py"),
        "--target",
        target,
    ]
    subprocess.run(command, check=True)
    return ROOT / "target/notices" / target

def copy_executable(source: Path, destination: Path):
    shutil.copy2(source, destination)
    destination.chmod(0o755)

def copy_notices(source: Path, destination: Path):
    shutil.copy2(source / "THIRD_PARTY_NOTICES.md", destination / "THIRD_PARTY_NOTICES.md")
    licenses = destination / "third-party"
    if licenses.exists():
        shutil.rmtree(licenses)
    shutil.copytree(source / "third-party", licenses)

def create_archive(directory: Path, output: Path, target: str) -> Path:
    if "windows" in target:
        archive = output / f"{directory.name}.zip"
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as destination:
            for source in sorted(directory.rglob("*")):
                if source.is_file():
                    destination.write(source, source.relative_to(directory.parent))
    else:
        archive = output / f"{directory.name}.tar.gz"
        with tarfile.open(archive, "w:gz") as destination:
            destination.add(directory, arcname=directory.name)
    return archive

def stage_extension(binary: Path, executable: str, notices: Path):
    binaries = EXTENSION / "bin"
    binaries.mkdir(exist_ok=True)
    for name in ("glsld", "glsld.exe"):
        (binaries / name).unlink(missing_ok=True)

    copy_executable(binary, binaries / executable)
    shutil.copy2(ROOT / "LICENSE", EXTENSION / "LICENSE")
    copy_notices(notices, EXTENSION)

def package(args):
    version = check()
    executable = "glsld.exe" if "windows" in args.target else "glsld"
    default_binary = ROOT / "target" / args.target / "release" / executable
    binary = (args.binary or default_binary).resolve()
    reported = subprocess.check_output([str(binary), "--version"], text=True).strip()
    if reported != f"glsld {version}":
        raise SystemExit(f"Unexpected binary version: {reported!r}")

    notices = generate_notices(args.target)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(prefix="glsld-package-") as temporary:
        directory = Path(temporary) / f"glsld-v{version}-{args.target}"
        directory.mkdir()
        copy_executable(binary, directory / executable)
        for filename in ("LICENSE", "README.md"):
            shutil.copy2(ROOT / filename, directory / filename)
        copy_notices(notices, directory)
        shutil.copytree(ROOT / "examples", directory / "examples")
        archive = create_archive(directory, output, args.target)

    stage_extension(binary, executable, notices)
    print(f"Packaged {archive}")
    print(f"Staged editors/vscode for --target {TARGETS[args.target]}")

def sha256(path: Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()

def checksums(directory: Path):
    version = check()
    names = []
    for target, vsix_target in TARGETS.items():
        suffix = "zip" if "windows" in target else "tar.gz"
        names.append(f"glsld-v{version}-{target}.{suffix}")
        names.append(f"glsld-v{version}-{vsix_target}.vsix")

    lines = []
    for name in sorted(names):
        path = directory / name
        if not path.is_file():
            raise SystemExit(f"Missing release artifact: {name}")
        lines.append(f"{sha256(path)}  {name}\n")

    (directory / "SHA256SUMS").write_text("".join(lines), encoding="utf-8")
    print(f"Recorded checksums for {len(lines)} release artifacts")

def parse_args():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)

    metadata = commands.add_parser(
        "check", help="check versions and optional publishing metadata"
    )
    metadata.add_argument("--tag", default="", help="release tag, for example v0.1.0")
    metadata.add_argument("--marketplace", action="store_true")

    packaging = commands.add_parser(
        "package", help="package a native archive and stage the extension"
    )
    packaging.add_argument("--target", required=True, choices=TARGETS)
    packaging.add_argument(
        "--binary", type=Path, help="defaults to target/TARGET/release/glsld[.exe]"
    )
    packaging.add_argument("--output", type=Path, default=Path("dist"))

    hashing = commands.add_parser(
        "checksums", help="require and hash all six native archives and VSIXs"
    )
    hashing.add_argument("directory", nargs="?", type=Path, default=Path("dist"))
    return parser.parse_args()

def main():
    args = parse_args()
    if args.command == "check":
        print(f"Release metadata is consistent: v{check(args.tag, args.marketplace)}")
    elif args.command == "package":
        package(args)
    else:
        checksums(args.directory)


if __name__ == "__main__":
    main()
