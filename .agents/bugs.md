# Bugs

Known bugs and their fixes. Newest first. Status: `open` / `fixed` / `won't fix`.

## Format

```
### <Short bug title> (YYYY-MM-DD) - status

- Symptom
- Root cause
- Fix applied
- How to prevent recurrence
```

---

### Import silently dropped proxy passwords (2026-10-28) - fixed

- Symptom: importing `host:port:user:pass` wrote `socks5://user@host:port` into
  `proxies.txt`, so the proxy rejected every connection.
- Root cause: the canonical form used for writing came from `Proxy.label`, which
  omitted the password; the parser only accepted that line shape for `>= 4` colons.
- Fix: `label` carries the full credential for dedup and storage, and `display`
  is the masked form used in all output.
- Prevent: `test_display_masks_password` asserts the split, and the token written
  to disk is the parsed label, never a partially rebuilt string.

### `host:port:user:pass` proxies are not detected on import (2026-10-28) - fixed

- Symptom: importing `proxy/webshare.txt` reported `imported 0, skipped 10`.
- Root cause: `parse_spec()` required four or more colons for the credential
  form; WebShare lists use three.
- Fix: credential branch accepts three or more colons with a host that contains
  no colon, so IPv6 without brackets is not swallowed.
- Prevent: `tests/test_specs.py` covers the exact WebShare shape.
