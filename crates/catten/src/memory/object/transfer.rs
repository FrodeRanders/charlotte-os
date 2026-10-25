//! Memory move/copy/loan transactions, from preparation through atomic
//! publication and ordinary cancellation. The same owner retains source escrow,
//! pins, destination metadata and private backing throughout these phases.
//!
//! Drop preserves the existing containing-operation cleanup policy; extracting
//! this module does not qualify new IRQ/guard contexts or permit physical replay.
use super::*;

/// Prepared authority never exposes an application-accessible destination.
/// One owner cancels staging, restores source authority, and releases backing.
#[must_use]
pub(crate) struct PreparedTransfer {
    target: AddressSpaceId,
    target_handle: super::super::AddressSpaceHandle,
    // Read loans retain fallible borrower storage before source escrow. Ordinary
    // containing cleanup explicitly frees it before consuming destination
    // storage and the original authority reservation.
    pub(super) borrower_storage: Option<BorrowerStorage>,
    destination: Option<cap_storage::Preparing>,
    payload: PreparedPayload,
    pub(super) committed: bool,
}

struct PreparedSource {
    owner: AddressSpaceId,
    handle: super::super::AddressSpaceHandle,
    cap: MemoryObjectCap,
    entry: MemoryCap,
    escrow: Option<crate::capability::SourceEscrow>,
    retired_payload: Option<cap_storage::RetiredCapability>,
    _pin: ScopedCopyPin,
}

enum PreparedPayload {
    Move {
        source: PreparedSource,
        rights: MemoryObjectRights,
    },
    Copy {
        object: MemoryObjectId,
        frames: Option<ChargedFrames>,
    },
    Loan {
        source: PreparedSource,
        write: bool,
    },
}

impl PreparedPayload {
    fn source_mut(&mut self) -> Option<&mut PreparedSource> {
        match self {
            Self::Move {
                source,
                ..
            }
            | Self::Loan {
                source,
                ..
            } => Some(source),
            Self::Copy {
                ..
            } => None,
        }
    }
}

impl PreparedTransfer {
    pub(crate) fn target(&self) -> AddressSpaceId {
        self.target
    }

    pub(crate) fn target_cap(&self) -> MemoryObjectCap {
        self.destination.as_ref().unwrap().identity()
    }

    /// IPC adapter captures loan provenance before publishing the batch.
    pub(crate) fn loan_origin(&self) -> Option<(AddressSpaceId, MemoryObjectCap)> {
        match &self.payload {
            PreparedPayload::Loan {
                source,
                ..
            } => Some((source.owner, source.cap)),
            _ => None,
        }
    }

    pub(crate) fn commit(mut self) -> Result<MemoryObjectCap, MemoryObjectError> {
        commit_transfers(core::slice::from_mut(&mut self))?;
        Ok(self.target_cap())
    }
}

impl Drop for PreparedTransfer {
    fn drop(&mut self) {
        if let Some(source) = self.payload.source_mut() {
            let mut registry = MEMORY_OBJECTS.lock();
            let interest = registry
                .objects
                .get_mut(&source.entry.object)
                .and_then(|object| object.close_interest.take());
            if !self.committed
                && let Some(escrow) = source.escrow.take()
            {
                // Restore only the captured namespace and original source slot.
                // Keep registry exclusion through pin release: a concurrent
                // close must not mistake restored-but-still-pinned for terminal.
                let _ = escrow.rollback();
            }
            let frames = source
                ._pin
                .take()
                .and_then(|pin| release_copy_pin_locked(&mut registry, pin.object_id()));
            if let Some(entry) = registry
                .caps
                .get_mut(&source.owner)
                .and_then(|caps| caps.caps.get_mut(&source.cap))
                // Teardown may already have removed this namespace and a
                // successor may reuse both scalar IDs. Object identities do
                // not recycle: never clear a successor's transfer fence.
                .filter(|entry| entry.object == source.entry.object)
            {
                assert!(entry.transfer_in_flight, "prepared source fence missing");
                entry.transfer_in_flight = false;
            }
            if let Some(object) = registry.objects.get_mut(&source.entry.object) {
                object.transfer_root = None;
            }
            drop(registry);
            deallocate_frames(frames);
            // Successful move publication carries the detached authority node
            // in this exact source owner through payload completion and pin
            // release. Only now may its original metadata/charge be disposed.
            if let Some(payload) = source.retired_payload.take() {
                payload.release();
            }
            if let Some(escrow) = source.escrow.as_mut() {
                escrow.finish_retired();
            }
            if let Some(interest) = interest {
                // IPC may still be held by this producer. Transfer completion
                // closes its exact source and queues admitted nodes; callbacks
                // and final list/node disposal follow this LP's guard exit.
                interest.finish();
            }
        }
        // Ordinary containing-owner policy remains responsible for this outer
        // context. Dispose unused borrower metadata before destination storage
        // and hidden authority; bare destination abandonment itself is inert.
        drop(self.borrower_storage.take());
        if let Some(destination) = self.destination.take() {
            destination.finish();
        }
        // Private-copy backing keeps its existing containing field policy.
    }
}

