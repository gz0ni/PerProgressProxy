#!/usr/bin/env python3
"""Dev-only SOCKS5 test server.

Acts as an "exit node": logs every CONNECT, requires user/pass when asked, and
answers plain-HTTP requests itself with a canned page so a client can prove
which exit it is really talking to.

    python testsocks.py --port 11080 --fake-ip 10.9.9.9 --user u --pass p
"""
import argparse
import socket
import struct
import sys
import threading
import time

FAKE_BODY = b"exit-ip: %s\n"


def recv_exact(sock, n):
    buf = b""
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise ConnectionError("eof")
        buf += chunk
    return buf


def log(msg):
    print(f"[{time.strftime('%H:%M:%S')}] {msg}", flush=True)


def handle(conn, addr, fake_ip, user, pw):
    try:
        conn.settimeout(20)
        ver, nmeth = recv_exact(conn, 2)
        if ver != 5:
            return
        methods = recv_exact(conn, nmeth)
        if user:
            if 2 not in methods:
                conn.sendall(b"\x05\xff")
                return
            conn.sendall(b"\x05\x02")
            v, ulen = recv_exact(conn, 2)
            uname = recv_exact(conn, ulen).decode()
            plen = recv_exact(conn, 1)[0]
            passwd = recv_exact(conn, plen).decode()
            ok = uname == user and passwd == pw
            log(f"auth user={uname} pass={passwd} -> {'OK' if ok else 'DENY'}")
            conn.sendall(b"\x01\x00" if ok else b"\x01\x01")
            if not ok:
                return
        else:
            conn.sendall(b"\x05\x00")

        ver, cmd, _rsv, atyp = recv_exact(conn, 4)
        if atyp == 1:
            host = socket.inet_ntoa(recv_exact(conn, 4))
        elif atyp == 3:
            ln = recv_exact(conn, 1)[0]
            host = recv_exact(conn, ln).decode()
        elif atyp == 4:
            host = socket.inet_ntop(socket.AF_INET6, recv_exact(conn, 16))
        else:
            return
        port = struct.unpack("!H", recv_exact(conn, 2))[0]
        log(f"CONNECT {host}:{port} from {addr[0]}:{addr[1]}")

        conn.sendall(b"\x05\x00\x00\x01" + socket.inet_aton("0.0.0.0") + b"\x00\x00")

        if port == 80 and host in ("example.com", "example.org", "test.exit", "checkip.amazonaws.com", "ip.test", "icanhazip.com"):
            # read the request first, otherwise closing with unread data sends RST
            try:
                req = b""
                while b"\r\n\r\n" not in req and len(req) < 65536:
                    chunk = conn.recv(4096)
                    if not chunk:
                        break
                    req += chunk
                log(f"  request {req.split(b'\\r\\n')[0].decode('latin1', 'replace')}")
            except OSError:
                pass
            if host in ("checkip.amazonaws.com", "icanhazip.com", "ip.test"):
                body = (fake_ip + chr(10)).encode()
            else:
                body = FAKE_BODY % fake_ip.encode()
            resp = (
                b"HTTP/1.0 200 OK\r\nContent-Type: text/plain\r\n"
                b"Content-Length: %d\r\nConnection: close\r\n\r\n" % len(body)
            ) + body
            conn.sendall(resp)
            try:
                conn.shutdown(socket.SHUT_WR)
            except OSError:
                pass
            time.sleep(0.2)
            return

        up = socket.create_connection((host, port), timeout=15)
        conn.settimeout(None)
        up.settimeout(None)

        def pipe(a, b):
            try:
                while True:
                    data = a.recv(65536)
                    if not data:
                        break
                    b.sendall(data)
            except OSError:
                pass
            finally:
                try:
                    b.shutdown(socket.SHUT_WR)
                except OSError:
                    pass

        t = threading.Thread(target=pipe, args=(up, conn), daemon=True)
        t.start()
        pipe(conn, up)
        t.join(timeout=5)
        up.close()
    except (ConnectionError, OSError):
        pass
    finally:
        try:
            conn.close()
        except OSError:
            pass


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=11080)
    ap.add_argument("--bind", default="127.0.0.1")
    ap.add_argument("--fake-ip", default="10.9.9.9")
    ap.add_argument("--user", default="")
    ap.add_argument("--pass", dest="pw", default="")
    args = ap.parse_args()

    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind((args.bind, args.port))
    srv.listen(64)
    log(f"listening on {args.bind}:{args.port} fake_ip={args.fake_ip}")
    while True:
        conn, addr = srv.accept()
        threading.Thread(target=handle, args=(conn, addr, args.fake_ip, args.user, args.pw),
                         daemon=True).start()


if __name__ == "__main__":
    sys.exit(main())
