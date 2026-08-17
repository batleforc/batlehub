#!/usr/bin/env python3
"""Transparent logging proxy, for heavy tests that assert on the wire.

    http_tap.py <log-file> <listen-port> <backend-port> [cert.pem key.pem]

With a certificate and key the tap terminates TLS and forwards to the plain-HTTP
backend, adding `X-Forwarded-Proto: https`. Terraform needs that: the registry
protocol refuses a plain-`http:` host outright (RFC 0009 §12.3), so the only way
to observe a `terraform init` on the wire is to give it TLS — and the server has
to know the client's scheme, or it advertises `http://` URLs inside documents
that were fetched over `https://`.

Writes one line per request:

    GET /proxy/gems/versions -> 206 (54B) | Range: bytes=89- | Content-Range: …

Why a tap rather than the server's own access log: what these tests assert is
the *client's* request sequence — which conditional headers it sent, whether it
re-fetched after a partial response — and the sequence is the evidence, not the
statuses in isolation.

`Host` is passed through untouched, deliberately. BatleHub builds its absolute
URL templates (npm `dist.tarball`, NuGet service index, PyPI simple pages) from
that header, so a tap that rewrites it hands the client URLs pointing straight
at the backend: every request after the first bypasses the tap and the
transcript looks clean because nothing was observed (RFC 0009 §12.10).
"""

import http.client
import ssl
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOG_PATH, LISTEN_PORT, BACKEND_PORT = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
TLS_CERT, TLS_KEY = (sys.argv[4], sys.argv[5]) if len(sys.argv) > 5 else (None, None)
LOG = open(LOG_PATH, "a", buffering=1)  # noqa: SIM115 — lives for the process

# Hop-by-hop headers must not be forwarded (RFC 9110 §7.6.1); Content-Length is
# recomputed because the body is buffered here.
HOP = {"connection", "keep-alive", "transfer-encoding", "te", "trailer", "upgrade"}

# Request headers worth recording: the ones that make an answer conditional.
ASKED = ("Range", "If-None-Match", "If-Modified-Since")
# Response headers worth recording: the ones that say what kind of answer it is.
ANSWERED = ("Content-Range", "ETag", "Repr-Digest")


class Tap(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_args):
        """Silence BaseHTTPRequestHandler's own stderr logging."""

    def _read_chunked(self):
        """Decode a `Transfer-Encoding: chunked` request body.

        `BaseHTTPRequestHandler` does not do this, and the clients that need it
        do not announce themselves: `ovsx publish` streams the VSIX chunked with
        no `Content-Length`, so a tap that reads only `Content-Length` forwards
        an **empty body** and the server answers 400 "could not read
        extension/package.json". The failure looks exactly like a broken
        publish endpoint, and the endpoint is fine.
        """
        chunks = []
        while True:
            line = self.rfile.readline().strip()
            size = int(line.split(b";")[0] or b"0", 16)
            if size == 0:
                # Consume the trailer section up to the blank line.
                while self.rfile.readline().strip():
                    pass
                break
            chunks.append(self.rfile.read(size))
            self.rfile.read(2)  # CRLF after each chunk
        return b"".join(chunks)

    def _proxy(self):
        body = None
        length = self.headers.get("Content-Length")
        if length:
            body = self.rfile.read(int(length))
        elif "chunked" in (self.headers.get("Transfer-Encoding") or "").lower():
            body = self._read_chunked()

        headers = {k: v for k, v in self.headers.items() if k.lower() not in HOP}
        if TLS_CERT:
            # The client spoke TLS to us and we speak plain HTTP to the backend.
            # Without this the server builds `http://…` URLs into documents the
            # client fetched over `https://`, and a client that requires TLS
            # (Terraform does) refuses to follow them.
            headers["X-Forwarded-Proto"] = "https"
        conn = http.client.HTTPConnection("127.0.0.1", BACKEND_PORT, timeout=120)
        conn.request(self.command, self.path, body=body, headers=headers)
        resp = conn.getresponse()
        payload = resp.read()

        asked = [f"{h}: {self.headers[h]}" for h in ASKED if self.headers.get(h)]
        answered = [f"{h}: {resp.getheader(h)}" for h in ANSWERED if resp.getheader(h)]
        LOG.write(
            f"{self.command} {self.path} -> {resp.status} ({len(payload)}B)"
            + (" | " + " ; ".join(asked) if asked else "")
            + (" | " + " ; ".join(answered) if answered else "")
            + "\n"
        )

        self.send_response(resp.status)
        for k, v in resp.getheaders():
            if k.lower() in HOP or k.lower() == "content-length":
                continue
            self.send_header(k, v)
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(payload)
        conn.close()

    do_GET = do_POST = do_PUT = do_HEAD = do_DELETE = _proxy


if __name__ == "__main__":
    server = ThreadingHTTPServer(("127.0.0.1", LISTEN_PORT), Tap)
    if TLS_CERT:
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(TLS_CERT, TLS_KEY)
        server.socket = context.wrap_socket(server.socket, server_side=True)
    server.serve_forever()