pub(crate) fn prepare_move(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    target: AddressSpaceId,
) -> Result<PreparedTransfer, MemoryObjectError> {
    prepare_source_transfer(owner, cap, target, None, false)
}

fn prepare_move_with_rights(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    target: AddressSpaceId,
    read_only: bool,
) -> Result<PreparedTransfer, MemoryObjectError> {
    prepare_source_transfer(owner, cap, target, None, read_only)
}

pub(crate) fn prepare_loan(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    target: AddressSpaceId,
    write: bool,
) -> Result<PreparedTransfer, MemoryObjectError> {
    if owner == target {
        return Err(MemoryObjectError::WrongOwner);
    }
    prepare_source_transfer(owner, cap, target, Some(write), false)
}

fn prepare_source_transfer(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    target: AddressSpaceId,
    loan: Option<bool>,
    read_only: bool,
) -> Result<PreparedTransfer, MemoryObjectError> {
    prepare_source_transfer_with(owner, cap, target, loan, read_only, || {
        BorrowerStorage::try_new().map_err(|_| MemoryObjectError::ResourceLimit)
    })
}

pub(super) fn prepare_source_transfer_with(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    target: AddressSpaceId,
    loan: Option<bool>,
    read_only: bool,
    prepare_borrower: impl FnOnce() -> Result<BorrowerStorage, MemoryObjectError>,
) -> Result<PreparedTransfer, MemoryObjectError> {
    let source_handle = super::super::current_address_space_handle(owner)
        .ok_or(MemoryObjectError::AddressSpaceMissing)?;
    let target_handle = super::super::current_address_space_handle(target)
        .ok_or(MemoryObjectError::AddressSpaceMissing)?;
    // Reserve the exact destination and prepare scalar borrower storage before
    // entering memory serialization. Complete source/target validation below
    // precedes escrow; ordinary rejection drops this unused prefix after unlock.
    let destination =
        cap_storage::Preparing::new(admit_capability(target, target_handle)?, target_handle)?;
    let (destination, (borrower_storage, payload)) = destination.try_prepare(|_| {
        // Shared capability admission bounds live borrower records. Reject a
        // missing node before changing source authority or taking its pin.
        let borrower_storage = if loan == Some(false) {
            Some(prepare_borrower()?)
        } else {
            None
        };
        let mut registry = MEMORY_OBJECTS.lock();
        if !super::super::budget::accepting(source_handle)
            || !super::super::budget::accepting(target_handle)
        {
            return Err(MemoryObjectError::AddressSpaceMissing);
        }
        let entry = registry.lookup(owner, cap)?;
        if entry.transfer_in_flight {
            return Err(MemoryObjectError::LendingActive);
        }
        let needed = match loan {
            Some(true) => MemoryObjectRights::MAP_WRITE,
            Some(false) => MemoryObjectRights::MAP_READ,
            None => MemoryObjectRights::TRANSFER,
        };
        if !entry.rights.contains(needed)
            || (read_only && !entry.rights.contains(MemoryObjectRights::MAP_READ))
        {
            return Err(MemoryObjectError::MissingRight);
        }
        let object =
            registry.objects.get(&entry.object).ok_or(MemoryObjectError::UnknownCapability)?;
        if object.owner != owner {
            return Err(MemoryObjectError::WrongOwner);
        }
        validate_source_transfer(object, target, loan, false)?;
        let pins = object.copy_pins.checked_add(1).ok_or(MemoryObjectError::ResourceLimit)?;
        let escrow = crate::capability::escrow_captured(
            owner,
            cap,
            crate::capability::ObjectKind::Memory,
            Some(source_handle),
        )
        .map_err(capability_error)?;
        // No fallible work follows source escrow. A read-retention pin fences
        // writes/DMA and keeps backing alive even if source teardown drains it.
        let object = registry.objects.get_mut(&entry.object).unwrap();
        object.copy_pins = pins;
        object.transfer_root = Some(source_handle);
        registry.caps.get_mut(&owner).unwrap().caps.get_mut(&cap).unwrap().transfer_in_flight =
            true;
        let source = PreparedSource {
            owner,
            handle: source_handle,
            cap,
            entry,
            escrow: Some(escrow),
            retired_payload: None,
            _pin: ScopedCopyPin::new(Some(CopyPin::new(
                entry.object,
                snapshot_storage::Snapshot::default(),
            ))),
        };
        let payload = match loan {
            Some(write) => PreparedPayload::Loan {
                source,
                write,
            },
            None => PreparedPayload::Move {
                source,
                rights: if read_only {
                    MemoryObjectRights::MAP_READ
                } else {
                    entry.rights
                },
            },
        };
        Ok((borrower_storage, payload))
    })?;
    Ok(PreparedTransfer {
        target,
        target_handle,
        destination: Some(destination),
        borrower_storage,
        payload,
        committed: false,
    })
}

