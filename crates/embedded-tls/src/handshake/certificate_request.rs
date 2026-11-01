use crate::TlsError;
use crate::extensions::extension_data::signature_algorithms::SignatureScheme;
use crate::extensions::messages::CertificateRequestExtension;
use crate::parse_buffer::ParseBuffer;
use heapless::Vec;

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct CertificateRequestRef<'a> {
    pub(crate) request_context: &'a [u8],
    pub(crate) signature_algorithms: Vec<SignatureScheme, 25>,
}

impl<'a> CertificateRequestRef<'a> {
    pub fn parse(buf: &mut ParseBuffer<'a>) -> Result<CertificateRequestRef<'a>, TlsError> {
        let request_context_len = buf
            .read_u8()
            .map_err(|_| TlsError::InvalidCertificateRequest)?;
        let request_context = buf
            .slice(request_context_len as usize)
            .map_err(|_| TlsError::InvalidCertificateRequest)?;

        // Validate extensions
        let extensions = CertificateRequestExtension::parse_vector::<6>(buf)?;

        let mut signature_algorithms = None;
        for extension in extensions {
            if let CertificateRequestExtension::SignatureAlgorithms(algorithms) = extension
                && signature_algorithms
                    .replace(algorithms.supported_signature_algorithms)
                    .is_some()
            {
                return Err(TlsError::InvalidCertificateRequest);
            }
        }
        if !buf.is_empty() {
            return Err(TlsError::InvalidCertificateRequest);
        }
        Ok(Self {
            request_context: request_context.as_slice(),
            signature_algorithms: signature_algorithms
                .ok_or(TlsError::InvalidCertificateRequest)?,
        })
    }
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct CertificateRequest {
    pub(crate) request_context: Vec<u8, 256>,
    pub(crate) signature_algorithms: Vec<SignatureScheme, 25>,
}

impl<'a> TryFrom<CertificateRequestRef<'a>> for CertificateRequest {
    type Error = TlsError;
    fn try_from(cert: CertificateRequestRef<'a>) -> Result<Self, Self::Error> {
        let mut request_context = Vec::new();
        request_context
            .extend_from_slice(cert.request_context)
            .map_err(|_| {
                error!("CertificateRequest: InsufficientSpace");
                TlsError::InsufficientSpace
            })?;

        Ok(Self {
            request_context,
            signature_algorithms: cert.signature_algorithms,
        })
    }
}
