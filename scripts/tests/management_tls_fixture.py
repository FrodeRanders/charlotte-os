"""One disposable mTLS fixture for each host-test process and its real CLI.

This owns its private directory through process exit. Every mock listener checks
the client certificate; the CLI checks the fixture's server root and IP name.
"""
import atexit
import functools
import importlib.util
import os
from pathlib import Path
import ssl
import tempfile


@functools.cache
def configure_client():
    owner = tempfile.TemporaryDirectory(prefix="charlotte-management-http-")
    atexit.register(owner.cleanup)
    directory = Path(owner.name)
    spec = importlib.util.spec_from_file_location(
        "prepare_management_tls_fixture", Path(__file__).resolve().parents[1] / "prepare-management-tls-fixture.py")
    generator = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(generator)
    generator.prepare(directory, retain_server_key=True)
    os.environ.update(CATTEN_MANAGEMENT_CA_FILE=str(directory / "ca.pem"),
                      CATTEN_MANAGEMENT_CERT_FILE=str(directory / "administrator.pem"),
                      CATTEN_MANAGEMENT_KEY_FILE=str(directory / "administrator.key"),
                      CATTEN_MANAGEMENT_SERVER_NAME="127.0.0.1")
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_3
    context.maximum_version = ssl.TLSVersion.TLSv1_3
    context.load_cert_chain(directory / "server.pem", directory / "server.key")
    context.load_verify_locations(directory / "ca.pem")
    context.verify_mode = ssl.CERT_REQUIRED
    context.set_alpn_protocols(["http/1.1"])
    return owner, context


def install(server):
    _, context = configure_client()
    server.socket = context.wrap_socket(server.socket, server_side=True)
