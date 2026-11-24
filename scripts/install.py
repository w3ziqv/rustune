#!/usr/bin/env python3
"""Install the pinned Rustune release into the current user's ~/.local/bin."""

import filecmp
import gzip
import hashlib
import io
import os
from pathlib import Path
import platform
import re
import shlex
import signal
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile

VERSION = "0.5.1"
ASSET = f"rustune-v{VERSION}-x86_64-unknown-linux-gnu.tar.gz"
SHA256 = "4dbc08e2ec5c4df84600c75c3682f3589d3d753d8ec76cc2b0c458fb418236ec"
URL = f"https://github.com/w3ziqv/rustune/releases/download/v{VERSION}/{ASSET}"
MIN_GLIBC = (2, 39)
PROGRAMS = ("rustune", "rustune-audio")
MANAGED = ".rustune-managed"
LAYOUT = "rustune managed pair v1\n"
MAX_ARCHIVE = 32 * 1024 * 1024
MAX_TAR = 64 * 1024 * 1024


class InstallError(Exception):
    pass


def check_platform():
    if sys.platform != "linux" or platform.machine() != "x86_64":
        raise InstallError("The prebuilt release requires x86_64 GNU/Linux. See docs/installation.md.")
    try:
        libc = os.confstr("CS_GNU_LIBC_VERSION")
    except (OSError, ValueError):
        libc = None
    match = re.fullmatch(r"glibc (\d+)\.(\d+)", libc or "")
    if not match or tuple(map(int, match.groups())) < MIN_GLIBC:
        minimum = ".".join(map(str, MIN_GLIBC))
        raise InstallError(f"The prebuilt release requires glibc {minimum} or newer. See docs/installation.md.")


def download(archive):
    subprocess.run(
        ["curl", "--disable", "--fail", "--silent", "--show-error", "--location",
         "--proto", "=https", "--proto-redir", "=https", "--connect-timeout", "15",
         "--max-time", "120", "--max-filesize", str(MAX_ARCHIVE), "--output",
         str(archive), URL],
        check=True,
    )
    if archive.stat().st_size > MAX_ARCHIVE:
        raise InstallError("Release archive exceeds its size limit.")
    with archive.open("rb") as stream:
        digest = hashlib.sha256()
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    if digest.hexdigest() != SHA256:
        raise InstallError("Release archive SHA-256 mismatch.")


def unpack(archive, pair):
    with gzip.open(archive, "rb") as stream:
        content = stream.read(MAX_TAR + 1)
    if len(content) > MAX_TAR:
        raise InstallError("Expanded release archive exceeds its size limit.")
    found = set()
    with tarfile.open(fileobj=io.BytesIO(content), mode="r:") as members:
        for member in members:
            name = member.name
            if (name not in PROGRAMS or name in found or member.type != tarfile.REGTYPE
                    or member.sparse is not None or member.pax_headers or member.size <= 0
                    or member.offset_data - member.offset != tarfile.BLOCKSIZE):
                raise InstallError("Unexpected entry in release archive.")
            with members.extractfile(member) as source, (pair / name).open("xb") as target:
                shutil.copyfileobj(source, target)
            (pair / name).chmod(0o755)
            found.add(name)
    if found != set(PROGRAMS):
        raise InstallError("Release archive does not contain both executables.")


