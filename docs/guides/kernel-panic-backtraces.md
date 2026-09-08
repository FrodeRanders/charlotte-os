# Kernel panic backtraces

CharlotteOS kernel panics are fail-stop events. They do not unwind Rust frames
or run destructors, and the panicking LP never re-enables interrupts. The first
panicking LP writes the panic and a conservative raw frame-pointer backtrace
directly to the serial device without allocating or taking its software lock.
Later panicking LPs stop without competing for diagnostic output.

The kernel targets force frame pointers through `.cargo/config.toml`. The
walker follows frames only while they remain in the stack page containing the
panic handler. This trades depth for safety: an allocator failure normally
retains enough callers to identify the allocating subsystem, while a corrupt
frame cannot direct the walker into an unrelated or unmapped page.

## Build artifacts

Both QEMU runners call `catten_boot_report_kernel` after a kernel build. In
addition to printing the kernel SHA-256, that helper uses the Rust toolchain's
`llvm-nm` to write a demangled, address-sorted symbol map beside the ELF:

```text
target/<kernel-target>/<profile>/catten.symbols
```

The map header records the SHA-256 of the exact ELF. A copy is also retained by
content hash:

```text
target/kernel-symbols/<kernel-sha256>.symbols
```

This archive is small enough to retain across rebuilds and is sufficient for
function-plus-offset lookup. Keep the matching unstripped ELF as well when
file/line lookup will be needed. Development ELFs contain full DWARF; release
ELFs currently retain function symbols but not DWARF source lines.

Before a bounded QEMU run, the runner also copies the matching map to
`<serial-log>.symbols` and writes `<serial-log>.kernel.sha256`. Keeping these
two small sidecars with a captured log makes later function lookup independent
of subsequent kernel rebuilds.

If `llvm-nm` is not on `PATH`, the build helper searches the active Rust
toolchain. Install the `llvm-tools-preview` rustup component or set `LLVM_NM`
to an executable explicitly if neither location is available.

## Symbolizing a panic

Pass the matching kernel ELF and serial log to the postprocessor:

```sh
scripts/symbolize-kernel-panic.py \
  --kernel target/x86_64-unknown-none-catten/debug/catten \
  tmp/kernel-panic.log
```

For logs produced by the bounded runners, the adjacent symbol-map sidecar is
discovered automatically:

```sh
scripts/symbolize-kernel-panic.py /tmp/charlotte-x86-serial.log
```

Add `--source` to ask LLDB for DWARF file and line information:

```sh
scripts/symbolize-kernel-panic.py --source \
  --kernel target/x86_64-unknown-none-catten/debug/catten \
  tmp/kernel-panic.log
```

For an older build whose ELF is no longer present, select its archived map:

```sh
scripts/symbolize-kernel-panic.py \
  --symbols target/kernel-symbols/<kernel-sha256>.symbols \
  tmp/kernel-panic.log
```

Raw addresses may be supplied instead of a log. The bounded QEMU runners also
invoke the function-level postprocessor automatically when their serial log
contains `Kernel panic:`.

The fail-stop loop is local to each LP. It prevents the panicking LP from
dispatching more work through abandoned locks; it does not attempt a risky
lock-taking broadcast from an arbitrary panic context. Other LPs may therefore
run briefly or block on state owned by the stopped LP until the VM runner
terminates the failed guest.