fn validate_source_transfer(
    object: &MemoryObject,
    target: AddressSpaceId,
    loan: Option<bool>,
    prepared: bool,
) -> Result<(), MemoryObjectError> {
    if object.destroy_when_unpinned || object.retirement_pins != 0 || object.dma_pins != 0 {
        return Err(MemoryObjectError::LendingActive);
    }
    if loan == Some(false) {
        if matches!(object.lend_state, LendState::Write { .. } | LendState::Revoking)
            || matches!(&object.lend_state, LendState::Read { borrowers }
                if borrowers.contains_key(&target))
        {
            return Err(MemoryObjectError::LendingActive);
        }
        if object.mappings.values().any(|mapping| mapping.state.writable) {
            return Err(MemoryObjectError::AlreadyMapped);
        }
        if prepared && object.copy_pins == 0 {
            return Err(MemoryObjectError::LendingActive);
        }
    } else {
        if object.lend_state.is_active() || object.copy_pins != usize::from(prepared) {
            return Err(MemoryObjectError::LendingActive);
        }
        if object.mappings.first_key_value().is_some() {
            return Err(MemoryObjectError::AlreadyMapped);
        }
    }
    Ok(())
}

/// Every mixed-mode destination publishes or none. Copies retain private
/// charged frames; loans retain source authority without live borrower state.
pub(crate) fn commit_transfers(
    transfers: &mut [PreparedTransfer],
) -> Result<(), MemoryObjectError> {
    commit_transfers_with_authority(transfers, &mut [])
}

/// Compose memory publication with already-reserved authority from another
/// subsystem. Its caller must retain that payload registry until all additional
/// payloads are installed. No extra authority is published on validation failure.
pub(crate) fn commit_transfers_with_authority(
    transfers: &mut [PreparedTransfer],
    additional: &mut [&mut crate::capability::Reservation],
) -> Result<(), MemoryObjectError> {
    commit_transfers_with_delivery(transfers, additional, false)
}

/// IPC retains move/copy destinations until delivery. Loans keep their existing
/// revocation contract: their lender/borrower receipts own any mapped backing.
pub(crate) fn commit_undelivered_transfers(
    transfers: &mut [PreparedTransfer],
) -> Result<(), MemoryObjectError> {
    commit_undelivered_transfers_with_authority(transfers, &mut [])
}

pub(crate) fn commit_undelivered_transfers_with_authority(
    transfers: &mut [PreparedTransfer],
    additional: &mut [&mut crate::capability::Reservation],
) -> Result<(), MemoryObjectError> {
    commit_transfers_with_delivery(transfers, additional, true)
}

/// Two stable slices form one authority transaction. The capability gate
/// reborrows them twice while holding its existing registry; membership and
/// transfer modes stay unchanged until complete validation and publication.
struct TransferPublications<'batch, 'authority> {
    transfers: &'batch mut [PreparedTransfer],
    additional: &'batch mut [&'authority mut crate::capability::Reservation],
}

impl crate::capability::PublicationBatch for TransferPublications<'_, '_> {
    fn publications(&mut self) -> impl Iterator<Item = crate::capability::Publication<'_>> {
        use crate::capability::{
            Publication,
            SourceDisposition,
        };
        self.additional
            .iter_mut()
            .map(|destination| Publication {
                destination,
                source: None,
            })
            .chain(self.transfers.iter_mut().map(|transfer| {
                let source = match &mut transfer.payload {
                    PreparedPayload::Move {
                        source,
                        ..
                    } => Some((source.escrow.as_mut().unwrap(), SourceDisposition::Revoke)),
                    PreparedPayload::Loan {
                        source,
                        ..
                    } => Some((source.escrow.as_mut().unwrap(), SourceDisposition::Restore)),
                    PreparedPayload::Copy {
                        ..
                    } => None,
                };
                Publication {
                    destination: transfer.destination.as_mut().unwrap().authority(),
                    source,
                }
            }))
    }
}

