//! Prepare management credentials against the one selected kernel policy.
//! Only ciphertext is a build input; the recipient never enters an EL0 profile.

use charlotte_launch::management::sealed::PreparedProfile;

pub(super) fn prepare(
    trust: &super::admission::PreparedBootTrust,
) -> Result<Option<PreparedProfile>, charlotte_launch::management::sealed::Error> {
    #[cfg(catten_management_resource)]
    {
        let resource = include_bytes!(env!("CATTEN_MANAGEMENT_TLS_RESOURCE_FILE"));
        trust.management_profile(resource).map(Some)
    }
    #[cfg(not(catten_management_resource))]
    {
        let _ = trust;
        Ok(None)
    }
}
