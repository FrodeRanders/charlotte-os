#!/usr/bin/env python3
"""Create disposable public mTLS credentials and a signed/encrypted boot resource.

This uses the repository's public development authorities. It is a QEMU fixture,
not production enrollment. Private files stay in the requested mode-0700 directory.
"""
import argparse
from datetime import datetime, timedelta, timezone
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def prepare_credentials(directory, addresses=(), *, authority_name="Charlotte disposable management CA", validity_controls=False):
    directory.mkdir(parents=True, exist_ok=True)
    directory.chmod(0o700)
    def openssl(*arguments):
        subprocess.run(["openssl", *arguments], cwd=directory, check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    for name in ("ca", "server", "reader", "administrator", "unknown"):
        openssl("ecparam", "-name", "prime256v1", "-genkey", "-noout", "-out", name + ".key")
        (directory / (name + ".key")).chmod(0o600)
    openssl("req", "-x509", "-new", "-key", "ca.key", "-days", "2", "-subj",
            "/CN=" + authority_name, "-addext", "basicConstraints=critical,CA:TRUE",
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
    if validity_controls:
        # Explicit windows exercise certificate validation without overriding
        # the guest clock. All CA state remains in this disposable owner.
        (directory / "validity-index").write_text("")
        (directory / "validity-serial").write_text("1000\n")
        (directory / "validity-ca.conf").write_text(
            "[ca]\ndefault_ca=fixture\n[fixture]\ndatabase=validity-index\n"
            "new_certs_dir=.\ncertificate=ca.pem\nprivate_key=ca.key\nserial=validity-serial\n"
            "default_md=sha256\npolicy=fixture_policy\n[fixture_policy]\ncommonName=supplied\n")
        now = datetime.now(timezone.utc).replace(microsecond=0)
        records = []
        for category, start, end in (("expired", -2, -1), ("future", 1, 2)):
            for role in ("reader", "administrator"):
                name = category + "-" + role
                openssl("ecparam", "-name", "prime256v1", "-genkey", "-noout", "-out", name + ".key")
                (directory / (name + ".key")).chmod(0o600)
                openssl("req", "-new", "-key", name + ".key", "-subj", "/CN=" + name, "-out", name + ".csr")
                before, after = (now + timedelta(days=offset) for offset in (start, end))
                openssl("ca", "-batch", "-notext", "-config", "validity-ca.conf", "-extfile", "extensions",
                        "-startdate", before.strftime("%Y%m%d%H%M%SZ"), "-enddate", after.strftime("%Y%m%d%H%M%SZ"),
                        "-in", name + ".csr", "-out", name + ".pem")
                openssl("x509", "-in", name + ".pem", "-outform", "DER", "-out", name + ".der")
                # Validate signature, chain, purpose and encoding independently
                # of time, then require the exact current-time failure below.
                openssl("verify", "-no_check_time", "-purpose", "sslclient", "-CAfile", "ca.pem", name + ".pem")
                checked = subprocess.run(["openssl", "verify", "-purpose", "sslclient", "-CAfile", "ca.pem", name + ".pem"],
                                         cwd=directory, capture_output=True, text=True)
                reason = "certificate has expired" if category == "expired" else "certificate is not yet valid"
                if checked.returncode == 0 or reason not in checked.stderr:
                    raise RuntimeError("validity fixture did not produce its specific certificate-date rejection")
                records.append({"name": name, "role": role, "category": category,
                                "not_before": before.isoformat(), "not_after": after.isoformat(),
                                "sha256": hashlib.sha256((directory / (name + ".der")).read_bytes()).hexdigest(),
                                "chain_purpose_without_time": "verified", "current_time_rejection": reason})
        (directory / "validity.json").write_text(json.dumps(records, indent=2) + "\n")
    (directory / "ca.key").unlink()
    return directory


def build_signer():
    channel = tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    # Host tools must build outside the kernel's build-std Cargo configuration.
    subprocess.run(["cargo", "+" + channel, "build", "--locked", "--quiet", "--manifest-path",
                    str(ROOT / "tools/cluster-sign/Cargo.toml")], cwd=tempfile.gettempdir(), check=True)
    return ROOT / "target/debug/cluster-sign"


def prepare(directory, addresses=(), retain_server_key=False, *, validity_controls=False):
    prepare_credentials(directory, addresses, validity_controls=validity_controls)
    command = [str(build_signer())]
    def tool(*arguments):
        subprocess.run(command + list(arguments), cwd=directory, check=True)
    tool("management-limits-default", "limits.txt")
    tool("management-policy-development", "policy.bin", "charlotte")
    operators = ["reader=reader.der", "administrator=administrator.der"]
    if validity_controls:
        operators.extend(f"{role}={category}-{role}.der" for category in ("expired", "future") for role in ("reader", "administrator"))
    tool("management-profile-create", "private-profile.bin", "limits.txt", "server-key.der",
         "chain=server.der", "root=ca.der", *operators)
    tool("management-resource-seal", "resource.bin", "policy.bin",
         str(ROOT / "tools/cluster-sign/dev-operations-key.hex"), "private-profile.bin")
    tool("management-resource-verify", "resource.bin", "policy.bin")
    for name in ("private-profile.bin", "server-key.der"):
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