fn publish_transfer_authority(
    transfers: &mut [PreparedTransfer],
    additional: &mut [&mut crate::capability::Reservation],
) -> Result<(), MemoryObjectError> {
    crate::capability::publish_batch(&mut TransferPublications {
        transfers,
        additional,
    })
    .map_err(capability_error)
}

pub(super) fn commit_transfers_with_delivery(
    transfers: &mut [PreparedTransfer],
    additional: &mut [&mut crate::capability::Reservation],
    delivery_pending: bool,
) -> Result<(), MemoryObjectError> {
    // Check the complete shape before registry access. The borrowed batch
    // needs no storage proportional to its length and admits no new authority.
    transfers.len().checked_add(additional.len()).ok_or(MemoryObjectError::ResourceLimit)?;
    // Scalar-only calls/replies need no memory-registry serialization.
    if transfers.is_empty() {
        return publish_transfer_authority(transfers, additional);
    }
    let mut registry = MEMORY_OBJECTS.lock();
    for transfer in transfers.iter() {
        if transfer.committed || !super::super::budget::accepting(transfer.target_handle) {
            return Err(MemoryObjectError::AddressSpaceMissing);
        }
        transfer.destination.as_ref().unwrap().validate(&registry)?;
        let (source, loan) = match &transfer.payload {
            PreparedPayload::Copy {
                frames,
                ..
            } => {
                if !frames.as_ref().unwrap().admission_active() {
                    return Err(MemoryObjectError::AddressSpaceMissing);
                }
                continue;
            }
            PreparedPayload::Move {
                source,
                ..
            } => (source, None),
            PreparedPayload::Loan {
                source,
                write,
            } => (source, Some(*write)),
        };
        if !super::super::budget::accepting(source.handle) {
            return Err(MemoryObjectError::AddressSpaceMissing);
        }
        let stored = registry
            .caps
            .get(&source.owner)
            .and_then(|caps| caps.caps.get(&source.cap))
            .ok_or(MemoryObjectError::AddressSpaceMissing)?;
        let object = registry
            .objects
            .get(&source.entry.object)
            .ok_or(MemoryObjectError::AddressSpaceMissing)?;
        if stored.object != source.entry.object
            || stored.rights != source.entry.rights
            || !stored.transfer_in_flight
            || object.owner != source.owner
        {
            return Err(MemoryObjectError::AddressSpaceMissing);
        }
        validate_source_transfer(object, transfer.target, loan, true)?;
    }
    publish_transfer_authority(transfers, additional)?;
    // The registries serialize all verified payloads. No fallible work follows
    // authority publication; application memory lookup waits for this guard.
    for transfer in transfers.iter_mut() {
        let target_cap = transfer.target_cap();
        let entry = match &mut transfer.payload {
            PreparedPayload::Move {
                source,
                rights,
            } => {
                source.retired_payload = Some(
                    registry
                        .caps
                        .get_mut(&source.owner)
                        .unwrap()
                        .caps
                        .take(&source.cap)
                        .expect("prepared source payload missing"),
                );
                registry.objects.get_mut(&source.entry.object).unwrap().owner = transfer.target;
                MemoryCap {
                    object: source.entry.object,
                    rights: *rights,
                    transfer_in_flight: false,
                    delivery_pending,
                }
            }
            PreparedPayload::Copy {
                object,
                frames,
            } => {
                let object = *object;
                let (storage, frames, charge) = frames.take().unwrap().into_parts();
                registry.objects.insert(
                    storage,
                    object,
                    MemoryObject {
                        owner: transfer.target,
                        frames,
                        charge,
                        mappings: AdmittedMap::new(),
                        lend_state: LendState::None,
                        dma_pins: 0,
                        exclusive_dma_pins: 0,
                        copy_pins: 0,
                        retirement_pins: 0,
                        destroy_when_unpinned: false,
                        close_interest: None,
                        transfer_root: None,
                        retired_caps: cap_storage::RetiredCapabilities::new(),
                    },
                );
                MemoryCap {
                    object,
                    rights: MemoryObjectRights::ALL,
                    transfer_in_flight: false,
                    delivery_pending,
                }
            }
            PreparedPayload::Loan {
                source,
                write,
            } => {
                let object = registry.objects.get_mut(&source.entry.object).unwrap();
                if *write {
                    object.lend_state = LendState::Write {
                        borrower: transfer.target,
                        cap: target_cap,
                    };
                } else {
                    object.lend_state.publish_read(
                        transfer
                            .borrower_storage
                            .take()
                            .expect("prepared borrower storage missing"),
                        transfer.target,
                        target_cap,
                    );
                }
                MemoryCap {
                    object: source.entry.object,
                    rights: if *write {
                        MemoryObjectRights(
                            MemoryObjectRights::MAP_READ.0 | MemoryObjectRights::MAP_WRITE.0,
                        )
                    } else {
                        MemoryObjectRights::MAP_READ
                    },
                    transfer_in_flight: false,
                    delivery_pending: false,
                }
            }
        };
        transfer.destination.as_mut().unwrap().install(&mut registry, entry);
        transfer.committed = true;
    }
    Ok(())
}

