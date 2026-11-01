use crate::buffer::CryptoBuffer;
use crate::extensions::extension_data::signature_algorithms::{
    SignatureAlgorithms, SignatureScheme,
};

use crate::TlsError;
use crate::parse_buffer::{ParseBuffer, ParseError};

use heapless::Vec;

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct SignatureAlgorithmsCert<const N: usize> {
    pub supported_signature_algorithms: Vec<SignatureScheme, N>,
}

impl<const N: usize> SignatureAlgorithmsCert<N> {
    pub fn parse(buf: &mut ParseBuffer) -> Result<Self, ParseError> {
        Ok(Self {
            supported_signature_algorithms: SignatureAlgorithms::<N>::parse(buf)?
                .supported_signature_algorithms,
        })
    }

    pub fn encode(&self, buf: &mut CryptoBuffer) -> Result<(), TlsError> {
        buf.with_u16_length(|buf| {
            for &a in &self.supported_signature_algorithms {
                buf.push_u16(a.as_u16())
                    .map_err(|_| TlsError::EncodeError)?;
            }
            Ok(())
        })
    }
}
