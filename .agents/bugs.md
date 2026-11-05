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

### Clash fake-ip connections refused for Bun apps on macOS (2026-11-05) - fixed

- Symptom: opencode (Bun) under the dylib failed every request; the log showed
  nothing.
- Root cause: Bun's resolver uses `DNSServiceQueryRecordWithAttribute`, so the
  name map was empty and `connect()` to `198.18.x.x` hit the silent
  `ENETUNREACH` branch.
- Fix: interpose the DNS-SD query calls and `getaddrinfo_async_start`, record
  A / AAAA answers under the queried name.
- Prevent: test new runtimes with a dead proxy (`PROXYINJECT_FORCE=socks5://127.0.0.1:1`):
  every request must log `failed for <hostname>:443`.


### macOS shim never hooked `connect()` (2026-11-05) - fixed

- Symptom: targets launched with `proxyinject.dylib` kept their direct IP.
- Root cause: the shim relied on symbol overriding plus `dlsym(RTLD_NEXT)`,
  which only works with flat namespaces (Linux); dyld ignores it.
- Fix: on `__APPLE__` the hooks are `ppp_connect` / `ppp_getaddrinfo`, registered
  in `__DATA,__interpose`; the originals are called directly.
- Prevent: run `tests/posix_selftest.sh` on a Mac after touching the shim.


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
