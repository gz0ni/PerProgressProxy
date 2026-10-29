"""Long-running probe used to prove that live rotation works.

Run it inside an injected process; it prints the exit IP reported by the
proxy every second. Nothing is installed, no dependencies.
"""
import socket
import sys
import time

HOST = sys.argv[1] if len(sys.argv) > 1 else "example.com"
PATH = sys.argv[2] if len(sys.argv) > 2 else "/"
INTERVAL = float(sys.argv[3]) if len(sys.argv) > 3 else 1.0
OUT = sys.argv[4] if len(sys.argv) > 4 else ''
COUNT = int(sys.argv[5]) if len(sys.argv) > 5 else 0


def once():
    s = socket.create_connection((HOST, 80), 8)
    try:
        req = "GET %s HTTP/1.0\r\nHost: %s\r\nConnection: close\r\n\r\n" % (PATH, HOST)
        s.sendall(req.encode())
        chunks = []
        while True:
            b = s.recv(4096)
            if not b:
                break
            chunks.append(b)
    finally:
        s.close()
    text = b"".join(chunks).decode("latin1", "replace")
    body = text.split("\r\n\r\n", 1)[-1].strip()
    return body.replace("\n", " | ")[:80]


done = 0
while True:
    stamp = time.strftime("%H:%M:%S")
    try:
        line = "%s  %s" % (stamp, once())
    except Exception as exc:  # noqa: BLE001
        line = "%s  ERROR %s" % (stamp, exc)
    if OUT:
        with open(OUT, "a", encoding="utf-8") as fh:
            fh.write(line + "\n")
    else:
        print(line, flush=True)
    done += 1
    if COUNT and done >= COUNT:
        break
    time.sleep(INTERVAL)
