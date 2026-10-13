# Current object-store format

Object-store **v4** has one grammar shared by the service, image producer and
read-only inspector. Internal RPC version 1 and its opcodes remain unchanged.

| Record | Current contract |
| --- | --- |
| Superblock | `COBJSTR4`, version 4, exact 80-byte payload in one device block; generation/next ID nonzero; reserved fields/padding zero; CRC32 |
| Directory | Exact 32-byte locator, current CRC salt `0x34444952`; zero-filled unused slots or checksummed generation-bearing tombstones/live entries |
| Object header | `CCOBJHD4`, version 4, exact 384 bytes in one header block; allocated flag; nonzero ID/generation/owner; known attributes; zero reserved/unused fields; CRC32 |
| Extents | Up to 16 descriptors, nonzero block counts and zero reserved words; exact allocation lengths and device bounds checked against the captured layout |

Host-staged signed artifacts carry kernel principal 1. Runtime creation stores
the authenticated creating principal. Ordinary objects require their owner,
kernel authority, explicit shared-read attributes or operator scope grants.
Reserved artifacts remain publicly readable by authenticated callers and
mutable only by kernel/administration authority. Owner zero and caller zero
cannot gain authority through reserved, shared-read or scoped shortcuts.

Both superblock reads must succeed. Only **two zero-filled slots** authorize
first-boot formatting. A valid current slot can recover a blank/corrupt peer;
retired/unsupported format tags reject even alongside a valid current slot.
Nonblank slots without a valid current copy and incompatible device/layout
metadata fail startup without formatting. Two invalid directory copies fail
mount rather than synthesizing an unused slot. Header rejection fails before
allocation-state reconstruction is persisted. A valid newer tombstone still
prevents resurrection from an older live locator.

The old v3 superblock's actual bytes spelled `COJBSTR3`, despite its comment.
That tag is explicitly unsupported; no old decoder or owner-default path is
retained. Checksummed old headers/directories cannot qualify under a current
root tag. Current header lengths cannot expand through a compatibility suffix.

This pre-production change provides rejection, not automatic migration. Once
stores are deployed, changing the grammar/layout requires a separately reviewed,
version-directed and interruption-tested migration preserving committed state.
Do not reset a store on unsupported, corrupt or unreadable metadata. Developer
runners may explicitly request a fresh fixture image; the inspector's explicit
`format` command replaces its specified development image.

Copy-on-write replacement, dual-bank generations, bounded extents, original
operation ownership and backing-retention policy remain unchanged. CRC32 and
FNV-1a detect corruption; they do not authenticate storage or prove power-loss
atomicity. See [S08a evidence](../reports/audits/2026-10-13-protocol-simplification-object-store.md).
