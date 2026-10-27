#!/usr/bin/env python3
"""Signed-policy 7 -> 8 and management credential rotation in x86 Secure Boot QEMU.

The installed snapshot, firmware enrollment and keys are public/disposable test
state. This is not a production installer or rollback-resistant storage. A final
retired-kernel replay explicitly records the remaining platform rollback gap.
"""
import argparse
from contextlib import contextmanager
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import signal
import socket
import ssl
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


secure = module("secure_boot_rotation", ROOT / "scripts/run-secure-boot-test.py")
fixture = module("management_rotation_credentials", ROOT / "scripts/prepare-management-tls-fixture.py")


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def policy_digest(path):
    data = path.read_bytes()
    if len(data) != 328 or data[:8] != b"CBTRUST1":
        raise ValueError("expected complete current signed fixture policy")
    return hashlib.sha256(b"CharlotteOS bootstrap trust policy v1\0" + data[:264]).hexdigest()


def commit_fixture_snapshot(path, candidate):
    """Commit already verified PUBLIC fixture state before building its kernel."""
    pending = path.with_suffix(".pending")
    with os.fdopen(os.open(pending, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "wb") as output:
        output.write(candidate.read_bytes())
        output.flush()
        os.fsync(output.fileno())
    os.replace(pending, path)
    descriptor = os.open(path.parent, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)
    if path.read_bytes() != candidate.read_bytes():
        raise RuntimeError("fixture installed-state readback mismatch")


def snapshot(text, revision, rejection=None):
    """Require a specific terminal cause or an exact live installed revision."""
    published = "[boot trust] admission policy published: revision="
    steady = "[launch] steady-state service set published."
    if rejection:
        cause = ("[launch] management TLS resource rejected before trust publication: WrongPolicy"
                 if rejection == "WrongPolicy" else f"[boot trust fixture] verification rejected: {rejection}")
        if "Kernel panic:" not in text:
            return False
        if not any(line.endswith(cause) for line in text.splitlines()) or published in text or steady in text:
            raise secure.TestFailure("rejection was unrelated or followed operational publication")
        if rejection == "WrongPolicy" and "boot admission trust rejected" not in text.splitlines():
            raise secure.TestFailure("resource rejection did not reach its terminal boot gate")
        if rejection != "WrongPolicy" and any(marker in text for marker in (
                "[boot trust fixture] verified signed module:", "[boot trust] artifact authority selected before platform services")):
            raise secure.TestFailure("uninstalled/rollback module passed the boot gate")
        return True
    if "Kernel panic:" in text:
        raise secure.TestFailure("positive rotation boot panicked")
    if "[net] startup rejected " in text:
        raise secure.TestFailure("network driver rejected the fixture hardware configuration")
    result = re.search(r"SELFTEST COMPLETE: passed=(\d+) failed=0 pending=0 passed_bitmap=(0x[0-9a-f]+) failed_bitmap=0x0 pending_bitmap=0x0\r?\n", text)
    if re.search(r"SELFTEST COMPLETE:[^\n]*\n", text) and not result:
        raise secure.TestFailure("authoritative kernel suite failed or remained pending")
    if not result or "[time] synchronized " not in text or "[deployd] listening port=7444" not in text:
        return False
    if int(result[1]) == 0 or int(result[2], 16).bit_count() != int(result[1]):
        raise secure.TestFailure("invalid authoritative self-test bitmap")
    if text.count(published) != 1 or f"{published}{revision}\n" not in text:
        raise secure.TestFailure("boot did not publish the exact compiled installed revision")
    if f"[boot trust fixture] verified signed module: revision={revision}\n" not in text or steady not in text:
        raise secure.TestFailure("boot did not use the signed module and steady-state handoff")
    if "[boot trust] signed handoff" not in text or text.index(published) > text.index(steady):
        raise secure.TestFailure("shared policy handoff/order was not verified")
    return True


def wait_for(child, serial, revision, rejection, end):
    while time.monotonic() < end:
        if serial.exists() and snapshot(serial.read_text(), revision, rejection):
            return
        if child.poll() is not None:
            raise secure.TestFailure("QEMU exited before the specific rotation observation")
        time.sleep(0.1)  # The fixture serial file supplies no readiness subscription.
    raise secure.TestFailure("rotation observation deadline expired")


@contextmanager
def guest(arguments, log):
    with log.open("wb") as errors:
        child = subprocess.Popen(arguments, cwd=ROOT, stdin=subprocess.DEVNULL, stdout=errors, stderr=errors)
        try:
            yield child
        finally:
            if child.poll() is None:
                child.terminate()
            try:
                child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()


def context(ca, directory, name):
    result = ssl.create_default_context(cafile=ca)
    result.minimum_version = result.maximum_version = ssl.TLSVersion.TLSv1_3
    result.set_alpn_protocols(["http/1.1"])
    result.load_cert_chain(directory / (name + ".pem"), directory / (name + ".key"))
    return result


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
            raise secure.TestFailure("unexpected TLS negotiation")
        client.settimeout(remaining())
        client.sendall(payload)  # Accepted HTTP uncertainty is never retried.
        data = bytearray()
        while True:
            client.settimeout(remaining())
            received = client.recv(4096)
            if not received:
                raise EOFError("no complete HTTP response")
            data.extend(received)
            if len(data) > 1024 * 1024:
                raise secure.TestFailure("oversized management response")
            if b"\r\n\r\n" not in data:
                continue
            header, body = bytes(data).split(b"\r\n\r\n", 1)
            length = int(next(line.split(b":", 1)[1] for line in header.split(b"\r\n") if line.lower().startswith(b"content-length:")))
            if len(body) < length:
                continue
            code = int(header.split(b" ", 2)[1])
            if code != expected:
                raise secure.TestFailure(f"unexpected HTTP result {code}: {body[:length]!r}")
            observed = {"status": code, "server_sha256": hashlib.sha256(client.getpeercert(binary_form=True)).hexdigest(),
                        "session_reused": client.session_reused, "has_ticket": client.session.has_ticket}
            return observed, client.session


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--firmware-code", type=Path, required=True)
    parser.add_argument("--vars-template", type=Path, required=True)
    parser.add_argument("--tools-dir", type=Path, default=ROOT / "target/secure-boot-tools")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--probe-timeout", type=float, default=12)
    parser.add_argument("--status-port", type=int, default=26443)
    parser.add_argument("--deployment-port", type=int, default=26744)
    args = parser.parse_args()
    if not __debug__ or args.timeout <= 0 or args.probe_timeout <= 0:
        parser.error("enabled runtime checks and positive observation budgets required")
    if not all(1 <= port <= 65535 for port in (args.status_port, args.deployment_port)) or args.status_port == args.deployment_port:
        parser.error("two distinct valid host ports required")
    tools = args.tools_dir.resolve()
    for path in (args.firmware_code, args.vars_template, tools / "limine", tools / "osslsigncode-build/osslsigncode", tools / "venv/bin/virt-fw-vars"):
        if not path.is_file():
            parser.error(f"missing fixture input/tool: {path}")
    if os.environ.get("CATTEN_TRUST_MODE", "development") != "development":
        parser.error("production remains disabled")
    output = (args.output or ROOT / "target/management-rotation-tests" / f"run-{time.time_ns()}").resolve()
    output.mkdir(parents=True, exist_ok=False)
    preparation = output / "preparation.log"
    evidence = {"scope": "x86 Secure Boot public fixture; compiled exact installed snapshots, not protected state/custody",
                "results": [], "commits": [], "firmware_sha256": sha(args.firmware_code)}
    def interrupted(_number, _frame):
        raise KeyboardInterrupt("rotation campaign terminated")
    signal.signal(signal.SIGTERM, interrupted)
    def save():
        (output / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    save()
    with tempfile.TemporaryDirectory(prefix="private.", dir=output) as temporary:
        private = Path(temporary)
        signer = fixture.build_signer()
        def run(*arguments, env=None):
            with preparation.open("ab") as log:
                subprocess.run([str(value) for value in arguments], cwd=ROOT, env=env,
                               stdout=log, stderr=log, check=True)
        run(ROOT / "scripts/verify-limine.sh")
        for generation in ("old", "new"):
            fixture.prepare_credentials(private / generation, authority_name="Charlotte rotation " + generation + " CA")
        evidence["credentials_sha256"] = {
            generation: {name: sha(private / generation / (name + ".der"))
                         for name in ("ca", "server", "reader", "administrator")}
            for generation in ("old", "new")}
        save()
        old, new = private / "old", private / "new"
        policy_dir = ROOT / "crates/catten/src/service/admission"
        policies = {7: policy_dir / "test-policy.bin", 8: policy_dir / "test-policy-next.bin"}
        key_program = '''import os,sys
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
seed=bytes([int(sys.argv[1])])*32
public=Ed25519PrivateKey.from_private_bytes(seed).public_key().public_bytes_raw()
with os.fdopen(os.open(sys.argv[2],os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600),'w') as out: out.write((seed+public).hex()+'\\n')
with open(sys.argv[3],'w') as out: out.write(public.hex()+'\\n')
'''
        for seed, name in ((20, "artifact"), (22, "operations"), (31, "bootstrap")):
            run(tools / "venv/bin/python", "-c", key_program, seed, private / (name + ".key"), private / (name + ".pub"))
        for revision in (7, 8):
            (private / f"policy-{revision}.bin").write_bytes(policies[revision].read_bytes()[80:264])
        run(signer, "trust-policy-verify", policies[7], private / "bootstrap.pub", "boot-trust-tests", "installed", 7, policy_digest(policies[7]))
        run(signer, "trust-policy-verify", policies[8], private / "bootstrap.pub", "boot-trust-tests", "installed", 7, policy_digest(policies[7]))
        run(signer, "management-limits-default", private / "limits")
        for revision, directory in ((7, old), (8, new)):
            profile = private / f"profile-{revision}"
            roots = ["root=" + str(old / "ca.der")]
            if revision == 8:
                roots.append("root=" + str(new / "ca.der"))
            run(signer, "management-profile-create", profile, private / "limits", directory / "server-key.der",
                "chain=" + str(directory / "server.der"), *roots,
                "reader=" + str(directory / "reader.der"), "administrator=" + str(directory / "administrator.der"))
            run(signer, "management-resource-seal", private / f"resource-{revision}", private / f"policy-{revision}.bin", private / "operations.key", profile)
        evidence["resources_sha256"] = {str(revision): sha(private / f"resource-{revision}") for revision in (7, 8)}
        save()
        ca = private / "both-ca.pem"
        ca.write_bytes((old / "ca.pem").read_bytes() + (new / "ca.pem").read_bytes())
        for directory in (old, new):
            for name in ("reader", "administrator"):
                run("openssl", "verify", "-purpose", "sslclient", "-CAfile", ca, directory / (name + ".pem"))
        peers = {(generation, name): context(ca, directory, name)
                 for generation, directory in (("old", old), ("new", new)) for name in ("reader", "administrator")}
        old_server_root = context(old / "ca.pem", new, "reader")
        bundle = private / "platform-bundle"
        shutil.copytree(ROOT / "target/embedded-services/x86_64-unknown-none", bundle)
        environment = dict(os.environ)
        for name in ("CATTEN_MANAGEMENT_TLS_RESOURCE", "CATTEN_BOOT_TEST_INSTALLED_POLICY", "CATTEN_CLUSTER_SERVICES", "CATTEN_RESOURCE_POLICY"):
            environment.pop(name, None)
        environment.update(CATTEN_TRUST_MODE="development", CATTEN_X86_64_SERVICE_BUNDLE=str(bundle))
        signing_environment = dict(environment, CLUSTER_SIGN_KEY_FILE=str(private / "artifact.key"))
        run(ROOT / "scripts/sign-service-elfs.sh", bundle, env=signing_environment)
        certificates, variables = secure.prepare_firmware_authority(private, output, args.vars_template, "openssl", tools / "venv/bin/virt-fw-vars", preparation)
        state = private / "installed-policy.snapshot"
        def build(revision, resource_revision):
            kernel = private / f"kernel-{revision}-{resource_revision}"
            build_env = dict(environment, CATTEN_BOOT_TEST_INSTALLED_POLICY=str(state), CATTEN_MANAGEMENT_TLS_RESOURCE=str(private / f"resource-{resource_revision}"))
            run("cargo", "build", "--locked", "-p", "catten", "--target", "target_specs/x86_64-unknown-none-catten.json",
                "--no-default-features", "--features", "acpi,boot_trust_test,virtio_net_test", env=build_env)
            shutil.copyfile(ROOT / "target/x86_64-unknown-none-catten/debug/catten", kernel)
            run("python3", ROOT / "scripts/check-kernel-asm-sections.py", kernel)
            return kernel
        def image(kernel, policy, name):
            config = output / (name + ".conf")
            config.write_bytes(secure.configuration(secure.blake2(kernel), secure.blake2(policy)))
            loader = private / (name + "-unsigned.EFI")
            shutil.copyfile(ROOT / "limine-binary/BOOTX64.EFI", loader)
            run(tools / "limine", "enroll-config", loader, secure.blake2(config))
            # The shared image helper preserves the basename. UEFI removable
            # media must contain the architecture's fixed fallback filename.
            loader_directory = private / (name + "-loader")
            loader_directory.mkdir()
            signed = loader_directory / "BOOTX64.EFI"
            run(tools / "osslsigncode-build/osslsigncode", "sign", "-h", "sha256", "-certs", certificates["db"][1],
                "-key", certificates["db"][0], "-in", loader, "-out", signed)
            run(tools / "osslsigncode-build/osslsigncode", "verify", "-CAfile", certificates["db"][1], "-in", signed)
            disk = private / (name + ".img")
            run("bash", "-c", 'source "$1/scripts/lib/boot-common.sh"; catten_boot_create_uefi_image "$2" "$3" "$4" "$5" "$6" CATOS',
                "rotation-image", ROOT, disk, secure.BOOT_IMAGE_MIB, signed, kernel, config)
            run("mcopy", "-i", disk, policy, "::/boot-policy.bin")
            return disk, signed
        data = private / "data.img"
        run("python3", ROOT / "scripts/make-nvme-image.py", data, bundle)
        status_request = b"GET /metrics HTTP/1.1\r\nHost: localhost\r\n\r\n"
        deployment_request = b"GET /v1/deployments/audit_missing HTTP/1.1\r\nHost: localhost\r\n\r\n"
        mutation = b"POST /v1/deployments HTTP/1.1\r\nHost: localhost\r\nContent-Length: 3584\r\n\r\nx"
        session = None
        def phase(name, kernel, policy, revision, rejection=None, generation=None):
            nonlocal session
            disk, signed = image(kernel, policy, name)
            serial = output / (name + ".serial.log")
            row = {"case": name, "compiled_revision": revision, "module_sha256": sha(policy),
                   "kernel_sha256": sha(kernel), "signed_loader_sha256": sha(signed), "state": "running", "clients": []}
            evidence["results"].append(row); save()
            arguments = ["qemu-system-x86_64", "-machine", "q35,smm=on", "-cpu", "max", "-smp", "4", "-m", "512M",
                         "-global", "driver=cfi.pflash01,property=secure,value=on",
                         "-drive", f"if=pflash,format=raw,unit=0,file={args.firmware_code.resolve()},readonly=on",
                         "-drive", f"if=pflash,format=raw,unit=1,file={variables}",
                         "-drive", f"if=none,file={disk},format=raw,id=esp,readonly=on", "-device", "qemu-xhci,id=xhci",
                         "-device", "usb-storage,bus=xhci.0,drive=esp,bootindex=1", "-drive", f"if=none,file={data},format=raw,id=data0",
                         "-device", "nvme,drive=data0,serial=cat0", "-device", "intel-iommu",
                         "-netdev", f"user,id=net0,hostfwd=tcp:127.0.0.1:{args.status_port}-:443,hostfwd=tcp:127.0.0.1:{args.deployment_port}-:7444",
                         "-device", "virtio-net-pci-non-transitional,netdev=net0,iommu_platform=on,mac=52:54:00:12:34:56",
                         "-display", "none", "-serial", f"file:{serial}", "-no-reboot"]
            print("START " + name, flush=True)
            try:
                with guest(arguments, output / (name + ".qemu.log")) as child:
                    wait_for(child, serial, revision, rejection, time.monotonic() + args.timeout)
                    if generation:
                        for port, payload, code in ((args.status_port, status_request, 200), (args.deployment_port, deployment_request, 404)):
                            for role in ("reader", "administrator"):
                                result, saved = exchange(peers[generation, role], port, payload, code, args.probe_timeout)
                                if result["server_sha256"] != evidence["credentials_sha256"][generation]["server"]:
                                    raise secure.TestFailure("wrong generation's server certificate")
                                if result["session_reused"] or result["has_ticket"]:
                                    raise secure.TestFailure("session resumption/tickets unexpectedly enabled")
                                row["clients"].append({"generation": generation, "role": role, "port": port, **result})
                                if name == "revision-7" and port == args.status_port and role == "reader":
                                    session = saved
                        exchange(peers[generation, "reader"], args.deployment_port, mutation, 403, args.probe_timeout)
                    if name == "revision-8":
                        for port, payload in ((args.status_port, status_request), (args.deployment_port, deployment_request)):
                            for role in ("reader", "administrator"):
                                before = serial.read_text().count("client policy rejected")
                                try:
                                    exchange(peers["old", role], port, payload, 200, args.probe_timeout,
                                             session=session if role == "reader" and port == args.status_port else None)
                                except ssl.SSLCertVerificationError:
                                    raise secure.TestFailure("server verification failure masked client revocation")
                                except (ssl.SSLError, ConnectionError, EOFError):
                                    if serial.read_text().count("client policy rejected") <= before:
                                        raise secure.TestFailure("revocation lacked the native client-policy rejection event")
                                else:
                                    raise secure.TestFailure("revoked identity received management data")
                                row["clients"].append({"generation": "old", "role": role, "port": port, "client_policy_rejected": True})
                                exchange(peers["new", "reader"], port, payload, 200 if port == args.status_port else 404, args.probe_timeout)
                        try:
                            exchange(old_server_root, args.status_port, status_request, 200, args.probe_timeout)
                        except ssl.SSLCertVerificationError:
                            row["old_server_root_rejected"] = True
                        else:
                            raise secure.TestFailure("retired server trust root accepted replacement")
                    row["observations"] = [line for line in serial.read_text().splitlines() if any(value in line for value in (
                        "verified signed module:", "admission policy published:", "SELFTEST COMPLETE:", "verification rejected:",
                        "management TLS resource rejected before trust publication:", "client policy rejected", "[management] connection=", "boot admission trust rejected"))]
                row["state"] = "observed-limitation" if name == "retired-kernel-replay" else "passed"
            except BaseException as error:
                row.update(state="failed", diagnostic=f"{type(error).__name__}: {error}")
                if serial.exists():
                    row["failure_tail"] = serial.read_text().splitlines()[-25:]
                save(); raise
            save()
            print(("OBSERVED LIMITATION " if row["state"] == "observed-limitation" else "PASS ") + name, flush=True)
        commit_fixture_snapshot(state, policies[7])
        evidence["commits"].append({"revision": 7, "snapshot_sha256": sha(state), "signature_digest": policy_digest(state), "kind": "public host fixture commit before build"}); save()
        first = build(7, 7)
        phase("revision-7", first, policies[7], 7, generation="old")
        phase("uninstalled-successor", first, policies[8], 7, rejection="UninstalledRevision")
        commit_fixture_snapshot(state, policies[8])
        evidence["commits"].append({"revision": 8, "snapshot_sha256": sha(state), "signature_digest": policy_digest(state), "kind": "verified successor; public host fixture commit before build"}); save()
        second = build(8, 8)
        phase("revision-8", second, policies[8], 8, generation="new")
        stale_resource = build(8, 7)
        phase("retired-resource", stale_resource, policies[8], 8, rejection="WrongPolicy")
        phase("retired-module", second, policies[7], 8, rejection="RevisionRollback")
        phase("retired-kernel-replay", first, policies[7], 7, generation="old")
        evidence["platform_rollback_gap_observed"] = True
        evidence["resources_sha256"] = {str(revision): sha(private / f"resource-{revision}") for revision in (7, 8)}
        evidence["platform_bundle_sha256"] = {path.name: sha(path) for path in sorted(bundle.glob("*.elf"))}
        save()
    print(f"Evidence: {output / 'evidence.json'}", flush=True)


if __name__ == "__main__":
    main()
