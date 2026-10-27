#!/usr/bin/env python3
"""Three native ARM nodes: verified management, signed mutation and leader loss.

Public development trust only. One QEMU hub bridge supplies ordinary DHCP/NAT
and loopback forwards to the shared L2. TLS terminates inside each real service.
"""
import argparse
from contextlib import ExitStack, contextmanager
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import signal
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


fixture = module("management_credentials", ROOT / "scripts/prepare-management-tls-fixture.py")
startup = module("management_startup", ROOT / "scripts/run-management-startup-tests.py")
tls = module("management_tls", ROOT / "scripts/tests/management_tls.py")


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


@contextmanager
def process(command, capture):
    with capture.open("wb") as output:
        child = subprocess.Popen([str(value) for value in command], cwd=ROOT, stdin=subprocess.DEVNULL,
                                 stdout=output, stderr=output, start_new_session=True)
        try:
            yield child
        finally:
            if child.poll() is None:
                child.terminate()
            try:
                child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait()


def wait_for(test, children, end, description):
    while time.monotonic() < end:
        if any(child.poll() is not None for child in children):
            raise RuntimeError("fixture process exited while waiting for " + description)
        value = test()
        if value:
            return value
        # Host serial files expose no readiness source. This bounded observer
        # does not replace any native service resource/CQ wait.
        time.sleep(min(0.1, max(0, end - time.monotonic())))
    raise TimeoutError("deadline waiting for " + description)


def context(directory, name):
    value = ssl.create_default_context(cafile=directory / "ca.pem")
    value.minimum_version = value.maximum_version = ssl.TLSVersion.TLSv1_3
    value.set_alpn_protocols(["http/1.1"])
    if name:
        value.load_cert_chain(directory / (name + ".pem"), directory / (name + ".key"))
    return value


def topology(snapshots, expected_members, old_leader=None, previous_term=0):
    """Require one fresh committed term/leader and exact active membership."""
    if not snapshots:
        return None
    leaders = set()
    membership = None
    for value in snapshots.values():
        if not isinstance(value, dict) or value.get("schema") != "charlotte.cluster.v2" or value.get("truncated") or not value.get("fresh_committed"):
            return None
        raft = value["raft"]
        if raft["term"] <= previous_term or not raft["leader"] or raft["leader"] == old_leader:
            return None
        members = {node["node_key"] for node in value["nodes"] if node["member"]}
        if len(members) != expected_members:
            return None
        if membership is not None and members != membership:
            return None
        membership = members
        leaders.add((raft["leader"], raft["term"]))
    if len(leaders) != 1:
        return None
    leader, term = next(iter(leaders))
    owners = [name for name, value in snapshots.items() if value["served_by_leader"]]
    if len(owners) != 1 or snapshots[owners[0]]["raft"]["served_by"] != leader:
        return None
    return owners[0], leader, term


def ingress_projection(snapshots, expected_nodes):
    """Observe complete agreeing projections, never infer liveness from silence."""
    targets = {("10.0.2.42", 443, 6), ("10.0.2.42", 7444, 6)}
    observed = {}
    if not snapshots or not expected_nodes:
        return None
    for name, snapshot in snapshots.items():
        if not isinstance(snapshot, dict) or not isinstance(snapshot.get("ingress"), list):
            return None
        rows = snapshot["ingress"]
        if len(rows) != len(targets):
            return None
        current = {}
        for row in rows:
            if not isinstance(row, dict):
                return None
            key = (row.get("vip"), row.get("port"), row.get("protocol"))
            if key not in targets or key in current or row.get("projection_present") is not True or row.get("backend_name") is not None:
                return None
            if row.get("member_count") != 3 or row.get("eligible_nodes") != sorted(expected_nodes) or type(row.get("epoch")) is not int or row.get("advertiser_node") not in expected_nodes:
                return None
            current[key] = (row["epoch"], row["advertiser_node"])
        if observed and current != next(iter(observed.values())):
            return None
        observed[name] = current
    return observed


