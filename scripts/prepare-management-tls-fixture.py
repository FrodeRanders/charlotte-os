#!/usr/bin/env python3
"""Create disposable public mTLS credentials and a signed/encrypted boot resource.

This uses the repository's public development authorities. It is a QEMU fixture,
not production enrollment. Private files stay in the requested mode-0700 directory.
"""
import argparse
import ipaddress
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def prepare(directory, addresses=(), retain_server_key=False):
    directory.mkdir(parents=True, exist_ok=True)
    directory.chmod(0o700)
    def openssl(*arguments):
        subprocess.run(["openssl", *arguments], cwd=directory, check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    for name in ("ca", "server", "reader", "administrator", "unknown"):
        openssl("ecparam", "-name", "prime256v1", "-genkey", "-noout", "-out", name + ".key")
        (directory / (name + ".key")).chmod(0o600)
    openssl("req", "-x509", "-new", "-key", "ca.key", "-days", "2", "-subj",
            "/CN=Charlotte disposable management CA", "-addext", "basicConstraints=critical,CA:TRUE",
            "-addext", "keyUsage=critical,keyCertSign,cRLSign", "-out", "ca.pem")
    ips = {"127.0.0.1", "10.0.2.15", *(str(ipaddress.ip_address(ip)) for ip in addresses)}
    san = "DNS:localhost,DNS:management.test," + ",".join("IP:" + ip for ip in sorted(ips))
    for serial, name in enumerate(("server", "reader", "administrator", "unknown"), 2):
        openssl("req", "-new", "-key", name + ".key", "-subj", "/CN=" + name,
                "-out", name + ".csr")
        purpose = "serverAuth" if name == "server" else "clientAuth"
        (directory / "extensions").write_text(
            "basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\n"
            + "extendedKeyUsage=" + purpose + "\nsubjectAltName=" + san + "\n")
        openssl("x509", "-req", "-in", name + ".csr", "-CA", "ca.pem", "-CAkey", "ca.key",
                "-set_serial", str(serial), "-days", "2", "-extfile", "extensions", "-out", name + ".pem")
    for name in ("ca", "server", "reader", "administrator"):
        openssl("x509", "-in", name + ".pem", "-outform", "DER", "-out", name + ".der")
    openssl("pkcs8", "-topk8", "-nocrypt", "-in", "server.key", "-outform", "DER", "-out", "server-key.der")
    channel = tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    # Host tools must build outside the kernel's build-std Cargo configuration.
    subprocess.run(["cargo", "+" + channel, "build", "--locked", "--quiet", "--manifest-path",
                    str(ROOT / "tools/cluster-sign/Cargo.toml")], cwd=tempfile.gettempdir(), check=True)
    command = [str(ROOT / "target/debug/cluster-sign")]
    def tool(*arguments):
        subprocess.run(command + list(arguments), cwd=directory, check=True)
    tool("management-limits-default", "limits.txt")
    tool("management-policy-development", "policy.bin", "charlotte")
    tool("management-profile-create", "private-profile.bin", "limits.txt", "server-key.der",
         "chain=server.der", "root=ca.der", "reader=reader.der", "administrator=administrator.der")
    tool("management-resource-seal", "resource.bin", "policy.bin",
         str(ROOT / "tools/cluster-sign/dev-operations-key.hex"), "private-profile.bin")
    tool("management-resource-verify", "resource.bin", "policy.bin")
    for name in ("private-profile.bin", "server-key.der", "ca.key"):
        (directory / name).unlink()
    if not retain_server_key:
        (directory / "server.key").unlink()
    return directory


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--address", action="append", default=[])
    parser.add_argument("--retain-server-key", action="store_true",
                        help="retain the private server PEM only for disposable host TLS servers")
    args = parser.parse_args()
    prepare(args.directory.resolve(), args.address, args.retain_server_key)
