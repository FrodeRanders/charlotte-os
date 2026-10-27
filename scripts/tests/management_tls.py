"""Verified TLS clients for native management probes; no plaintext fallback."""
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