/// IPC delivery boundary. The caller holds IPC exclusively and still owns the
/// exact queued message or unobserved result. No application can close, escrow,
/// map or DMA-pin an undelivered move/copy before this registry transition.
pub(crate) fn deliver_memory(asid: AddressSpaceId, caps: &[MemoryObjectCap]) {
    let mut registry = MEMORY_OBJECTS.lock();
    for &cap in caps {
        // A direct lender may have revoked a loan before receive. Those loan
        // receipts retain their existing semantics and need no publication.
        if let Some(entry) = registry.caps.get_mut(&asid).and_then(|caps| caps.caps.get_mut(&cap)) {
            entry.delivery_pending = false;
        }
    }
}

pub fn move_to(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    target: AddressSpaceId,
) -> Result<MemoryObjectCap, MemoryObjectError> {
    prepare_move(owner, cap, target)?.commit()
}

/// Transfer an immutable launch object, attenuating its destination rights.
pub fn move_read_only_to(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    target: AddressSpaceId,
) -> Result<MemoryObjectCap, MemoryObjectError> {
    prepare_move_with_rights(owner, cap, target, true)?.commit()
}

pub(crate) fn prepare_copy(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    target: AddressSpaceId,
) -> Result<PreparedTransfer, MemoryObjectError> {
    prepare_copy_with(owner, cap, target, ChargedFrames::allocate)
}

pub(super) fn prepare_copy_with(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    target: AddressSpaceId,
    allocate: impl FnOnce(
        super::super::AddressSpaceHandle,
        usize,
    ) -> Result<ChargedFrames, MemoryObjectError>,
) -> Result<PreparedTransfer, MemoryObjectError> {
    if owner == target {
        return Err(MemoryObjectError::WrongOwner);
    }
    let source_handle = super::super::current_address_space_handle(owner)
        .ok_or(MemoryObjectError::AddressSpaceMissing)?;
    let target_handle = super::super::current_address_space_handle(target)
        .ok_or(MemoryObjectError::AddressSpaceMissing)?;
    let copy_pin = ScopedCopyPin::new(Some(pin_for_copy(owner, cap)?));
    if !super::super::budget::accepting(source_handle) {
        return Err(MemoryObjectError::AddressSpaceMissing);
    }
    let destination =
        cap_storage::Preparing::new(admit_capability(target, target_handle)?, target_handle)?;
    let (destination, payload) = destination.try_prepare(|_| {
        let source = copy_pin.as_ref().unwrap();
        // This owner retains the private backing until destination publication.
        // Cancellation never consults a receiver-controlled numeric handle.
        let staged = allocate(source_handle, source.frames().len())?;
        for (source, target_frame) in source.frames().iter().zip(staged.prepared_frames().iter()) {
            let source_ptr: *const u8 = (*source).into();
            let target_ptr: *mut u8 = (*target_frame).into();
            unsafe {
                core::ptr::copy_nonoverlapping(source_ptr, target_ptr, PAGE_SIZE);
            }
        }
        let object = { MEMORY_OBJECTS.lock().allocate_object_id() }?;
        Ok(PreparedPayload::Copy {
            object,
            frames: Some(staged),
        })
    })?;
    Ok(PreparedTransfer {
        target,
        target_handle,
        destination: Some(destination),
        borrower_storage: None,
        payload,
        committed: false,
    })
}

pub fn copy_to(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    target: AddressSpaceId,
) -> Result<MemoryObjectCap, MemoryObjectError> {
    prepare_copy(owner, cap, target)?.commit()
}

pub fn lend_read(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    borrower: AddressSpaceId,
) -> Result<MemoryObjectCap, MemoryObjectError> {
    prepare_loan(owner, cap, borrower, false)?.commit()
}

pub fn lend_write(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    borrower: AddressSpaceId,
) -> Result<MemoryObjectCap, MemoryObjectError> {
    prepare_loan(owner, cap, borrower, true)?.commit()
}
