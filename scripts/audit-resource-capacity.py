#!/usr/bin/env python3
"""Read selected kernel policy and catalog through pinned Rust; report ceilings.

This does not measure live usage or prove that independent maxima are jointly
reachable. No kernel policy, admission state or resource limit is changed.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
POLICY = ROOT / "crates/catten/src/resource_policy.rs"
DEFAULT = POLICY.parent / "resource_policy/defaults.rs"
SDK = ROOT / "crates/catten-syscall/src/lib.rs"
MIB = 1024 * 1024

HARNESS = r'''
#![allow(dead_code)]
#[path = POLICY_PATH]
mod resource_policy;
use resource_policy::POLICY as P;
macro_rules! emit_pools {
    ($(($tag:ident, $field:ident, $n:literal, $first:literal, $second:literal),)*) => {
        $(println!("pool\t{}\t{}\t{}\t{}\t{:?}\t{:?}",
            stringify!($field), $n, $first, $second,
            P.$field.total, P.$field.ordinary);)*
    };
}
fn main() {
    println!("target\t{}", usize::BITS);
    catten_syscall::metadata_pool_catalog!(emit_pools);
    println!("queue\tmailbox_queues\t{}\t{}", P.mailbox_queues.pool.total[1], P.mailbox_queues.pool.ordinary[1]);
    println!("queue\tcompletion_queues\t{}\t{}", P.completion_queues.pool.total[1], P.completion_queues.pool.ordinary[1]);
    println!("heap\t{}\t{}\t{}\t{}", P.kernel_heap.initial_bytes,
        P.kernel_heap.growth_min_bytes, P.kernel_heap.growth_max_bytes, P.kernel_heap.ram_divisor);
    println!("local\ttelemetry_working_bytes\t{}", P.local.telemetry_working_bytes);
    for bytes in std::env::args().skip(1) {
        let usable = bytes.parse::<u64>().unwrap();
        let reserve = P.kernel_heap.growth_reserve_bytes(usable);
        println!("sample\t{}\t{}\t{}", usable, reserve, P.kernel_heap.initial_bytes + reserve);
    }
}
'''


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def capture(policy, usable_mib):
    channel = tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    env = dict(os.environ, CATTEN_RESOURCE_POLICY_FILE=str(policy))
    with tempfile.TemporaryDirectory(prefix="charlotte-capacity-") as temporary:
        directory = Path(temporary)
        library = directory / "libcatten_syscall.rlib"
        harness = directory / "capacity.rs"
        harness.write_text(HARNESS.replace("POLICY_PATH", json.dumps(str(POLICY))))
        common = ["rustc", "+" + channel, "--edition=2024", "-D", "warnings"]
        subprocess.run(common + ["--crate-name=catten_syscall", "--crate-type=rlib", str(SDK), "-o", str(library)],
                       cwd=ROOT, env=env, check=True, capture_output=True, text=True)
        binary = directory / "capacity"
        subprocess.run(common + [str(harness), "--extern", f"catten_syscall={library}", "-o", str(binary)],
                       cwd=ROOT, env=env, check=True, capture_output=True, text=True)
        result = subprocess.run([str(binary)] + [str(value * MIB) for value in usable_mib],
                                check=True, capture_output=True, text=True)
    pools, queues, samples = [], [], []
    heap, local, pointer_bits = {}, {}, None
    for line in result.stdout.splitlines():
        fields = line.split("\t")
        if fields[0] == "pool":
            _, name, dimensions, first, second, total, ordinary = fields
            dimensions = int(dimensions)
            pools.append(dict(name=name, dimensions=dimensions, units=[first, second][:dimensions],
                              total=json.loads(total), ordinary=json.loads(ordinary)))
        elif fields[0] == "queue":
            queues.append(dict(name=fields[1], total_bytes=int(fields[2]), ordinary_bytes=int(fields[3])))
        elif fields[0] == "heap":
            heap = dict(zip(["initial_bytes", "growth_min_bytes", "growth_max_bytes", "ram_divisor"],
                            map(int, fields[1:])))
        elif fields[0] == "sample":
            samples.append(dict(zip(["usable_bytes", "growth_reserve_bytes", "total_mapped_heap_bytes"],
                                    map(int, fields[1:]))))
        elif fields[0] == "local":
            local[fields[1]] = int(fields[2])
        elif fields[0] == "target":
            pointer_bits = int(fields[1])
        else:
            raise ValueError(f"Unexpected policy output: {line}")
    assert pools and heap and len(samples) == len(usable_mib)
    assert len({pool["name"] for pool in pools}) == len(pools)
    assert all(len(pool["total"]) == len(pool["ordinary"]) == pool["dimensions"] for pool in pools)
    byte_pools = [pool for pool in pools if pool["units"][0] == "bytes"]
    sources = {POLICY, DEFAULT, policy, ROOT / "rust-toolchain.toml", Path(__file__).resolve()}
    sources.update(SDK.parent.rglob("*.rs"))
    def identity(path):
        return str(path.relative_to(ROOT)) if path.is_relative_to(ROOT) else str(path)
    return dict(schema="charlotte.resource-capacity.v1", rust_toolchain=channel, host_pointer_bits=pointer_bits,
                selected_policy=identity(policy), source_sha256={identity(p): sha(p) for p in sorted(sources)},
                metadata_pools=pools, byte_pool_count=len(byte_pools),
                byte_ceiling_sums=dict(total=sum(p["total"][0] for p in byte_pools),
                                       ordinary=sum(p["ordinary"][0] for p in byte_pools)),
                separate_queue_byte_ceilings=queues, kernel_heap=heap, local=local, usable_ram_samples=samples,
                interpretation=[
                    "Independent ceilings, not allocated usage, disjoint partitions or jointly reachable maxima.",
                    "Count-only account/control allocations, allocator overhead and boot/transient/retained backing are not converted into bytes here.",
                    "Queue ceilings remain separate; this report does not assert absence of accounting overlap.",
                    "Usable-RAM samples are inputs to the real heap formula, not guest RAM or native measurements.",
                    "No original-principal coverage, cleanup progress or aggregate acceptance is established.",
                ])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--policy", type=Path, default=DEFAULT)
    parser.add_argument("--usable-mib", type=int, action="append",
                        help="Repeatable usable RAM input; defaults to 367, 378, 512, 1024, 4096, 8192, 16384 MiB")
    parser.add_argument("--output", type=Path, help="Write JSON here; otherwise stdout")
    args = parser.parse_args()
    usable = args.usable_mib if args.usable_mib is not None else [367, 378, 512, 1024, 4096, 8192, 16384]
    if any(value < 0 or value > (2**64 - 1) // MIB for value in usable):
        parser.error("usable MiB must fit an unsigned 64-bit byte count")
    try:
        result = capture(args.policy.resolve(strict=True), usable)
    except subprocess.CalledProcessError as error:
        parser.exit(1, error.stdout + error.stderr)
    output = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.write_text(output)
    else:
        print(output, end="")


if __name__ == "__main__":
    main()
