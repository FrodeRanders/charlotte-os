#!/usr/bin/env python3
"""Disposable x86 QEMU Secure Boot tests; never production enrollment/custody."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time


ROOT = Path(__file__).resolve().parents[1]
ANSI = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")
COMPLETE = re.compile(
    r"SELFTEST COMPLETE: passed=15 failed=0 pending=0 "
    r"passed_bitmap=0x1bbff failed_bitmap=0x0 pending_bitmap=0x0\r?\n"
)
CASES = {
    "valid-intel": "success",
    "valid-amd": "success",
    "unsigned-loader": "firmware",
    "foreign-loader": "firmware",
    "tampered-loader": "firmware",
    "config-tamper": "config",
    "kernel-tamper": "kernel-hash",
    "policy-tamper": "policy-hash",
    "policy-signature": "InvalidSignature",
    "policy-rollback": "RevisionRollback",
    "policy-conflict": "RevisionConflict",
    "policy-next": "uninstalled policy revision",
    "missing-module": "policy module",
    "duplicate-module": "exactly one policy module",
}


class TestFailure(RuntimeError):
    pass


def observed_result(raw, expected):
    """Return true only for complete, specific evidence; timeout is no result."""
    text = ANSI.sub("", raw.decode("utf-8", errors="replace"))
    if expected == "success":
        if "Kernel panic:" in text or "PANIC" in text:
            raise TestFailure("positive boot panicked")
        if COMPLETE.search(text):
            if "[boot trust fixture] verified signed module: revision=7" not in text:
                raise TestFailure("kernel suite did not use the signed policy module")
            if "[boot trust] signed handoff" not in text:
                raise TestFailure("boot trust assertions did not run")
            return True
        return False
    if expected in ("firmware", "config", "kernel-hash", "policy-hash"):
        if "Catten Kernel Version" in text:
            raise TestFailure("rejected boot input reached kernel entry")
        if expected == "firmware":
            return bool(re.search(r"BdsDxe: failed to load Boot[0-9A-Fa-f]+ .*USB.*: (Security Violation|Access Denied)", text))
        if expected == "config":
            return "CHECKSUM MISMATCH FOR CONFIG FILE" in text
        resource = "/catten" if expected == "kernel-hash" else "/boot-policy.bin"
        return "PANIC" in text and "Blake2b hash for URI" in text and resource in text and "does not match" in text
    if "[launch] steady-state service set published." in text or "[boot trust fixture] verified signed module" in text:
        raise TestFailure("unaccepted policy reached trust/service publication")
    if "Kernel panic:" in text:
        if "[boot trust fixture]" not in text or expected not in text:
            raise TestFailure("kernel panicked for an unrelated reason")
        return True
    return False


def blake2(path):
    digest = hashlib.blake2b()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def configuration(kernel_hash, policy_hash, modules=1):
    data = (
        "TIMEOUT: 0\nSERIAL: yes\nHASH_MISMATCH_PANIC: yes\nEDITOR_ENABLED: no\n"
        "/Catten boot trust fixture\n PROTOCOL: limine\n"
        f" KERNEL_PATH: boot():/catten#{kernel_hash}\n KASLR: no\n"
    )
    for _ in range(modules):
        data += f" MODULE_PATH: boot():/boot-policy.bin#{policy_hash}\n MODULE_STRING: charlotte.boot-trust-test\n"
    return data.encode("ascii")


def run(command, log):
    with log.open("ab") as output:
        subprocess.run([str(x) for x in command], cwd=ROOT, check=True, stdout=output, stderr=output)


def boot(command, serial, stderr, expected, timeout):
    with stderr.open("wb") as errors:
        child = subprocess.Popen(command, cwd=ROOT, stdin=subprocess.DEVNULL,
                                 stdout=errors, stderr=errors)
        try:
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                if serial.exists() and observed_result(serial.read_bytes(), expected):
                    return
                if child.poll() is not None:
                    raise TestFailure(f"QEMU exited before authoritative {expected} evidence")
                time.sleep(0.2)
            raise TestFailure(f"no authoritative {expected} evidence within {timeout}s")
        finally:
            # This owner always reaps its own VM, including failed assertions,
            # timeout and interruption. No guest process survives the fixture.
            if child.poll() is None:
                child.terminate()
            try:
                child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tools-dir", type=Path, default=ROOT / "target/secure-boot-tools")
    parser.add_argument("--firmware-code", type=Path, required=True)
    parser.add_argument("--vars-template", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--timeout", type=int, default=90)
    parser.add_argument("--cases", default=",".join(CASES))
    parser.add_argument("--qemu", default="qemu-system-x86_64")
    parser.add_argument("--openssl", default="openssl")
    args = parser.parse_args()
    selected = args.cases.split(",")
    if args.timeout <= 0 or not selected or any(case not in CASES for case in selected):
        parser.error("positive timeout and known comma-separated cases required")
    if os.environ.get("CATTEN_TRUST_MODE", "development") != "development":
        parser.error("disposable boot fixture requires development mode; production remains disabled")
    tools = args.tools_dir.resolve()
    signer = tools / "osslsigncode-build/osslsigncode"
    enroll = tools / "limine"
    fwvars = tools / "venv/bin/virt-fw-vars"
    for tool in (signer, enroll, fwvars):
        if not tool.is_file() or not os.access(tool, os.X_OK):
            parser.error(f"missing test tool: {tool}; run scripts/prepare-secure-boot-tools.sh")
    for image in (args.firmware_code, args.vars_template):
        if not image.is_file():
            parser.error(f"missing firmware input: {image}")
    output = (args.output or ROOT / "target/secure-boot-tests" / f"run-{time.time_ns()}").resolve()
    output.mkdir(parents=True, exist_ok=False)
    preparation = output / "preparation.log"
    print(f"DEVELOPMENT QEMU Secure Boot fixture; public test policy/recipient. Evidence: {output}", flush=True)
    run([ROOT / "scripts/verify-limine.sh"], preparation)
    environment = os.environ.copy()
    environment["CATTEN_TRUST_MODE"] = "development"
    environment["CATTEN_X86_64_SERVICE_BUNDLE"] = str(ROOT / "target/embedded-services/x86_64-unknown-none")
    with preparation.open("ab") as log:
        subprocess.run(["cargo", "build", "--locked", "-p", "catten", "--target",
                        "target_specs/x86_64-unknown-none-catten.json", "--no-default-features",
                        "--features", "acpi,boot_trust_test"], cwd=ROOT, env=environment,
                       stdout=log, stderr=log, check=True)
    kernel = ROOT / "target/x86_64-unknown-none-catten/debug/catten"
    run(["python3", ROOT / "scripts/check-kernel-asm-sections.py", kernel], preparation)
    policy_dir = ROOT / "crates/catten/src/service/admission"
    policy = policy_dir / "test-policy.bin"
    results = []
    # All private keys and mutable VM artifacts are owned by this directory.
    # Evidence contains only logs, public certificates/configs and digests.
    with tempfile.TemporaryDirectory(prefix="private.", dir=output) as temporary:
        private = Path(temporary)
        private.chmod(0o700)
        # Cargo may later build another feature set into the same target path.
        # Keep this matrix's exact kernel bytes through signing and every case.
        snapshot = private / "kernel-snapshot"
        shutil.copyfile(kernel, snapshot)
        kernel = snapshot
        run(["python3", ROOT / "scripts/check-kernel-asm-sections.py", kernel], preparation)
        certificates = {}
        for role in ("pk", "kek", "db", "foreign"):
            key, cert = private / f"{role}.key", output / f"{role}.crt"
            run([args.openssl, "req", "-new", "-x509", "-newkey", "rsa:2048", "-nodes",
                 "-sha256", "-days", "2", "-subj", f"/CN=Disposable CharlotteOS QEMU {role}/",
                 "-keyout", key, "-out", cert], preparation)
            key.chmod(0o600)
            certificates[role] = (key, cert)
        enrolled_vars = private / "enrolled-vars.fd"
        owner = "57912751-8c52-4aba-a758-0a53f5c634d7"
        inherited = private / "inherited-vars.fd"
        # Deliberately start with foreign KEK/db authority. Re-enrollment must
        # remove it; the foreign-signed boot case proves it is not retained.
        run([fwvars, "--input", args.vars_template.resolve(), "--output", inherited,
             "--add-kek", owner, certificates["foreign"][1],
             "--add-db", owner, certificates["foreign"][1]], preparation)
        run([fwvars, "--input", inherited, "--output", enrolled_vars,
             "--delete", "PK", "--delete", "KEK", "--delete", "db",
             "--set-pk", owner, certificates["pk"][1], "--add-kek", owner, certificates["kek"][1],
             "--add-db", owner, certificates["db"][1], "--secure-boot"], preparation)
        run([fwvars, "--input", enrolled_vars, "--output-json", output / "enrollment.json"], preparation)
        baseline = configuration(blake2(kernel), blake2(policy))
        config = output / "limine.conf"
        config.write_bytes(baseline)
        enrolled_loader = private / "enrolled.EFI"
        shutil.copyfile(ROOT / "limine-binary/BOOTX64.EFI", enrolled_loader)
        run([enroll, "enroll-config", enrolled_loader, blake2(config)], preparation)
        signed_loader = private / "BOOTX64.EFI"
        run([signer, "sign", "-h", "sha256", "-certs", certificates["db"][1],
             "-key", certificates["db"][0], "-in", enrolled_loader, "-out", signed_loader], preparation)
        run([signer, "verify", "-CAfile", certificates["db"][1], "-in", signed_loader], preparation)
        base_image = private / "base.img"
        run(["bash", "-c", 'source "$1/scripts/lib/boot-common.sh"; catten_boot_create_uefi_image "$2" 64 "$3" "$4" "$5" CATOS',
             "secure-test", ROOT, base_image, signed_loader, kernel, config], preparation)
        run(["mcopy", "-i", base_image, policy, "::/boot-policy.bin"], preparation)
        data_image = private / "data.img"
        for case in selected:
            image, variables = private / "case.img", private / "case-vars.fd"
            shutil.copyfile(base_image, image)
            shutil.copyfile(enrolled_vars, variables)
            case_policy = private / "policy.bin"
            case_policy.write_bytes(policy.read_bytes())
            if case in ("unsigned-loader", "foreign-loader", "tampered-loader"):
                replacement = private / "replacement.EFI"
                if case == "unsigned-loader":
                    shutil.copyfile(enrolled_loader, replacement)
                elif case == "foreign-loader":
                    if replacement.exists():
                        replacement.unlink()
                    run([signer, "sign", "-h", "sha256", "-certs", certificates["foreign"][1],
                         "-key", certificates["foreign"][0], "-in", enrolled_loader, "-out", replacement], preparation)
                else:
                    changed = bytearray(signed_loader.read_bytes())
                    offset = changed.index(b"++CONFIG_B2SUM_SIGNATURE++") + len(b"++CONFIG_B2SUM_SIGNATURE++")
                    changed[offset] = ord("1") if changed[offset] != ord("1") else ord("2")
                    replacement.write_bytes(changed)
                run(["mcopy", "-o", "-i", image, replacement, "::/EFI/BOOT/BOOTX64.EFI"], preparation)
            elif case == "config-tamper":
                changed = private / "limine.conf"
                changed.write_bytes(baseline + b"# unauthorized config substitution\n")
                run(["mcopy", "-o", "-i", image, changed, "::/limine.conf"], preparation)
            elif case == "kernel-tamper":
                changed = private / "catten"
                shutil.copyfile(kernel, changed)
                with changed.open("r+b") as stream:
                    stream.seek(-1, 2)
                    byte = stream.read(1)
                    stream.seek(-1, 2)
                    stream.write(bytes([byte[0] ^ 1]))
                run(["mcopy", "-o", "-i", image, changed, "::/catten"], preparation)
            elif case.startswith("policy-") or case.endswith("-module"):
                if case in ("policy-tamper", "policy-signature"):
                    changed = bytearray(case_policy.read_bytes())
                    changed[-1] ^= 1
                    case_policy.write_bytes(changed)
                elif case in ("policy-rollback", "policy-conflict", "policy-next"):
                    name = "previous" if case == "policy-rollback" else case.removeprefix("policy-")
                    shutil.copyfile(policy_dir / f"test-policy-{name}.bin", case_policy)
                run(["mcopy", "-o", "-i", image, case_policy, "::/boot-policy.bin"], preparation)
                if case != "policy-tamper":
                    # Authorized outer packaging of rejected inner policy
                    # proves the kernel's signature/state checks independently.
                    modules = 0 if case == "missing-module" else 2 if case == "duplicate-module" else 1
                    changed_config = private / "authorized.conf"
                    changed_config.write_bytes(configuration(blake2(kernel), blake2(case_policy), modules))
                    unsigned = private / "authorized.EFI"
                    shutil.copyfile(ROOT / "limine-binary/BOOTX64.EFI", unsigned)
                    run([enroll, "enroll-config", unsigned, blake2(changed_config)], preparation)
                    signed = private / "authorized-signed.EFI"
                    if signed.exists():
                        signed.unlink()
                    run([signer, "sign", "-h", "sha256", "-certs", certificates["db"][1],
                         "-key", certificates["db"][0], "-in", unsigned, "-out", signed], preparation)
                    run(["mcopy", "-o", "-i", image, signed, "::/EFI/BOOT/BOOTX64.EFI"], preparation)
                    run(["mcopy", "-o", "-i", image, changed_config, "::/limine.conf"], preparation)
            if case.startswith("valid-") or not data_image.exists():
                run(["python3", ROOT / "scripts/make-nvme-image.py", data_image,
                     ROOT / "target/embedded-services/x86_64-unknown-none"], preparation)
            qemu = [args.qemu, "-machine", "q35,smm=on", "-cpu", "max", "-smp", "4", "-m", "512M",
                    "-global", "driver=cfi.pflash01,property=secure,value=on",
                    "-drive", f"if=pflash,format=raw,unit=0,file={args.firmware_code.resolve()},readonly=on",
                    "-drive", f"if=pflash,format=raw,unit=1,file={variables}",
                    "-drive", f"if=none,file={image},format=raw,id=esp,readonly=on",
                    "-device", "qemu-xhci,id=xhci", "-device", "usb-storage,bus=xhci.0,drive=esp,bootindex=1",
                    "-drive", f"if=none,file={data_image},format=raw,id=data0", "-device", "nvme,drive=data0,serial=cat0",
                    "-device", "amd-iommu,dma-remap=on" if case == "valid-amd" else "intel-iommu",
                    "-display", "none", "-serial", f"file:{output / (case + '.serial.log')}", "-nic", "none", "-no-reboot"]
            boot(qemu, output / f"{case}.serial.log", output / f"{case}.qemu.log", CASES[case], args.timeout)
            results.append({"case": case, "evidence": CASES[case], "passed": True})
            print(f"PASS {case}: {CASES[case]}", flush=True)
        summary = {"test_only": True, "firmware_sha256": hashlib.sha256(args.firmware_code.read_bytes()).hexdigest(),
                   "kernel_blake2b": blake2(kernel), "policy_blake2b": blake2(policy), "config_blake2b": blake2(config),
                   "results": results}
        (output / "results.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(f"All {len(results)} Secure Boot cases passed; private keys and VM artifacts removed.", flush=True)


if __name__ == "__main__":
    try:
        main()
    except (TestFailure, subprocess.CalledProcessError) as error:
        raise SystemExit(f"Secure Boot fixture failed: {error}") from error