def check_pair(pair):
    for name in PROGRAMS:
        try:
            result = subprocess.run(
                [pair / name, "--version"], check=True, capture_output=True, text=True, timeout=10
            )
        except (OSError, subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
            raise InstallError(
                f"Cannot run {name}. Check the shared libraries in docs/installation.md."
            ) from error
        if result.stdout.strip() != f"{name} {VERSION}":
            raise InstallError(f"{name} does not report the expected version {VERSION}.")


def ensure_inactive():
    uid = os.getuid()
    for process in Path("/proc").iterdir():
        if not process.name.isdecimal():
            continue
        try:
            if process.stat().st_uid != uid:
                continue
            name = (process / "comm").read_text().strip()
            if name not in PROGRAMS:
                exe = os.readlink(process / "exe").removesuffix(" (deleted)")
                if Path(exe).name not in PROGRAMS:
                    continue
        except (FileNotFoundError, ProcessLookupError, PermissionError, UnicodeError):
            continue
        raise InstallError(
            "Rustune is running. Close its TUI, run `rustune ctl shutdown`, "
            "and stop its systemd user unit if you use one. Then retry."
        )


def owned_directory(path):
    info = path.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o022:
        raise InstallError(f"Expected a user-owned directory protected from other writers: {path}")


def managed_target(root):
    owned_directory(root)
    marker = root / "layout"
    if marker.is_symlink() or not marker.is_file() or marker.read_text() != LAYOUT:
        raise InstallError(f"Move the unrecognized path aside before installing: {root}")
    releases = root / "releases"
    owned_directory(releases)
    pointer = root / "current"
    if not os.path.lexists(pointer):
        return None
    if not pointer.is_symlink():
        raise InstallError(f"Move the unrecognized path aside before installing: {pointer}")
    target = os.readlink(pointer)
    if not re.fullmatch(r"releases/v[0-9]+\.[0-9]+\.[0-9]+-[a-z0-9_]+", target):
        raise InstallError(f"Unrecognized Rustune release pointer: {pointer}")
    previous = root / target
    owned_directory(previous)
    for name in PROGRAMS:
        entry = previous / name
        if entry.is_symlink() or not entry.is_file():
            raise InstallError(f"Incomplete managed Rustune release: {previous}")
    return previous


def existing_links(destination, managed):
    missing = []
    for name in PROGRAMS:
        path = destination / name
        target = f"{MANAGED}/current/{name}"
        if not os.path.lexists(path):
            missing.append((path, target))
        elif not managed or not path.is_symlink() or os.readlink(path) != target:
            raise InstallError(
                f"Move the existing {path} aside along with its companion before installing. "
                "See the manual-install migration in docs/installation.md."
            )
    return missing


def install(pair, destination):
    import fcntl

    destination.mkdir(exist_ok=True, parents=True)
    destination = destination.resolve(strict=True)
    owned_directory(destination)
    lock = os.open(destination, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        fcntl.flock(lock, fcntl.LOCK_EX)
        install_locked(pair, destination)
    finally:
        os.close(lock)


def install_locked(pair, destination):
    root = destination / MANAGED
    managed = os.path.lexists(root)
    previous = managed_target(root) if managed else None
    missing = existing_links(destination, managed)
    ensure_inactive()
    if previous is not None and not missing and all(
        os.access(previous / name, os.X_OK)
        and filecmp.cmp(pair / name, previous / name, shallow=False) for name in PROGRAMS
    ):
        check_pair(previous)
        return

    with tempfile.TemporaryDirectory(prefix=".rustune-stage-", dir=destination) as work:
        staging = Path(work)
        candidate = Path(tempfile.mkdtemp(prefix=f"v{VERSION}-", dir=staging))
        for name in PROGRAMS:
            shutil.copyfile(pair / name, candidate / name)
            (candidate / name).chmod(0o755)
        # Test from the installation filesystem, even when TMPDIR is noexec.
        check_pair(candidate)

        if not managed:
            layout = staging / "layout"
            layout.mkdir(mode=0o700)
            (layout / "releases").mkdir(mode=0o700)
            (layout / "layout").write_text(LAYOUT)
            layout.rename(root)

        release = root / "releases" / candidate.name
        if os.path.lexists(release):
            raise InstallError(f"Unexpected existing release directory: {release}")
        target = str(release.relative_to(root))
        pointer = staging / "current"
        pointer.symlink_to(target)
        ensure_inactive()
        # A normal interrupt must not break cleanup between link creation and commit.
        mask = signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGINT, signal.SIGTERM})
        links_added = []
        active = root / "current"
        try:
            candidate.rename(release)
            for path, link_target in missing:
                path.symlink_to(link_target)
                links_added.append(path)
            pointer.replace(active)
        finally:
            try:
                committed = active.is_symlink() and os.readlink(active) == target
                if not committed:
                    for path in links_added:
                        path.unlink()
                    if release.exists():
                        shutil.rmtree(release)
            finally:
                signal.pthread_sigmask(signal.SIG_SETMASK, mask)


def main():
    try:
        check_platform()
        destination = Path.home() / ".local" / "bin"
        ensure_inactive()
        if destination.is_dir():
            root = destination / MANAGED
            managed = os.path.lexists(root)
            if managed:
                managed_target(root)
            existing_links(destination, managed)
        with tempfile.TemporaryDirectory(prefix="rustune-install-") as work:
            archive = Path(work) / ASSET
            pair = Path(work) / "pair"
            pair.mkdir(mode=0o700)
            download(archive)
            unpack(archive, pair)
            install(pair, destination)
    except subprocess.CalledProcessError:
        print("Rustune installation failed: could not download the pinned release.", file=sys.stderr)
        return 1
    except (InstallError, OSError, ValueError, EOFError, tarfile.TarError) as error:
        print(f"Rustune installation failed: {error}", file=sys.stderr)
        return 1

    print(f"Rustune {VERSION} is installed in {destination}.")
    print(f"Next, run {shlex.quote(str(destination / 'rustune'))} doctor.")
    if shutil.which("rustune") != str(destination / "rustune"):
        print(f"Put {destination} first on PATH to use the short `rustune` command.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
