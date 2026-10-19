//! Fixed public vectors generated with Python hashlib/hmac, independently
//! of Charlotte's implementation. Boundary lengths exercise padding and block
//! transitions; every split must match the same reference digest.
use charlotte_launch::sha256::{
    Sha256,
    digest,
    hmac_sha256,
};

fn decode(hex: &str) -> [u8; 32] {
    let mut bytes = [0; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[2 * index..2 * index + 2], 16).unwrap();
    }
    bytes
}

#[test]
fn sha256_matches_reference_across_block_and_padding_boundaries() {
    for (len, expected) in [
        (0, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
        (1, "6e340b9cffb37a989ca544e6bb780a2c78901d3fb33738768511a30617afa01d"),
        (7, "57355ac3303c148f11aef7cb179456b9232cde33a818dfda2c2fcb9325749a6b"),
        (31, "4f23c2ca8c5c962e50cd31e221bfb6d0adca19111dca8e0c62598ff146dd19c4"),
        (55, "463eb28e72f82e0a96c0a4cc53690c571281131f672aa229e0d45ae59b598b59"),
        (56, "da2ae4d6b36748f2a318f23e7ab1dfdf45acdc9d049bd80e59de82a60895f562"),
        (63, "29af2686fd53374a36b0846694cc342177e428d1647515f078784d69cdb9e488"),
        (64, "fdeab9acf3710362bd2658cdc9a29e8f9c757fcf9811603a8c447cd1d9151108"),
        (65, "4bfd2c8b6f1eec7a2afeb48b934ee4b2694182027e6d0fc075074f2fabb31781"),
        (119, "da18797ed7c3a777f0847f429724a2d8cd5138e6ed2895c3fa1a6d39d18f7ec6"),
        (120, "f52b23db1fbb6ded89ef42a23ce0c8922c45f25c50b568a93bf1c075420bbb7c"),
        (127, "92ca0fa6651ee2f97b884b7246a562fa71250fedefe5ebf270d31c546bfea976"),
        (128, "471fb943aa23c511f6f72f8d1652d9c880cfa392ad80503120547703e56a2be5"),
        (129, "5099c6a56203f9687f7d33f4bfdf576d31dc91f6b695ecea38b2770c87631135"),
        (257, "a2c6cad2ffa699b14538231fef914f45d30440389e6f8d79091efba836165a2b"),
        (1024, "2bce1ba628720664be4b9fdd77aae0678e5f0f3f02fc6ff641ec879094f6a404"),
    ] {
        let bytes: Vec<u8> = (0..len).map(|index| (index % 251) as u8).collect();
        let expected = decode(expected);
        assert_eq!(digest(&bytes), expected, "whole input length {len}");
        for split in 0..=len {
            let mut hash = Sha256::new();
            hash.update(&bytes[..split]);
            hash.update(&[]);
            hash.update(&bytes[split..]);
            assert_eq!(hash.finalize(), expected, "length {len}, split {split}");
        }
        for chunk_size in [1, 7, 31, 55, 64, 65, 129] {
            let mut hash = Sha256::new();
            for chunk in bytes.chunks(chunk_size) {
                hash.update(chunk);
            }
            assert_eq!(hash.finalize(), expected, "length {len}, chunk {chunk_size}");
        }
    }
}

#[test]
fn hmac_matches_reference_with_short_and_hashed_keys() {
    let message: Vec<u8> = (0..129).map(|index| (index % 251) as u8).collect();
    for (len, expected) in [
        (0, "c3ad09df1d5ade409091a67f747147417c38c0b15efaca8c195e7f76a3cbc519"),
        (1, "c3ad09df1d5ade409091a67f747147417c38c0b15efaca8c195e7f76a3cbc519"),
        (32, "52cc48f5d76260a9df98c5e171fea39acc0aad5f5833899b5313a47965e71fad"),
        (63, "f2d68b260aebd26969043402f5d297f7fe973800125c44f8fa63d3e59c100500"),
        (64, "335e54597fb47ebfcffcaaa3a1711076bc8952532d562bf31c9bd3101de46dfa"),
        (65, "dd6235fb28670d7f90d31c390b2c15014ae42adeabbc338d3a15cffe8e1f9103"),
        (131, "893bdd6587edcb87b6da3f631ccf344b762361e772f4cf487b0cc0d599d7f654"),
    ] {
        let key: Vec<u8> = (0..len).map(|index| (index % 251) as u8).collect();
        let result: zeroize::Zeroizing<[u8; 32]> = hmac_sha256(&key, &message);
        assert_eq!(*result, decode(expected), "key length {len}");
    }
}
