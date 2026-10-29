from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from ppp import netcheck, store
from ppp.specs import SpecError, parse_spec, split_tokens


class ParseSpecTest(unittest.TestCase):
    def test_webshare_three_colon_form(self):
        line = "45.38.107.97:6014:mvcwtmiw:g36umzrtdlw0"
        p = parse_spec(line)
        self.assertEqual((p.host, p.port, p.user, p.pass_), ("45.38.107.97", 6014, "mvcwtmiw", "g36umzrtdlw0"))
        self.assertEqual(p.scheme, "http")

    def test_webshare_file_lines_all_parse(self):
        src = Path(__file__).resolve().parent.parent / "proxy" / "webshare.txt"
        if not src.exists():
            self.skipTest("proxy/webshare.txt not present")
        tokens = split_tokens(src.read_text(encoding="utf-8", errors="replace"))
        self.assertTrue(tokens)
        for token in tokens:
            parse_spec(token)

    def test_password_with_colon(self):
        p = parse_spec("1.2.3.4:8080:user:pa:ss")
        self.assertEqual(p.user, "user")
        self.assertEqual(p.pass_, "pa:ss")

    def test_socks_url_with_credentials(self):
        p = parse_spec("socks5://bob:secret@10.0.0.1:1080")
        self.assertEqual((p.scheme, p.host, p.port, p.user, p.pass_), ("socks5", "10.0.0.1", 1080, "bob", "secret"))

    def test_host_port_guesses_scheme_like_dll(self):
        self.assertEqual(parse_spec("10.0.0.1:1080").scheme, "socks5")
        self.assertEqual(parse_spec("10.0.0.1:9999").scheme, "http")

    def test_bracketed_ipv6(self):
        p = parse_spec("socks5://[::1]:1080")
        self.assertEqual((p.host, p.port), ("::1", 1080))

    def test_socks4_rejects_credentials(self):
        with self.assertRaises(SpecError):
            parse_spec("socks4://user:pass@1.2.3.4:1080")

    def test_rejects_garbage(self):
        for bad in ("", "# comment", "1.2.3.4", "1.2.3.4:notaport", "ftp://1.2.3.4:21"):
            with self.assertRaises(SpecError):
                parse_spec(bad)

    def test_display_masks_password(self):
        p = parse_spec("socks5://bob:secret@10.0.0.1:1080")
        self.assertNotIn("secret", p.display)
        self.assertIn("secret", p.label)


class SplitTokensTest(unittest.TestCase):
    def test_mixed_file(self):
        raw = "# header\n1.2.3.4:8080\n\n5.6.7.8:1080:u:p  9.9.9.9:80,other:3128\n"
        self.assertEqual(
            split_tokens(raw),
            ["1.2.3.4:8080", "5.6.7.8:1080:u:p", "9.9.9.9:80", "other:3128"],
        )


class StoreTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self._saved = {
            "PROXIES": store.PROXIES,
            "CONFIG": store.CONFIG,
            "PROXY_DIR": store.PROXY_DIR,
        }
        store.PROXIES = self.root / "proxies.txt"
        store.CONFIG = self.root / "config.ini"
        store.PROXY_DIR = self.root / "proxy"
        store.PROXY_DIR.mkdir()

    def tearDown(self):
        for key, value in self._saved.items():
            setattr(store, key, value)
        self._tmp.cleanup()

    def test_sync_imports_and_dedups(self):
        (store.PROXY_DIR / "a.txt").write_text("1.2.3.4:8080\n5.6.7.8:1080:u:p\n", encoding="utf-8")
        first = store.sync_proxy_dir()
        self.assertEqual(first["added"], 2)
        second = store.sync_proxy_dir()
        self.assertEqual(second["added"], 0)
        self.assertEqual(len(store.proxies()), 2)

    def test_add_specs_dedups(self):
        added, _notes = store.add_specs(["1.2.3.4:8080", "http://1.2.3.4:8080"])
        self.assertEqual(added, 1)


if __name__ == "__main__":
    unittest.main()
