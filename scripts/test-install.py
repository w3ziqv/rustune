#!/usr/bin/env python3
"""Local, network-free integration checks for the installer."""

from contextlib import redirect_stderr, redirect_stdout
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import time
import unittest
from unittest.mock import patch

script = Path(__file__).with_name("install.py")
spec = importlib.util.spec_from_file_location("rustune_installer", script)
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)
release_archive = os.environ.get("RUSTUNE_TEST_RELEASE_ARCHIVE")


class InstallerTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.home = self.root / "home"
        self.bin = self.home / ".local/bin"
        self.archive = self.root / installer.ASSET
        self.current = self.bin / installer.MANAGED / "current"
        self.curl_log = self.root / "curl.log"
        tools = self.root / "tools"
        tools.mkdir()
        curl = tools / "curl"
        curl.write_text(
            "#!/bin/sh\n"
            'printf "%s\\n" "$@" >> "$RUSTUNE_TEST_CURL_LOG"\n'
            '[ -z "$RUSTUNE_TEST_OFFLINE" ] || exit 6\n'
            'while [ "$1" != "--output" ]; do shift; done\n'
            'cp "$RUSTUNE_TEST_ASSET" "$2"\n'
        )
        curl.chmod(0o755)
        self.enterContext(patch.dict(os.environ, {
            "HOME": str(self.home),
            "PATH": f"{tools}:{os.defpath}",
            "TMPDIR": str(self.root),
            "XDG_CONFIG_HOME": str(self.root / "config"),
            "XDG_DATA_HOME": str(self.root / "data"),
            "XDG_CACHE_HOME": str(self.root / "cache"),
            "XDG_RUNTIME_DIR": str(self.root / "runtime"),
            "RUSTUNE_TEST_CURL_LOG": str(self.curl_log),
            "RUSTUNE_TEST_ASSET": str(self.archive),
            "RUSTUNE_TEST_OFFLINE": "",
        }))
        self.enterContext(patch.object(tempfile, "tempdir", str(self.root)))
        self.enterContext(patch.object(installer.os, "confstr", return_value="glibc 2.39"))
        self.fixture()

    def fixture(self, extra=None, version=None, replace=None):
        with tarfile.open(self.archive, "w:gz", format=tarfile.USTAR_FORMAT) as archive:
            for name in installer.PROGRAMS:
                if name == replace:
                    continue
                release = version if name == "rustune-audio" and version else installer.VERSION
                code = f"#!/bin/sh\nprintf '%s\\n' '{name} {release}'\n".encode()
                member = tarfile.TarInfo(name)
                member.size = len(code)
                archive.addfile(member, io.BytesIO(code))
            if extra is not None:
                archive.addfile(extra, io.BytesIO(b"X" * extra.size))

    def run_installer(self, digest=None):
        checksum = hashlib.sha256(self.archive.read_bytes()).hexdigest()
        with (
            patch.object(installer, "SHA256", checksum if digest is None else digest),
            redirect_stdout(io.StringIO()) as output,
            redirect_stderr(io.StringIO()) as error,
        ):
            code = installer.main()
        self.assertEqual(list(self.root.glob("rustune-install-*")), [])
        return code, output.getvalue(), error.getvalue()

    def installed(self):
        return tuple((self.bin / name).read_bytes() for name in installer.PROGRAMS)

    def test_clean_install_preserves_xdg_data_and_shell_files(self):
        files = [
            self.root / "config/rustune/music-user-token",
            self.root / "config/rustune/config.toml",
            self.root / "data/rustune/queue.json",
            self.root / "cache/rustune/drm/libwidevinecdm.so",
            self.home / ".bashrc",
        ]
        for path in files:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("fixture")
        before = [(path.read_bytes(), path.stat()) for path in files]
        code, output, error = self.run_installer()
        self.assertEqual(code, 0, error)
        self.assertIn("PATH", output)
        self.assertIn("doctor", output)
        self.assertEqual([(path.read_bytes(), path.stat()) for path in files],
                         [(data, info) for data, info in before])
        options = self.curl_log.read_text().splitlines()
        self.assertEqual(options[options.index("--proto") + 1], "=https")
        self.assertEqual(options[options.index("--proto-redir") + 1], "=https")
        self.assertEqual(options[-1], installer.URL)
        self.assertEqual((self.bin / "rustune").resolve().parent,
                         (self.bin / "rustune-audio").resolve().parent)

    def test_reinstall_and_managed_upgrade(self):
        with patch.object(installer, "VERSION", "0.5.0"):
            self.fixture()
            code, _, error = self.run_installer()
            self.assertEqual(code, 0, error)
        old = self.current.resolve()
        old_bytes = self.installed()
        self.fixture()
        code, _, error = self.run_installer()
        self.assertEqual(code, 0, error)
        new = self.current.resolve()
        self.assertNotEqual(old, new)
        self.assertNotEqual(self.installed(), old_bytes)
        self.assertEqual(tuple((old / name).read_bytes() for name in installer.PROGRAMS), old_bytes)
        code, _, error = self.run_installer()
        self.assertEqual(code, 0, error)
        self.assertEqual(self.current.resolve(), new)
        self.assertEqual(set(new.parent.iterdir()), {old, new})

    def test_checksum_mismatch(self):
        code, _, error = self.run_installer("0" * 64)
        self.assertEqual(code, 1)
        self.assertIn("SHA-256 mismatch", error)
        self.assertFalse(self.bin.exists())

    def test_no_network_keeps_old_pair(self):
        self.assertEqual(self.run_installer()[0], 0)
        old = self.current.resolve()
        with patch.dict(os.environ, {"RUSTUNE_TEST_OFFLINE": "1"}):
            code, _, error = self.run_installer()
        self.assertEqual(code, 1)
        self.assertIn("could not download", error)
        self.assertEqual(self.current.resolve(), old)

    def test_foreign_file_and_link_remain_untouched(self):
        self.bin.mkdir(parents=True)
        foreign = self.bin / "rustune-audio"
        foreign.write_text("user-owned")
        link = self.bin / "rustune"
        link.symlink_to(f"{installer.MANAGED}/current/rustune")
        code, _, error = self.run_installer()
        self.assertEqual(code, 1)
        self.assertIn("Move the existing", error)
        self.assertTrue(link.is_symlink())
        self.assertEqual(foreign.read_text(), "user-owned")
        self.assertFalse(self.curl_log.exists())

    def test_rejects_hostile_archive_after_checksum(self):
        for name, kind in (("../escape", tarfile.REGTYPE), ("rustune", tarfile.SYMTYPE),
                           ("rustune", tarfile.LNKTYPE), ("rustune", tarfile.REGTYPE)):
            with self.subTest(name=name, kind=kind):
                member = tarfile.TarInfo(name)
                member.type = kind
                member.linkname = "/tmp/elsewhere"
                replace = name if kind in (tarfile.SYMTYPE, tarfile.LNKTYPE) else None
                self.fixture(member, replace=replace)
                code, _, error = self.run_installer()
                self.assertEqual(code, 1)
                self.assertIn("Unexpected entry", error)
                self.assertFalse(self.bin.exists())
                self.assertFalse((self.root / "escape").exists())

    def test_unsupported_platform_and_libc(self):
        for attribute, value in (("machine", "aarch64"), ("confstr", None),
                                 ("confstr", "glibc 2.38")):
            with self.subTest(attribute=attribute, value=value):
                owner = installer.platform if attribute == "machine" else installer.os
                with patch.object(owner, attribute, return_value=value):
                    code, _, error = self.run_installer()
                self.assertEqual(code, 1)
                self.assertIn("requires", error)
                self.assertFalse(self.curl_log.exists())

    def test_mismatched_executable_versions(self):
        self.fixture(version="0.5.0")
        code, _, error = self.run_installer()
        self.assertEqual(code, 1)
        self.assertIn("does not report the expected version", error)
        self.assertEqual(list(self.bin.iterdir()), [])

    def test_refuses_to_switch_while_real_process_is_active(self):
        self.assertEqual(self.run_installer()[0], 0)
        old = self.current.resolve()
        running = self.root / "rustune"
        shutil.copy2("/bin/sleep", running)
        process = subprocess.Popen([running, "15"])
        try:
            deadline = time.monotonic() + 2
            while Path(f"/proc/{process.pid}/comm").read_text().strip() != running.name:
                self.assertLess(time.monotonic(), deadline)
                time.sleep(0.01)
            code, _, error = self.run_installer()
            self.assertEqual(code, 1)
            self.assertIn("rustune ctl shutdown", error)
            self.assertIsNone(process.poll())
            self.assertEqual(self.current.resolve(), old)
        finally:
            process.terminate()
            process.wait(timeout=5)

    def test_failure_at_switch_preserves_pair(self):
        self.assertEqual(self.run_installer()[0], 0)
        old = self.current.resolve()
        old_bytes = self.installed()
        with patch.object(installer, "VERSION", "0.5.2"):
            self.fixture()
            with patch.object(Path, "replace", side_effect=OSError("test switch failure")):
                code, _, error = self.run_installer()
        self.assertEqual(code, 1)
        self.assertIn("test switch failure", error)
        self.assertEqual(self.current.resolve(), old)
        self.assertEqual(self.installed(), old_bytes)
        self.assertEqual(list(old.parent.iterdir()), [old])

    @unittest.skipUnless(release_archive, "pass RUSTUNE_TEST_RELEASE_ARCHIVE to check built executables")
    def test_built_pair_and_doctor(self):
        shutil.copyfile(release_archive, self.archive)
        code, _, error = self.run_installer(
            digest=installer.SHA256 if os.environ.get("RUSTUNE_TEST_PINNED_SHA") == "1" else None
        )
        self.assertEqual(code, 0, error)
        for name in installer.PROGRAMS:
            version = subprocess.check_output([self.bin / name, "--version"], text=True).strip()
            self.assertEqual(version, f"{name} {installer.VERSION}")
        doctor = subprocess.check_output([self.bin / "rustune", "doctor", "--json"], text=True)
        report = json.loads(doctor)
        self.assertEqual(report["audio_helper"]["state"], "compatible")
        self.assertEqual(report["token"], "missing")
        self.assertFalse(report["cdm_present"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
