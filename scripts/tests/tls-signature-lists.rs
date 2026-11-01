// Compiled by test-tls-signature-lists.py against the actual parser modules.
use extensions::extension_data::{
    signature_algorithms::{
        SignatureAlgorithms,
        SignatureScheme,
    },
    signature_algorithms_cert::SignatureAlgorithmsCert,
};
use parse_buffer::{
    ParseBuffer,
    ParseError,
};

fn offers(codes: &[u16]) -> Vec<u8> {
    let mut bytes = (2 * codes.len() as u16).to_be_bytes().to_vec();
    bytes.extend(codes.iter().flat_map(|code| code.to_be_bytes()));
    bytes
}

#[test]
fn captured_openssl4_offer_ignores_sm2_and_preserves_p256() {
    // Exact signature_algorithms vector in the November 1 failed request.
    let codes = [
        0x0905, 0x0906, 0x0904, 0x0403, 0x0503, 0x0603, 0x0807, 0x0808, 0x081a, 0x081b, 0x081c,
        0x0809, 0x080a, 0x080b, 0x0804, 0x0805, 0x0806, 0x0401, 0x0501, 0x0601, 0x0708,
    ];
    let bytes = offers(&codes);
    let mut input = ParseBuffer::new(&bytes);
    let parsed = SignatureAlgorithms::<25>::parse(&mut input).unwrap();
    assert!(input.is_empty());
    assert_eq!(parsed.supported_signature_algorithms.len(), 20);
    assert_eq!(parsed.supported_signature_algorithms[0], SignatureScheme::MlDsa65);
    assert!(parsed.supported_signature_algorithms.contains(&SignatureScheme::EcdsaSecp256r1Sha256));
    assert!(!parsed.supported_signature_algorithms.iter().any(|scheme| scheme.as_u16() == 0x0708));
    let certs = SignatureAlgorithmsCert::<25>::parse(&mut ParseBuffer::new(&bytes)).unwrap();
    assert_eq!(certs.supported_signature_algorithms, parsed.supported_signature_algorithms);
}

#[test]
fn unknown_and_repeated_offers_do_not_consume_known_storage() {
    let mut codes = vec![0x0a0a; 1024]; // GREASE, unknown to this implementation.
    codes.extend([0x0403; 1024]);
    codes.push(0xfefe);
    let bytes = offers(&codes);
    let parsed = SignatureAlgorithms::<1>::parse(&mut ParseBuffer::new(&bytes)).unwrap();
    assert_eq!(
        parsed.supported_signature_algorithms.as_slice(),
        &[SignatureScheme::EcdsaSecp256r1Sha256]
    );
    let bytes = offers(&[0x0708, 0xffff, 0x0a0a]);
    assert!(SignatureAlgorithms::<1>::parse(&mut ParseBuffer::new(&bytes))
        .unwrap()
        .supported_signature_algorithms
        .is_empty());
}

#[test]
fn malformed_vectors_are_never_ignored() {
    for bytes in [
        &[][..],
        &[0],
        &[0, 0],
        &[0, 1, 0x07],
        &[0, 3, 0x07, 0x08, 0x04],
        &[0, 4, 0x07, 0x08],
        &[0, 2, 0x07, 0x08, 0],
        &[0, 4, 0x04, 0x03, 0x07],
    ] {
        assert!(
            SignatureAlgorithms::<25>::parse(&mut ParseBuffer::new(bytes)).is_err(),
            "{bytes:02x?}"
        );
        assert!(
            SignatureAlgorithmsCert::<25>::parse(&mut ParseBuffer::new(bytes)).is_err(),
            "{bytes:02x?}"
        );
    }
}

#[test]
fn distinct_known_capacity_stays_bounded() {
    let bytes = offers(&[0x0403, 0x0503]);
    assert!(matches!(
        SignatureAlgorithms::<1>::parse(&mut ParseBuffer::new(&bytes)),
        Err(ParseError::InsufficientSpace)
    ));
}

#[test]
fn unknown_selected_signature_is_rejected() {
    use handshake::certificate_verify::CertificateVerifyRef;
    for code in [0x0708u16, 0x0a0a, 0xfefe, 0xffff, 0x0420, 0x0520, 0x0620] {
        let mut bytes = code.to_be_bytes().to_vec();
        bytes.extend([0, 1, 0]);
        assert!(matches!(
            CertificateVerifyRef::parse(&mut ParseBuffer::new(&bytes)),
            Err(TlsError::InvalidSignatureScheme)
        ));
    }
    assert!(CertificateVerifyRef::parse(&mut ParseBuffer::new(&[0x04, 0x03, 0, 1, 0])).is_ok());
}

#[test]
fn legacy_certificate_offers_never_become_handshake_signatures() {
    use handshake::certificate_verify::CertificateVerifyRef;
    for code in [0x0401u16, 0x0501, 0x0601, 0x0201, 0x0203, 0x0301, 0x0302, 0x0303] {
        // Recognizing an offer preserves grammar without authorizing its use.
        let bytes = offers(&[code, 0x0403]);
        assert_eq!(
            SignatureAlgorithms::<25>::parse(&mut ParseBuffer::new(&bytes))
                .unwrap()
                .supported_signature_algorithms
                .len(),
            2
        );
        let mut selected = code.to_be_bytes().to_vec();
        selected.extend([0, 1, 0]);
        assert!(matches!(
            CertificateVerifyRef::parse(&mut ParseBuffer::new(&selected)),
            Err(TlsError::InvalidSignatureScheme)
        ));
    }
}

fn request(extensions: &[&[u8]]) -> Vec<u8> {
    let bytes: Vec<u8> = extensions.iter().flat_map(|bytes| bytes.iter().copied()).collect();
    let mut result = vec![0]; // Initial-handshake request context is empty.
    result.extend((bytes.len() as u16).to_be_bytes());
    result.extend(bytes);
    result
}

#[test]
fn request_retains_offers_and_rejects_incomplete_or_duplicate_extensions() {
    use handshake::certificate_request::{
        CertificateRequest,
        CertificateRequestRef,
    };
    let signature_extension = [0, 13, 0, 6, 0, 4, 0x07, 0x08, 0x04, 0x03];
    let unknown_extension = [0xfa, 0xfa, 0, 1, 0xaa];
    let owned: CertificateRequest = {
        let bytes = request(&[&unknown_extension, &signature_extension]);
        CertificateRequestRef::parse(&mut ParseBuffer::new(&bytes)).unwrap().try_into().unwrap()
    };
    assert!(owned.request_context.is_empty());
    assert_eq!(owned.signature_algorithms.as_slice(), &[SignatureScheme::EcdsaSecp256r1Sha256]);
    let mut trailing = request(&[&signature_extension]);
    trailing.push(0);
    let mut truncated = request(&[&signature_extension]);
    truncated.pop();
    for bytes in
        [request(&[]), request(&[&signature_extension, &signature_extension]), trailing, truncated]
    {
        assert!(
            CertificateRequestRef::parse(&mut ParseBuffer::new(&bytes)).is_err(),
            "{bytes:02x?}"
        );
    }
}
