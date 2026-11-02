//! Metadata pool identities and read-only statistics records. Counts describe
//! allocation lifetimes, not authority or physical completion.

#[macro_export]
macro_rules! metadata_pool_catalog {
    ($consumer:ident) => {
        $consumer! {
            (RootMetadata, root_metadata, 2, "bytes", "nodes"),
            (NamespaceNodes, namespace_nodes, 2, "bytes", "nodes"),
            (AuthorityNodes, authority_nodes, 2, "bytes", "nodes"),
            (MailboxNodes, mailbox_nodes, 2, "bytes", "nodes"),
            (DeferredNodes, deferred_nodes, 2, "bytes", "nodes"),
            (ObserverBacking, observer_backing, 2, "bytes", "bundles"),
            (CompletionNodes, completion_nodes, 2, "bytes", "nodes"),
            (DeviceNodes, device_nodes, 2, "bytes", "nodes"),
            (IommuNodes, iommu_nodes, 2, "bytes", "nodes"),
            (PrincipalHistory, principal_history, 2, "bytes", "nodes"),
            (ThreadMetadata, thread_metadata, 2, "bytes", "bundles"),
            (ThreadTable, thread_table, 2, "bytes", "slots"),
            (RootTable, root_table, 2, "bytes", "slots"),
            (WaiterAccounts, waiter_accounts, 1, "bundles", ""),
            (TimerAccounts, timer_accounts, 1, "bundles", ""),
            (CompletionAccounts, completion_accounts, 1, "bundles", ""),
            (NamespaceAccounts, namespace_accounts, 1, "bundles", ""),
            (ScratchNodes, scratch_nodes, 2, "bytes", "nodes"),
            (SnapshotBuffers, snapshot_buffers, 2, "bytes", "buffers"),
            (MappingNodes, mapping_nodes, 2, "bytes", "nodes"),
            (MemoryCapabilityNodes, memory_capability_nodes, 2, "bytes", "nodes"),
            (ObserverLists, observer_lists, 1, "allocations", ""),
            (IpcNodes, ipc_nodes, 2, "bytes", "nodes"),
        }
    };
}
macro_rules! identities {
    ($(($tag:ident, $field:ident, $n:literal, $first:literal, $second:literal),)*) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        #[repr(u64)]
        pub enum Id { $($tag,)* }
        pub const ALL: &[Id] = &[$(Id::$tag,)*];
        impl Id {
            pub const fn name(self) -> &'static str { match self { $(Self::$tag => stringify!($field),)* } }
            pub const fn dimensions(self) -> usize { match self { $(Self::$tag => $n,)* } }
            pub const fn units(self) -> [&'static str; 2] { match self { $(Self::$tag => [$first, $second],)* } }
        }
    };
}
metadata_pool_catalog!(identities);
pub const COUNT: usize = ALL.len();
pub const WORDS: usize = 11;
pub const SAMPLED: u64 = 0;
pub const BUSY: u64 = 1;

/// Limits are always available. Busy observations have no usage sample.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Sample {
    pub id: Id,
    pub total_limit: [u64; 2],
    pub ordinary_limit: [u64; 2],
    pub used: Option<([u64; 2], [u64; 2])>,
}
impl Sample {
    pub fn words(self) -> [u64; WORDS] {
        let (total, ordinary) = self.used.unwrap_or(([0; 2], [0; 2]));
        [
            self.id as u64,
            self.id.dimensions() as u64,
            if self.used.is_some() {
                SAMPLED
            } else {
                BUSY
            },
            self.total_limit[0],
            self.total_limit[1],
            self.ordinary_limit[0],
            self.ordinary_limit[1],
            total[0],
            total[1],
            ordinary[0],
            ordinary[1],
        ]
    }
}

/// Validate the complete current statistics payload, then borrow its pool
/// section. The caller must pass the declared payload length, not page padding.
/// Self snapshots contain no pools; observer snapshots contain the full catalog.
pub fn from_statistics(bytes: &[u8]) -> Option<impl ExactSizeIterator<Item = Sample> + '_> {
    use crate::{
        THREAD_STATISTICS_DOMAIN_RECORD_U64S as DOMAIN,
        THREAD_STATISTICS_HEADER_U64S as HEADER,
        THREAD_STATISTICS_RECORD_U64S as THREAD,
        thread_statistics_header as h,
    };
    let word = |index: usize| -> Option<u64> {
        let start = index.checked_mul(8)?;
        Some(u64::from_le_bytes(bytes.get(start..start.checked_add(8)?)?.try_into().ok()?))
    };
    if word(h::MAGIC)? != crate::THREAD_STATISTICS_MAGIC
        || word(h::VERSION)? != crate::THREAD_STATISTICS_VERSION
        || word(h::RECORD_BYTES)? != (THREAD * 8) as u64
        || word(h::DOMAIN_RECORD_BYTES)? != (DOMAIN * 8) as u64
        || word(h::METADATA_RECORD_BYTES)? != (WORDS * 8) as u64
    {
        return None;
    }
    let count = usize::try_from(word(h::METADATA_RECORD_COUNT)?).ok()?;
    if count != 0 && count != COUNT {
        return None;
    }
    let start = HEADER
        .checked_add(usize::try_from(word(h::RECORD_COUNT)?).ok()?.checked_mul(THREAD)?)?
        .checked_add(usize::try_from(word(h::DOMAIN_RECORD_COUNT)?).ok()?.checked_mul(DOMAIN)?)?;
    let end = start.checked_add(count.checked_mul(WORDS)?)?.checked_mul(8)?;
    if bytes.len() != end {
        return None;
    }
    let records = &bytes[start * 8..];
    for (id, record) in ALL.iter().zip(records.as_chunks::<{ WORDS * 8 }>().0.iter()) {
        decode(*id, record)?;
    }
    Some(
        ALL[..count]
            .iter()
            .zip(records.as_chunks::<{ WORDS * 8 }>().0.iter())
            .map(|(id, record)| decode(*id, record).expect("validated immutable pool record")),
    )
}
fn decode(id: Id, record: &[u8]) -> Option<Sample> {
    let mut w = [0; WORDS];
    for (slot, bytes) in w.iter_mut().zip(record.as_chunks::<8>().0.iter()) {
        *slot = u64::from_le_bytes(*bytes);
    }
    if w[0] != id as u64 || w[1] != id.dimensions() as u64 {
        return None;
    }
    let total_limit = [w[3], w[4]];
    let ordinary_limit = [w[5], w[6]];
    let total = [w[7], w[8]];
    let ordinary = [w[9], w[10]];
    for i in 0..2 {
        if i >= id.dimensions() {
            if [total_limit[i], ordinary_limit[i], total[i], ordinary[i]] != [0; 4] {
                return None;
            }
        } else if ordinary_limit[i] == 0
            || ordinary_limit[i] >= total_limit[i]
            || total[i] > total_limit[i]
            || ordinary[i] > ordinary_limit[i]
            || ordinary[i] > total[i]
        {
            return None;
        }
    }
    let used = match w[2] {
        SAMPLED => Some((total, ordinary)),
        BUSY if total == [0; 2] && ordinary == [0; 2] => None,
        _ => return None,
    };
    Some(Sample {
        id,
        total_limit,
        ordinary_limit,
        used,
    })
}

#[cfg(test)]
#[path = "metadata_pools/tests.rs"]
mod tests;
