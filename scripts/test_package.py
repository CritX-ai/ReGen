#!/usr/bin/env python3
"""Native notice collection must preserve Cargo's Unicode metadata on any locale."""

import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import package


class NoticeEncoding(unittest.TestCase):
    @unittest.skipUnless(sys.platform.startswith("linux"), "requires an ASCII C-locale subprocess")
    def test_non_utf8_locale_preserves_the_complete_notice_inventory(self):
        # Use real Cargo metadata and upstream notices, not a canned subprocess
        # response. Windows' legacy code page exposed the same decoding boundary.
        program = """import locale
from pathlib import Path
import sys
import package
if sys.argv[2] == "legacy":
    assert locale.getpreferredencoding(False).lower().replace("-", "") != "utf8"
package.collect_licenses(Path(sys.argv[1]), "x86_64-unknown-linux-gnu")
"""
        with tempfile.TemporaryDirectory(prefix="regen-notice-encoding-") as temporary:
            root = Path(temporary)
            snapshots = []
            for name, utf8 in (("utf8", "1"), ("legacy", "0")):
                destination = root / name
                environment = dict(os.environ, LC_ALL="C", PYTHONCOERCECLOCALE="0", PYTHONUTF8=utf8)
                result = subprocess.run(
                    [sys.executable, "-c", program, str(destination), name],
                    cwd=Path(package.__file__).parent, env=environment, capture_output=True,
                )
                self.assertEqual(result.returncode, 0, result.stderr.decode("utf-8"))
                snapshots.append({
                    path.relative_to(destination).as_posix(): path.read_bytes()
                    for path in destination.rglob("*") if path.is_file()
                })
            self.assertEqual(snapshots[0], snapshots[1])


if __name__ == "__main__":
    unittest.main()
