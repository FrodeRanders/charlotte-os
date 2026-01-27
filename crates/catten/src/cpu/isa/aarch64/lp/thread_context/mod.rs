pub struct ThreadContext;

impl ThreadContext {
    pub fn new(
        _asid: crate::memory::AddressSpaceId,
        _entry_point: crate::memory::VAddr,
    ) -> Result<Self, ()> {
        todo!()
    }
}
