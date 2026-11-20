This is the `flanterm_bindings` 1.1.4 registry source. `UPSTREAM.json` records
the original crate checksum and hashes of the copied files; `LICENSE` and all
C copyright headers are retained.

Local semantic change: `backends/fb.c` returns from its `fail` path when the
initial context allocation failed, before accessing `ctx->canvas` or other
children. The other failure prefixes retain the backend's size-aware cleanup.
Rust sources are formatted with the workspace, unused build-script imports
are removed and the README's trailing blank line is trimmed. No dependency
version or C success-path behavior is changed.

`scripts/tests/test-boot-allocation-edges.py` compiles this C source directly
against the actual kernel console adapter. It injects all six allocation
failures, checks exact allocation/free layouts, exercises scrolling/escapes
without new allocations, and rejects a base-context-only owner destructor.
The first-allocation control crashed with the unmodified registry source.

The kernel retains one complete successful framebuffer context for the boot
lifetime. Geometry validation and wider caller/IRQ review remain part of Q3/Q5;
this local patch does not qualify arbitrary bootloader framebuffer metadata.
