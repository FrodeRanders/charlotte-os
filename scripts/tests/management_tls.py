"""Verified TLS clients for native management probes; no plaintext fallback."""
import hashlib
import os
import socket
import ssl
import time

if not __debug__:
    raise RuntimeError("management TLS probes require enabled runtime checks; Python optimization is unsupported")


def context(role=None):
    ca = os.environ["CATTEN_MANAGEMENT_CA_FILE"]
    prefix = "CATTEN_MANAGEMENT_" + (role.upper() + "_" if role else "")
    cert = os.environ[prefix + "CERT_FILE"]
    key = os.environ[prefix + "KEY_FILE"]
    result = ssl.create_default_context(cafile=ca)
    result.minimum_version = ssl.TLSVersion.TLSv1_3
    result.maximum_version = ssl.TLSVersion.TLSv1_3
    result.set_alpn_protocols(["http/1.1"])
    result.load_cert_chain(certfile=cert, keyfile=key)
    return result


def connect(port, timeout=12, host="127.0.0.1", role=None):
    policy = context(role)
    end = time.monotonic() + timeout
    raw = socket.create_connection((host, port), timeout=timeout)
    try:
        raw.settimeout(max(0.001, end - time.monotonic()))
        return policy.wrap_socket(raw, server_hostname=os.environ.get("CATTEN_MANAGEMENT_SERVER_NAME", host))
    except BaseException:
        raw.close()
        raise


class ProbeFailure(RuntimeError):
    pass


def exchange(policy, port, payload, expected, budget, session=None):
    end = time.monotonic() + budget
    def remaining():
        value = end - time.monotonic()
        if value <= 0:
            raise TimeoutError("management exchange exhausted its original budget")
        return value
    for attempt in range(3):
        raw = None
        try:
            raw = socket.create_connection(("127.0.0.1", port), timeout=remaining())
            raw.settimeout(remaining())
            client = policy.wrap_socket(raw, server_hostname="127.0.0.1", session=session)
            break
        except ssl.SSLCertVerificationError:
            if raw:
                raw.close()
            raise
        except ConnectionError:
            if raw:
                raw.close()
            if attempt == 2:
                raise
            time.sleep(min(0.25, remaining()))  # Pre-HTTP listener handoff only.
        except BaseException:
            if raw:
                raw.close()
            raise
    with client:
        if client.version() != "TLSv1.3" or client.cipher()[0] != "TLS_AES_128_GCM_SHA256":
            raise ProbeFailure("unexpected TLS negotiation")
        client.settimeout(remaining())
        client.sendall(payload)  # Accepted HTTP uncertainty is never retried.
        data = bytearray()
        while True:
            client.settimeout(remaining())
            received = client.recv(4096)
            if not received:
                raise EOFError("no complete HTTP response")
            if expected is None:
                raise ProbeFailure("unauthorized client received application bytes")
            data.extend(received)
            if len(data) > 1024 * 1024:
                raise ProbeFailure("oversized management response")
            if b"\r\n\r\n" not in data:
                continue
            header, body = bytes(data).split(b"\r\n\r\n", 1)
            length = int(next(line.split(b":", 1)[1] for line in header.split(b"\r\n") if line.lower().startswith(b"content-length:")))
            if len(body) < length:
                continue
            code = int(header.split(b" ", 2)[1])
            if code != expected:
                raise ProbeFailure(f"unexpected HTTP result {code}: {body[:length]!r}")
            observed = {"status": code, "server_sha256": hashlib.sha256(client.getpeercert(binary_form=True)).hexdigest(),
                        "session_reused": client.session_reused, "has_ticket": client.session.has_ticket}
            return observed, client.session
