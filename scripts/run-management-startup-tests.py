#!/usr/bin/env python3
"""Native development-root management startup campaign; never policy installation.

Each case builds its exact kernel input and boots fresh isolated storage. Private
test keys live in one temporary directory; retained evidence contains hashes and
specific observations. Resource substitution must reject before trust publication.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
CASES = {"valid": "ready", "validity": "validity", "missing": "disabled", "invalid-der": "disabled",
         "wrong-policy": "WrongPolicy", "signature": "Signature", "ciphertext": "Authentication"}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def command(arguments, log, environment=None, timeout=900):
    with log.open("ab") as output:
        process = subprocess.Popen([str(value) for value in arguments], cwd=ROOT,
                                   env=environment, stdout=output, stderr=subprocess.STDOUT,
                                   start_new_session=True)
        try:
            return process.wait(timeout=timeout)
        except BaseException:
            # One owned process group includes this runner's QEMU child. Stop
            # the complete test on interruption; never leave a guest orphaned.
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
            raise


def resources(private, validity_controls=False):
    spec = importlib.util.spec_from_file_location("management_fixture", ROOT / "scripts/prepare-management-tls-fixture.py")
    generator = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(generator)
    directory = private / "credentials"
    generator.prepare(directory, validity_controls=validity_controls)
    signer = ROOT / "target/debug/cluster-sign"
    preparation = private / "preparation.log"
    def tool(*args):
        if command([signer, *args], preparation) != 0:
            raise RuntimeError("management fixture command failed; see private preparation capture")
    policy = directory / "policy.bin"
    original = (directory / "resource.bin").read_bytes()
    result = {"valid": directory / "resource.bin", "validity": directory / "resource.bin", "missing": None}
    for name, position in (("wrong-policy", 16), ("signature", 112)):
        changed = bytearray(original)
        changed[position] ^= 1
        result[name] = private / (name + ".bin")
        result[name].write_bytes(changed)
    key = private / "invalid-key.der"
    key.write_bytes(b"public synthetic invalid DER private key")
    key.chmod(0o600)
    profile = private / "invalid-profile.bin"
    tool("management-profile-create", profile, directory / "limits.txt", key,
         "chain=" + str(directory / "server.der"), "root=" + str(directory / "ca.der"),
         "reader=" + str(directory / "reader.der"), "administrator=" + str(directory / "administrator.der"))
    result["invalid-der"] = private / "invalid-der.bin"
    tool("management-resource-seal", result["invalid-der"], policy,
         ROOT / "tools/cluster-sign/dev-operations-key.hex", profile)

    # Correctly re-sign a damaged ciphertext using only the public development
    # signer. Public verification must succeed before exercising native AEAD.
    changed = bytearray(original)
    changed[176] ^= 1
    text = (ROOT / "tools/cluster-sign/dev-operations-key.hex").read_text()
    material = bytes.fromhex("".join(line.split("#", 1)[0].strip() for line in text.splitlines()))
    assert len(material) == 64
    signing_key = private / "public-fixture-operations.der"
    signing_key.write_bytes(bytes.fromhex("302e020100300506032b657004220420") + material[:32])
    signing_key.chmod(0o600)
    message = private / "ciphertext-signature-digest"
    message.write_bytes(hashlib.sha256(b"CharlotteOS management boot resource signature v1\0" + changed[:112] + changed[176:]).digest())
    signature = private / "ciphertext-signature"
    if command(["openssl", "pkeyutl", "-sign", "-rawin", "-inkey", signing_key,
                "-keyform", "DER", "-in", message, "-out", signature], preparation) != 0:
        raise RuntimeError("public fixture re-signing failed")
    assert signature.stat().st_size == 64
    changed[112:176] = signature.read_bytes()
    result["ciphertext"] = private / "ciphertext.bin"
    result["ciphertext"].write_bytes(changed)
    tool("management-resource-verify", result["ciphertext"], policy)
    return directory, result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--architectures", nargs="+", choices=("aarch64", "x86_64"), default=["x86_64", "aarch64"])
    parser.add_argument("--cases", nargs="+", choices=tuple(CASES), default=list(CASES))
    parser.add_argument("--output", type=Path)
    parser.add_argument("--reuse-bundle", action="store_true", help="reuse an already built and signed current-source service bundle")
    parser.add_argument("--status-port", type=int, default=25443)
    parser.add_argument("--deployment-port", type=int, default=25744)
    parser.add_argument("--timeout", type=int, default=90, help="original guest observation deadline in seconds")
    parser.add_argument("--build-timeout", type=int, default=900)
    args = parser.parse_args()
    if os.name != "posix" or args.timeout <= 0 or args.build_timeout <= 0:
        parser.error("requires POSIX process groups and positive deadlines")
    if not (1 <= args.status_port <= 65535 and 1 <= args.deployment_port <= 65535) or args.status_port == args.deployment_port:
        parser.error("management forwards require two distinct valid ports")
    output = (args.output or ROOT / "target/management-startup-tests" / f"run-{time.time_ns()}").resolve()
    output.mkdir(parents=True, exist_ok=False)
    evidence = {"scope": "Native development-root startup only; no installed-state rotation or production custody proof",
                "parent_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
                "results": []}
    report = output / "evidence.json"
    def save():
        report.write_text(json.dumps(evidence, indent=2) + "\n")
    save()
    with tempfile.TemporaryDirectory(prefix="private.", dir=output) as temporary:
        private = Path(temporary)
        credentials, variants = resources(private, validity_controls="validity" in args.cases)
        if "validity" in args.cases:
            evidence["validity_credentials"] = json.loads((credentials / "validity.json").read_text())
            save()
        for architecture in args.architectures:
            if not args.reuse_bundle:
                builder = [ROOT / "scripts/build-catten-services.sh", "--embed"] if architecture == "aarch64" else [ROOT / "scripts/build-catten-services-x86_64.sh"]
                if command(builder, output / (architecture + "-bundle.log"), timeout=args.build_timeout) != 0:
                    raise RuntimeError("service bundle build failed")
            for case in args.cases:
                instance = f"management-startup-{architecture[:3]}-{case}-{private.name[-6:]}"
                environment = dict(os.environ)
                for name in ("CATTEN_MANAGEMENT_TLS_RESOURCE", "CATTEN_MANAGEMENT_VALIDITY_DIR", "CATTEN_BOOT_TEST_INSTALLED_POLICY", "CATTEN_SKIP_KERNEL_BUILD", "CATTEN_CLUSTER_SERVICES",
                             "CATTEN_CLUSTER_STATIC_NETWORK", "CATTEN_RESOURCE_POLICY", "CATTEN_LIMINE_CONFIG",
                             "CATTEN_QEMU_PID_FILE", "CATTEN_QEMU_MONITOR", "CATTEN_QEMU_DEBUG_STUB"):
                    environment.pop(name, None)
                environment.update(CATTEN_TRUST_MODE="development", CATTEN_SKIP_EMBED_BUILD="1",
                                   CATTEN_HTTP_HOST_PORT=str(args.status_port), CATTEN_DEPLOY_HOST_PORT=str(args.deployment_port),
                                   CATTEN_MANAGEMENT_CA_FILE=str(credentials / "ca.pem"),
                                   CATTEN_MANAGEMENT_CERT_FILE=str(credentials / "administrator.pem"),
                                   CATTEN_MANAGEMENT_KEY_FILE=str(credentials / "administrator.key"),
                                   CATTEN_MANAGEMENT_READER_CERT_FILE=str(credentials / "reader.pem"),
                                   CATTEN_MANAGEMENT_READER_KEY_FILE=str(credentials / "reader.key"),
                                   CATTEN_MANAGEMENT_SERVER_NAME="127.0.0.1")
                if case == "validity":
                    environment["CATTEN_MANAGEMENT_VALIDITY_DIR"] = str(credentials)
                resource = variants[case]
                if resource:
                    environment["CATTEN_MANAGEMENT_TLS_RESOURCE"] = str(resource)
                arguments = [ROOT / f"scripts/run-{architecture}.sh", "--management-startup-test", CASES[case],
                             "--instance", instance, "--timeout", str(args.timeout), "--no-failure-capture"]
                print(f"START {architecture} {case}", flush=True)
                capture = output / f"{architecture}-{case}.log"
                record = {"architecture": architecture, "case": case, "expected": CASES[case],
                          "exit_status": None, "state": "running",
                          "resource_sha256": digest(resource) if resource else None}
                evidence["results"].append(record)
                save()
                try:
                    status = command(arguments, capture, environment, args.build_timeout + args.timeout)
                except subprocess.TimeoutExpired:
                    status = 124
                    record["deadline_failure"] = "outer build/guest observation budget expired; no startup result"
                except BaseException:
                    record["state"] = "interrupted"
                    save()
                    raise
                text = capture.read_text(errors="replace")
                kernel = re.search(r"Kernel SHA-256: ([0-9a-f]{64})", text)
                record.update({"state": "passed" if status == 0 else "failed", "exit_status": status,
                          "kernel_sha256": kernel.group(1) if kernel else None,
                          "observations": [line.rstrip() for line in text.splitlines() if line.startswith(("PASS:", "OBSERVED:")) or
                                           any(marker in line for marker in ("admission policy published:", "credentials unavailable; ingress disabled",
                                                                            "management TLS resource rejected before trust publication:", "SELFTEST COMPLETE:",
                                                                            "Kernel panic:", "boot admission trust rejected", "[time] synchronized ",
                                                                            "client certificate expired", "client certificate not yet valid"))],
                          "failure_tail": text.splitlines()[-20:] if status else []})
                save()
                print(f"{'PASS' if status == 0 else 'FAIL'} {architecture} {case}", flush=True)
                if status:
                    return 1
    print(f"Evidence: {report}", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