@contextmanager
def journal(output, evidence):
    try:
        yield
    except BaseException as error:
        evidence.update(state="failed", diagnostic=f"{type(error).__name__}: {error}")
        evidence["failure_observations"] = {path.name: path.read_text(errors="replace").splitlines()[-35:]
                                            for path in output.glob("*.serial.log")}
        (output / "evidence.json").write_text(json.dumps(evidence, indent=2)+"\n")
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--firmware", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--reuse-bundle", action="store_true")
    parser.add_argument("--phase-timeout", type=float, default=150)
    parser.add_argument("--probe-timeout", type=float, default=12)
    parser.add_argument("--build-timeout", type=int, default=900)
    parser.add_argument("--port-base", type=int, default=28440)
    parser.add_argument("--smp", type=int, default=4)
    parser.add_argument("--direct-only", action="store_true", help="qualify direct-node failover; explicitly leave VIP reconnection unqualified")
    parser.add_argument("--trace-tcp", action="store_true", help="capture bounded TCP packet metadata, including data lengths but no payload bytes")
    parser.add_argument("--trace-limit", type=int, default=512)
    args = parser.parse_args()
    if not args.firmware.is_file() or min(args.phase_timeout, args.probe_timeout, args.build_timeout, args.smp, args.trace_limit) <= 0:
        parser.error("firmware and positive fixture budgets required")
    if not 1024 <= args.port_base <= 65526:
        parser.error("port base must leave ten distinct unprivileged ports")
    if os.environ.get("CATTEN_TRUST_MODE", "development") != "development":
        parser.error("production remains disabled")
    def interrupted(_number, _frame):
        raise KeyboardInterrupt("management failover campaign terminated")
    signal.signal(signal.SIGTERM, interrupted)
    output = (args.output or ROOT / "target/management-failover-tests" / f"run-{time.time_ns()}").resolve()
    output.mkdir(parents=True, exist_ok=False)
    evidence = {"scope": "ARM native management failover; public development trust, unauthenticated L2/SNTP",
                "parent_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
                "state": "preparing", "phases": [], "firmware_sha256": sha(args.firmware)}
    def save():
        (output / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    save()
    with tempfile.TemporaryDirectory(prefix="private.", dir=output) as temporary, ExitStack() as owners, journal(output, evidence):
        private = Path(temporary)
        preparation = output / "preparation.log"
        def run(arguments, environment=None):
            if startup.command(arguments, preparation, environment, args.build_timeout):
                raise RuntimeError("fixture preparation command failed")
        if not args.reuse_bundle:
            run([ROOT / "scripts/build-catten-services.sh", "--embed"])
        run([ROOT / "scripts/verify-limine.sh"])
        credentials = private / "credentials"
        fixture.prepare(credentials, addresses=("10.0.2.42",))
        signer = ROOT / "target/debug/cluster-sign"
        environment = dict(os.environ)
        for name in ("CATTEN_BOOT_TEST_INSTALLED_POLICY", "CATTEN_CLUSTER_STATIC_NETWORK", "CATTEN_RESOURCE_POLICY"):
            environment.pop(name, None)
        environment.update(CATTEN_TRUST_MODE="development", CATTEN_CLUSTER_SERVICES="10.0.2.42:443;10.0.2.42:7444",
                           CATTEN_AARCH64_SERVICE_BUNDLE=str(ROOT / "target/embedded-services/aarch64-unknown-none"),
                           CATTEN_MANAGEMENT_TLS_RESOURCE=str(credentials / "resource.bin"))
        run(["cargo", "build", "--locked", "-p", "catten", "--target", "target_specs/aarch64-unknown-none-catten.json",
             "--no-default-features", "--features", "acpi,virtio_net_test"], environment)
        kernel = private / "catten"
        kernel.write_bytes((ROOT / "target/aarch64-unknown-none-catten/debug/catten").read_bytes())
        run(["python3", ROOT / "scripts/check-kernel-asm-sections.py", kernel])
        image = private / "boot.img"
        run(["bash", "-c", 'source "$1/scripts/lib/boot-common.sh"; catten_boot_create_uefi_image "$2" 128 "$1/limine-binary/BOOTAA64.EFI" "$3" "$1/limine.conf" CATOS',
             "management-failover-image", ROOT, image, kernel])
        evidence.update(kernel_sha256=sha(kernel), resource_sha256=sha(credentials / "resource.bin"),
                        service_sha256={p.name: sha(p) for p in sorted((ROOT / "target/embedded-services/aarch64-unknown-none").glob("*.elf"))})
        save()
        reader, administrator, unknown, missing = (context(credentials, name) for name in ("reader", "administrator", "unknown", None))
        hub_port, vip_status, vip_deploy = args.port_base, args.port_base + 7, args.port_base + 8
        trace = ["--trace-tcp", "--trace-tcp-data"] if args.trace_tcp else []
        hub = owners.enter_context(process(["python3", ROOT / "scripts/qemu-stream-l2-hub.py", "--listen", f"127.0.0.1:{hub_port}", "--trace-raft-votes", "--trace-limit", str(args.trace_limit), *trace], output / "hub.log"))
        wait_for(lambda: "QEMU L2 hub listening" in (output / "hub.log").read_text(), [hub], time.monotonic() + 5, "L2 hub")
        forwards = [f"hostfwd=tcp:127.0.0.1:{vip_status}-10.0.2.42:443", f"hostfwd=tcp:127.0.0.1:{vip_deploy}-10.0.2.42:7444"]
        for i in range(3):
            forwards.extend((f"hostfwd=tcp:127.0.0.1:{args.port_base+1+i*2}-10.0.2.{15+i}:443",
                             f"hostfwd=tcp:127.0.0.1:{args.port_base+2+i*2}-10.0.2.{15+i}:7444"))
        bridge = owners.enter_context(process(["qemu-system-aarch64", "-machine", "none", "-nodefaults", "-display", "none", "-monitor", "none", "-serial", "none",
            "-netdev", "user,id=nat," + ",".join(forwards), "-netdev", f"stream,id=lan,server=off,addr.type=inet,addr.host=127.0.0.1,addr.port={hub_port}",
            "-netdev", "hubport,id=nat-port,hubid=0,netdev=nat", "-netdev", "hubport,id=lan-port,hubid=0,netdev=lan"], output / "bridge.log"))
        nodes = {}
        def text(name):
            return (output / (name + ".serial.log")).read_text()
        def children():
            return [hub, bridge, *(row["process"] for row in nodes.values())]
        def health():
            for name in nodes:
                if "Kernel panic:" in text(name) or "FATAL USER FAULT:" in text(name):
                    raise RuntimeError("native failure on " + name)
        def get(port, path, policy=reader, *, observation=False, end=None):
            evidence["current_probe"] = {"port": port, "path": path, "kind": "read", "monotonic": time.monotonic()}; save()
            budget = args.probe_timeout if end is None else min(args.probe_timeout, max(0, end-time.monotonic()))
            try:
                result, _ = tls.exchange(policy, port, f"GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n".encode(), (200, 404, 409, 503), budget, include_body=True)
            except (EOFError, TimeoutError, ConnectionError) as error:
                if not observation:
                    raise
                # A fresh read-only observation may follow an unconfirmed read;
                # record the failure. Never apply this policy to a mutation or
                # a TLS identity-verification failure.
                key = f"{port}:{type(error).__name__}"
                errors = evidence.setdefault("read_observation_failures", {})
                errors[key] = errors.get(key, 0)+1
                save()
                return None
            if not observation and result["status"] != 200:
                raise RuntimeError("healthy management read was unavailable")
            return json.loads(result.pop("body")) if result["status"] == 200 else None
        for index, suffix in enumerate((3, 2, 1)):
            name = "node-" + str(index)
            data = private / (name + "-data.img")
            run(["python3", ROOT / "scripts/make-nvme-image.py", data, ROOT / "target/embedded-services/aarch64-unknown-none"])
            serial = output / (name + ".serial.log")
            serial.touch()
            command = ["qemu-system-aarch64", "-M", "virt,gic-version=3,msi=gicv2m,iommu=smmuv3,default-bus-bypass-iommu=off",
                       "-m", "512M", "-smp", args.smp, "-cpu", "cortex-a710", "-bios", args.firmware.resolve(),
                       "-drive", f"file={image},format=raw,if=none,id=boot0,readonly=on", "-device", "virtio-blk-pci,drive=boot0,addr=3,bootindex=1",
                       "-object", "rng-random,filename=/dev/urandom,id=rng", "-device", "virtio-rng-pci,rng=rng,disable-legacy=on,iommu_platform=on,addr=4",
                       "-drive", f"if=none,file={data},format=raw,id=nvme0", "-device", "nvme,drive=nvme0,serial=cat0,max_ioqpairs=4,addr=2",
                       "-nic", "none", "-netdev", f"stream,id=net,server=off,addr.type=inet,addr.host=127.0.0.1,addr.port={hub_port}",
                       "-device", f"virtio-net-pci,netdev=net,disable-legacy=on,iommu_platform=on,mac=52:54:00:12:34:{suffix:02x},addr=1",
                       "-display", "none", "-serial", f"file:{serial}", "-no-reboot"]
            child = owners.enter_context(process(command, output / (name + ".qemu.log")))
            nodes[name] = {"process": child, "status": args.port_base + 1 + index*2, "deploy": args.port_base + 2 + index*2}
            print("START " + name, flush=True)
            def ready():
                health()
                value = text(name)
                if "[time] synchronized " not in value or "[deployd] listening port=7444" not in value:
                    return False
                if not re.search(r"SELFTEST COMPLETE: passed=18 failed=0 pending=0 passed_bitmap=0x3ffff failed_bitmap=0x0 pending_bitmap=0x0", value):
                    return False
                if "DHCP assigned 10.0.2." + str(15 + index) not in value:
                    raise RuntimeError("DHCP lease did not match the explicitly forwarded fixture address")
                return True
            wait_for(ready, children(), time.monotonic()+args.phase_timeout, name + " live TLS/time/self-test readiness")
            get(nodes[name]["status"], "/metrics")
            if index:
                wait_for(lambda: all(re.search(rf"MEMBERSHIP epoch=.*joint=false members={index+1}", text(n)) for n in nodes),
                         children(), time.monotonic()+args.phase_timeout, "committed voter membership")
        snapshots = {}
        def observe(old=None, term=0, end=None):
            health()
            for name, node in nodes.items():
                snapshots[name] = get(node["status"], "/cluster/metrics", observation=True, end=end)
            return topology(snapshots, 3, old, term)
        end = time.monotonic()+args.phase_timeout
        owner, leader, term = wait_for(lambda: observe(end=end), children(), end, "one committed three-voter leader")
        evidence["phases"].append({"phase": "three-voter-ready", "owner": owner, "leader": leader, "term": term, "snapshots": dict(snapshots)})
        evidence["state"] = "testing"; save()
        follower = next(name for name in nodes if name != owner)
        cluster_id = (credentials / "policy.bin").read_bytes()[24:56].hex()
        def policy_file(sequence):
            path = private / f"ingress-policy-{sequence}"
            now = int(time.time())
            run([signer, "ingress-policy-sign", path, sequence, now-60, now+1800, cluster_id,
                 ROOT / "tools/cluster-sign/dev-operations-key.hex", "10.0.2.42:443", "10.0.2.42:7444"])
            return path
        def mutate(path, port, peer, expected):
            body = path.read_bytes()
            request = f"POST /v1/ingress-policy HTTP/1.1\r\nHost: localhost\r\nContent-Length: {len(body)}\r\n\r\n".encode()+body
            result, _ = tls.exchange(peer, port, request, expected, args.probe_timeout, include_body=True)
            value = json.loads(result.pop("body"))
            if expected == 202 and (type(value.get("generation")) is not int or value["generation"] <= 0):
                raise RuntimeError("invalid mutation receipt; accepted outcome remains uncertain, never replayable")
            return value
        first = policy_file(1)
        denied = mutate(first, nodes[follower]["deploy"], reader, 403)
        receipt = mutate(first, nodes[follower]["deploy"], administrator, 202)
        evidence["phases"].append({"phase": "follower-admission", "follower": follower, "denied": denied, "receipt": receipt, "policy_sha256": sha(first)})
        save()
        def policy_status(sequence, end):
            values = {name: get(node["deploy"], "/v1/ingress-policy", observation=True, end=end) for name, node in nodes.items()}
            return values if all(isinstance(value, dict) and value.get("sequence") == sequence for value in values.values()) else None
        end = time.monotonic()+args.phase_timeout
        committed = wait_for(lambda: policy_status(1, end), children(), end, "first policy on all nodes")
        def projection_barrier(phase, old=None, previous_term=0):
            end = time.monotonic()+args.phase_timeout
            def ready_projection():
                if not observe(old, previous_term, end):
                    return None
                own_rows = [[row["node_key"] for row in value["nodes"] if row.get("self")] for value in snapshots.values()]
                if any(len(rows) != 1 for rows in own_rows):
                    return None
                expected = {rows[0] for rows in own_rows}
                return ingress_projection(snapshots, expected) if len(expected) == len(nodes) else None
            wait_for(ready_projection, children(), end, phase)
            evidence["phases"].append({"phase": phase, "snapshots": dict(snapshots)})
            save()
        projection_barrier("initial-ingress-projection")
        if not args.direct_only:
            get(vip_status, "/cluster/metrics")
        killed = nodes.pop(owner)
        snapshots.pop(owner)
        killed["process"].terminate(); killed["process"].wait(timeout=10)
        print("STOP leader " + owner, flush=True)
        end = time.monotonic()+args.phase_timeout
        new_owner, new_leader, new_term = wait_for(lambda: observe(leader, term, end), children(), end, "replacement leader in the retained three-voter configuration")
        evidence["phases"].append({"phase": "replacement-leader", "old_owner": owner, "owner": new_owner, "leader": new_leader, "term": new_term,
                                  "committed_before": committed, "snapshots": dict(snapshots)})
        save()
        new_follower = next(name for name in nodes if name != new_owner)
        second = policy_file(2)
        mutate(second, nodes[new_follower]["deploy"], reader, 403)
        accepted = mutate(second, nodes[new_follower]["deploy"], administrator, 202)
        end = time.monotonic()+args.phase_timeout
        values = wait_for(lambda: policy_status(2, end), children(), end, "second policy on survivors")
        evidence["phases"].append({"phase": "post-failover-admission", "follower": new_follower, "receipt": accepted, "policy_sha256": sha(second), "policy_status": values,
                                  "previous_receipt": receipt})
        save()
        projection_barrier("survivor-ingress-projection", leader, term)
        def clients(endpoints, phase):
            cases = []
            progress = {"phase": phase, "cases": cases}
            evidence["phases"].append(progress); save()
            for port, path in endpoints:
                progress.update(port=port, path=path, step="healthy-reader-before"); save()
                if not isinstance(get(port, path), dict):
                    raise RuntimeError("healthy reader returned no observation before rejection control")
                for peer, label in ((unknown, "unknown"), (missing, "missing")):
                    case = {"port": port, "client": label, "state": "running"}
                    progress["step"] = "reject-" + label
                    cases.append(case); save()
                    before = sum(text(name).count("client policy rejected") for name in nodes)
                    evidence["current_probe"] = {"port": port, "path": path, "kind": label, "monotonic": time.monotonic()}; save()
                    end = time.monotonic()+args.probe_timeout
                    try:
                        tls.exchange(peer, port, f"GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n".encode(), None, max(0, end-time.monotonic()))
                    except ssl.SSLCertVerificationError:
                        raise RuntimeError("server verification cannot prove client rejection") from None
                    except (ssl.SSLError, EOFError) as error:
                        case["transport_result"] = type(error).__name__
                        if label == "unknown":
                            wait_for(lambda: sum(text(name).count("client policy rejected") for name in nodes)>before,
                                     children(), end, "exact unknown-client policy rejection event")
                            case["native_policy_rejection"] = True
                    else:
                        raise RuntimeError("unauthorized client not rejected")
                    progress["step"] = "healthy-reader-after-" + label; save()
                    if not isinstance(get(port, path), dict):
                        raise RuntimeError("healthy reader returned no observation after rejection control")
                    case.update(state="passed", application_bytes=0, healthy_reader_status=200); save()
            progress["step"] = "complete"; save()
        clients([(node["status"], "/metrics") for node in nodes.values()] +
                [(node["deploy"], "/v1/ingress-policy") for node in nodes.values()], "survivor-client-rejections")
        if not args.direct_only:
            clients([(vip_status, "/metrics"), (vip_deploy, "/v1/ingress-policy")], "vip-client-rejections")
        projection_barrier("post-control-ingress-projection", leader, term)
        evidence["qualification"] = "direct-node-only" if args.direct_only else "direct-and-vip"
        evidence["native_observations"] = {name: [line for line in (output / (name + ".serial.log")).read_text().splitlines() if any(marker in line for marker in (
            "SELFTEST COMPLETE:", "MEMBERSHIP epoch=", "[time] synchronized ", "[management] connection=", "admission policy published:", "VIP ADVERTISER"))]
            for name in ("node-0", "node-1", "node-2")}
        evidence["state"] = "passed"; save()
        print("PASS native management follower admission and leader failover", flush=True)
    print("Evidence: " + str(output / "evidence.json"), flush=True)


if __name__ == "__main__":
    main()
