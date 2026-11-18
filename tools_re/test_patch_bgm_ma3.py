import contextlib
import io
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest.mock import patch

import patch_bgm_ma3 as bgm


def archive(members):
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w") as z:
        for name, data in members.items():
            z.writestr(name, data)
    return out.getvalue()


class PatchBgmTests(unittest.TestCase):
    def run_case(self, pristine_names):
        scores = {}
        for name, rows in bgm.PATCHES.items():
            data = bytearray(max(row[0] for row in rows) + 1)
            for offset, old, new in rows:
                data[offset] = old if name in pristine_names else new
            scores[f"res/snd/{name}"] = bytes(data)
        scores["untouched.bin"] = b"preserve me"
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "game.zip"
            path.write_bytes(archive({"game.jar": archive(scores), "metadata": b"unchanged"}))
            with patch.object(bgm.PA, "backup_paths", return_value=(str(Path(directory) / "backup" / "game.zip"), "unused")):
                with patch("sys.argv", ["patch_bgm_ma3.py", str(path), "--apply"]), contextlib.redirect_stdout(io.StringIO()):
                    bgm.main()
            _, _, _, jar, found = bgm._members(path.read_bytes())
            for name, rows in bgm.PATCHES.items():
                self.assertEqual(bgm._state(jar.read(found[name]), rows, name), (len(rows), 0))
            self.assertEqual(jar.read("untouched.bin"), b"preserve me")
            with zipfile.ZipFile(path) as z:
                self.assertEqual(z.read("metadata"), b"unchanged")
            before = path.read_bytes()
            with patch("sys.argv", ["patch_bgm_ma3.py", str(path), "--apply"]), contextlib.redirect_stdout(io.StringIO()):
                bgm.main()
            self.assertEqual(path.read_bytes(), before)

    def test_repairs_one_unpatched_score(self):
        self.run_case({"bgm4_mf"})

    def test_patches_pristine_archive(self):
        self.run_case(set(bgm.PATCHES))

    def test_already_patched_archive_is_unchanged(self):
        self.run_case(set())

    def test_rejects_unknown_byte(self):
        with self.assertRaises(SystemExit):
            bgm._state(b"\xff", [(0, 1, 2)], "unknown")


if __name__ == "__main__":
    unittest.main()
