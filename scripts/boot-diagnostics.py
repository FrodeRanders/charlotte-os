#!/usr/bin/env python3
"""Per-run, bounded QEMU failure capture. Diagnostic state is never a cleanup proof."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import tempfile
import time


MARKERS = ((b"Kernel panic:", "kernel-panic"),
           (b"SELFTEST FAILED:", "selftest-failed"),
           (b"[raft-store] FATAL:", "raft-persistence-fail-stop"))
DIAGNOSTIC_SYMBOLS = {"TIMER_DIAGNOSTICS", "WAKER_DIAGNOSTICS",
                      "SCHEDULER_LIFECYCLE_PROGRESS", "SCHEDULER_LP_DIAGNOSTICS"}


def digest(path):
    value = hashlib.sha256()
    with Path(path).open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def write_json(path, value):
    path = Path(path)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2) + "\n")
    temporary.replace(path)


def failure_reason(data):
    for marker, reason in MARKERS:
        if marker in data:
            return reason
    # Use the runner's exact complete-result grammar, including bitmaps. A
    # partial UART line is neither a terminal result nor proof of success.
    import re
    pattern = rb"SELFTEST COMPLETE: passed=\d+ failed=(\d+) pending=(\d+) passed_bitmap=0x[0-9a-f]+ failed_bitmap=0x([0-9a-f]+) pending_bitmap=0x([0-9a-f]+)\r?\n"
    for result in re.finditer(pattern, data):
        if any(int(result[i], base) for i, base in ((1, 10), (2, 10), (3, 16), (4, 16))):
            return "unsuccessful-selftest-result"
    return None


class Qmp:
    """One private QMP connection, with absolute per-command I/O budgets."""

    def __init__(self, path):
        self.socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.socket.settimeout(2)
        self.socket.connect(str(path))
        self.buffer = b""
        self.serial = 0
        self.receive(time.monotonic() + 2)
        self.command("qmp_capabilities")

    def receive(self, deadline):
        if time.monotonic() >= deadline:
            raise TimeoutError("QMP response deadline")
        while b"\n" not in self.buffer:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("QMP response deadline")
            self.socket.settimeout(remaining)
            block = self.socket.recv(65536)
            if not block:
                raise ConnectionError("QMP disconnected")
            self.buffer += block
        line, self.buffer = self.buffer.split(b"\n", 1)
        return json.loads(line)

    def command(self, execute, arguments=None):
        self.serial += 1
        request = {"execute": execute, "id": self.serial}
        if arguments is not None:
            request["arguments"] = arguments
        deadline = time.monotonic() + 2
        self.socket.settimeout(2)
        self.socket.sendall(json.dumps(request).encode() + b"\n")
        while True:
            response = self.receive(deadline)
            if response.get("id") != self.serial:
                continue
            if "error" in response:
                raise RuntimeError(response["error"])
            return response["return"]

    def close(self):
        self.socket.close()


def prepare(args):
    output = Path(args.root).resolve() / "target/boot-diagnostics"
    output.mkdir(parents=True, exist_ok=True)
    directory = Path(tempfile.mkdtemp(prefix=args.arch + "-", dir=output))
    transport = None
    try:
        manifest = prepare_inputs(args, directory)
        # macOS Unix sockets have a small fixed path bound; do not inherit
        # potentially long repository/instance names in the transport path.
        transport = Path(tempfile.mkdtemp(prefix="ctdiag-", dir="/tmp"))
        manifest["transport"] = str(transport)
        write_json(directory / "manifest.json", manifest)
    except BaseException:
        if transport is not None:
            shutil.rmtree(transport)
        shutil.rmtree(directory)
        raise
    print(directory)


def prepare_inputs(args, directory):
    root = Path(args.root).resolve()
    # Inspect the actual boot-image payload, not a build output which another
    # build could replace before a delayed debugger attach.
    subprocess.run(["mcopy", "-i", args.boot_image, "::/catten", str(directory / "kernel.elf")],
                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True)
    kernel_hash = digest(directory / "kernel.elf")
    expected = Path(args.log + ".kernel.sha256").read_text().strip()
    if kernel_hash != expected:
        raise ValueError("kernel changed after runner payload identification")
    symbols = Path(args.log + ".symbols")
    if symbols.is_file():
        if "# kernel-sha256: " + kernel_hash not in symbols.read_text().splitlines():
            raise ValueError("symbol map does not identify the captured kernel")
        shutil.copyfile(symbols, directory / "kernel.symbols")
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
    changes = subprocess.check_output(["git", "status", "--porcelain"], cwd=root, text=True)
    patch = subprocess.check_output(["git", "diff", "HEAD", "--"], cwd=root)
    bundle = {p.name: digest(p) for p in sorted(Path(args.bundle).glob("*.elf"))}
    manifest = {
        "schema": "charlotte.boot-diagnostics.v1", "arch": args.arch,
        "instance": args.instance, "cpus": args.cpus, "revision": revision,
        "dirty_paths": changes.splitlines(), "tracked_diff_sha256": hashlib.sha256(patch).hexdigest(),
        "kernel_source": str(Path(args.kernel).resolve()),
        "kernel_sha256": kernel_hash, "bundle_sha256": bundle,
        "boot_image": str(Path(args.boot_image).resolve()), "boot_image_sha256": digest(args.boot_image),
        "disk": str(Path(args.disk).resolve()), "initial_disk_sha256": digest(args.disk),
        "serial_log": str(Path(args.log).resolve()), "timeout_seconds": args.timeout,
        "prepared_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "tooling_sha256": {name: digest(root / name) for name in (
            "scripts/boot-diagnostics.py", "scripts/lib/boot-common.sh", f"scripts/run-{args.arch}.sh")},
    }
    return manifest


def diagnostic_reads(directory):
    """Read symbol-declared sizes, rather than assuming a four-LP array layout."""
    symbols = directory / "kernel.symbols"
    commands = []
    if symbols.is_file():
        for line in symbols.read_text().splitlines():
            fields = line.split()
            if len(fields) != 4 or fields[3] not in DIAGNOSTIC_SYMBOLS:
                continue
            address, size = int(fields[0], 16), int(fields[1], 16)
            if size <= 0 or size > 65536 or size % 8:
                continue
            commands.extend([f"image lookup --address 0x{address:x}",
                             f"memory read --force --format x --size 8 --count {size // 8} 0x{address:x}"])
    return commands


def archive_serial(directory, manifest, name):
    source = Path(manifest["serial_log"])
    if source.is_file():
        shutil.copyfile(source, directory / name)


def capture(directory, manifest, reason):
    result = {"reason": reason, "errors": [], "qmp": None, "lldb": None}
    archive_serial(directory, manifest, "serial-at-trigger.log")
    qmp = None
    was_running = None
    cpus = []
    try:
        qmp = Qmp(Path(manifest["transport"]) / "qmp.sock")
        was_running = qmp.command("query-status")["running"]
        qmp.command("stop")
        cpus = qmp.command("query-cpus-fast")
        write_json(directory / "cpus.json", cpus)
        registers = qmp.command("human-monitor-command", {"command-line": "info registers -a"})
        (directory / "registers.log").write_text(registers)
        result["qmp"] = "captured"
    except (OSError, ValueError, RuntimeError) as error:
        result["errors"].append("QMP: " + str(error))
    try:
        debugger = shutil.which("lldb")
        if debugger is None:
            result["lldb"] = "unavailable"
        else:
            endpoint = Path(manifest["transport"]) / "gdb.sock"
            commands = ["settings set interpreter.stop-command-source-on-error false",
                        f"process connect --plugin gdb-remote unix-connect://{endpoint}",
                        "thread list", "thread backtrace all"]
            for index in range(1, (len(cpus) or manifest["cpus"]) + 1):
                commands.extend([f"thread select {index}", "register read --all"])
            commands.extend(diagnostic_reads(directory))
            commands.append("process detach")
            command = [debugger, "--batch", "--file", str(directory / "kernel.elf")]
            for item in commands:
                command.extend(["-o", item])
            with (directory / "lldb.log").open("w") as output:
                completed = subprocess.run(command, stdout=output, stderr=subprocess.STDOUT, timeout=15, check=False)
            result["lldb"] = {"exit_code": completed.returncode,
                              "reported_errors": "error:" in (directory / "lldb.log").read_text()}
    except (OSError, ValueError, subprocess.TimeoutExpired) as error:
        result["errors"].append("LLDB: " + str(error))
    finally:
        # A debugger detach can resume CPUs. Restore the pre-capture run state;
        # no stop/wake observation is a kernel quiescence or cleanup receipt.
        if qmp is not None:
            try:
                if was_running is not None:
                    qmp.command("cont" if was_running else "stop")
            except (OSError, RuntimeError, ValueError) as error:
                result["errors"].append("QMP restore: " + str(error))
            qmp.close()
    result["cpu_count"] = len(cpus)
    result["snapshot"] = "captured" if (
        len(cpus) == manifest["cpus"] and result["qmp"] == "captured"
        and result["lldb"] == {"exit_code": 0, "reported_errors": False}
        and not result["errors"]
    ) else "partial"
    return result


def alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False


def watch(args):
    def interrupted(_signal, _frame):
        raise SystemExit("diagnostic watcher interrupted")

    signal.signal(signal.SIGTERM, interrupted)
    directory = Path(args.directory)
    manifest = json.loads((directory / "manifest.json").read_text())
    manifest["qemu_pid"] = args.pid
    manifest["qemu_arguments"] = args.qemu[1:] if args.qemu[:1] == ["--"] else args.qemu
    write_json(directory / "manifest.json", manifest)
    started = time.monotonic()
    result = {"reason": "watcher-error", "errors": []}
    try:
        while True:
            log = Path(manifest["serial_log"])
            data = log.read_bytes() if log.exists() else b""
            reason = failure_reason(data)
            elapsed = time.monotonic() - started
            if reason is None and elapsed >= manifest["timeout_seconds"]:
                reason = "deadline"
            request_path = directory / "request.json"
            request = json.loads(request_path.read_text()) if request_path.exists() else None
            if reason is not None:
                write_json(directory / "failure.json", {"reason": reason, "elapsed_seconds": elapsed})
            elif request is not None:
                reason = request["reason"]
                if reason == "complete" and not request["snapshot"]:
                    result = {"reason": "complete", "snapshot": "not-requested"}
                    break
                if reason != "complete":
                    write_json(directory / "failure.json", {"reason": reason, "elapsed_seconds": elapsed})
            elif not alive(args.pid):
                result = {"reason": "guest-exited", "snapshot": "unavailable"}
                break
            else:
                # Bounded host fixture observation, independent of synchronous
                # HTTP probes in the runner; not a guest resource wait.
                time.sleep(0.1)
                continue
            result = capture(directory, manifest, reason)
            result["elapsed_seconds"] = elapsed
            break
    except Exception as error:
        result = {"reason": "watcher-error", "errors": [repr(error)]}
        raise
    finally:
        archive_serial(directory, manifest, "serial.log")
        write_json(directory / "result.json", result)


def finish(args):
    directory = Path(args.directory)
    manifest = json.loads((directory / "manifest.json").read_text())
    write_json(directory / "request.json", {"reason": args.reason, "snapshot": args.snapshot})
    deadline = time.monotonic() + 35
    while not (directory / "result.json").exists() and alive(args.watcher):
        if time.monotonic() >= deadline:
            os.kill(args.watcher, signal.SIGTERM)
            write_json(directory / "capture-timeout.json", {"error": "watcher did not finish within 35 seconds"})
            break
        time.sleep(0.1)
    if not (directory / "result.json").exists():
        write_json(directory / "result.json", {
            "reason": "watcher-unavailable", "snapshot": "unavailable",
            "errors": ["watcher ended or exceeded its finalization budget without a result"],
        })
    archive(argparse.Namespace(directory=str(directory)))
    transport = Path(manifest["transport"])
    for name in ("qmp.sock", "gdb.sock"):
        (transport / name).unlink(missing_ok=True)
    transport.rmdir()


def archive(args):
    """Refresh once more after QEMU has exited, retaining its last UART bytes."""
    directory = Path(args.directory)
    manifest = json.loads((directory / "manifest.json").read_text())
    archive_serial(directory, manifest, "serial.log")
    write_json(directory / "artifacts.json", {
        path.name: digest(path) for path in sorted(directory.iterdir())
        if path.is_file() and path.name != "artifacts.json" and not path.name.endswith(".tmp")
    })


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    prepare_parser = commands.add_parser("prepare")
    for name in ("root", "kernel", "log", "arch", "instance", "bundle", "disk", "boot-image"):
        prepare_parser.add_argument("--" + name, required=True)
    prepare_parser.add_argument("--cpus", type=int, required=True)
    prepare_parser.add_argument("--timeout", type=int, required=True)
    watch_parser = commands.add_parser("watch")
    watch_parser.add_argument("--directory", required=True)
    watch_parser.add_argument("--pid", type=int, required=True)
    watch_parser.add_argument("qemu", nargs=argparse.REMAINDER)
    finish_parser = commands.add_parser("finish")
    finish_parser.add_argument("--directory", required=True)
    finish_parser.add_argument("--watcher", type=int, required=True)
    finish_parser.add_argument("--reason", required=True)
    finish_parser.add_argument("--snapshot", action="store_true")
    archive_parser = commands.add_parser("archive")
    archive_parser.add_argument("--directory", required=True)
    args = parser.parse_args()
    {"prepare": prepare, "watch": watch, "finish": finish, "archive": archive}[args.command](args)


if __name__ == "__main__":
    main()
