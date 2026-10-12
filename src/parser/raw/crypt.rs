//! PDF decryption support (Standard Security Handler, R2-R6).
//!
//! Implements password verification and key derivation per the PDF spec,
//! plus per-object decryption using RC4, AES-128-CBC or AES-256-CBC.

use md5::{Digest, Md5};
use rc4::{KeyInit, Rc4, StreamCipher};
use sha2::{Sha256, Sha384, Sha512};

/// How a document's strings and streams are encrypted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cipher {
    /// RC4 with a per-object key (R2-R4, `/V` 1-2, or crypt filter `/V2`).
    Rc4,
    /// AES-128-CBC with a per-object key (R4, crypt filter `/AESV2`).
    Aes128,
    /// AES-256-CBC with the file key itself (R5-R6, crypt filter `/AESV3`).
    Aes256,
}

/// PDF encryption parameters parsed from the /Encrypt dictionary.
#[derive(Debug, Clone)]
pub struct EncryptionParams {
    /// /V — algorithm version (1, 2, or 4 for R2-R4).
    pub version: u32,
    /// /R — Standard security handler revision (2, 3, or 4).
    pub revision: u32,
    /// /Length — encryption key length in bits (default 40).
    pub key_length: u32,
    /// /O — owner password hash (32 bytes for R2-R4).
    pub owner_hash: Vec<u8>,
    /// /U — user password hash (32 bytes for R2-R4).
    pub user_hash: Vec<u8>,
    /// /P — permissions flags.
    pub permissions: i32,
    /// First element of trailer /ID array.
    pub file_id: Vec<u8>,
    /// How strings and streams are encrypted.
    pub cipher: Cipher,
    /// /OE — the file key encrypted with a key from the owner password (R5-R6, 32 bytes).
    pub owner_key: Vec<u8>,
    /// /UE — the file key encrypted with a key from the user password (R5-R6, 32 bytes).
    pub user_key: Vec<u8>,
    /// Whether document metadata is encrypted (/EncryptMetadata, default true).
    pub encrypt_metadata: bool,
}

/// The standard 32-byte padding used in PDF encryption (ISO 32000-1, Table 20).
const PADDING: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08,
    0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

/// Derive the file encryption key from a user password (Algorithm 2, PDF spec).
/// Works for Standard Security Handler R2-R4.
pub fn compute_encryption_key(params: &EncryptionParams, password: &[u8]) -> Vec<u8> {
    // 40 to 128 bits (ISO 32000-1, Table 21). `/Length` is the document's word: outside that
    // range a zero-length key would make RC4 refuse it, and more than the 16 bytes MD5 gives
    // would slice past the hash.
    let key_len = (params.key_length / 8).clamp(5, 16) as usize;

    // Step a: Pad or truncate password to exactly 32 bytes
    let mut padded = Vec::with_capacity(32);
    let take = password.len().min(32);
    padded.extend_from_slice(&password[..take]);
    if padded.len() < 32 {
        padded.extend_from_slice(&PADDING[..32 - padded.len()]);
    }

    // Steps b-f: MD5(padded || O || P || fileID [|| 0xFFFFFFFF])
    let mut hasher = Md5::new();
    hasher.update(&padded);
    hasher.update(&params.owner_hash);
    hasher.update(params.permissions.to_le_bytes());
    hasher.update(&params.file_id);
    // Step f (R>=4): when metadata is not encrypted, append 4 bytes of 0xFF
    if params.revision >= 4 && !params.encrypt_metadata {
        hasher.update([0xFFu8; 4]);
    }

    let mut hash = hasher.finalize().to_vec();

    // Step g: For R >= 3, re-hash 50 times (using only key_len bytes)
    if params.revision >= 3 {
        for _ in 0..50 {
            let mut h = Md5::new();
            h.update(&hash[..key_len]);
            hash = h.finalize().to_vec();
        }
    }

    hash.truncate(key_len);
    hash
}

/// Verify user password and return the encryption key if correct.
/// Algorithm 6 (R2) / Algorithm 7 (R3-R4) from the PDF spec.
pub fn authenticate_user_password(params: &EncryptionParams, password: &[u8]) -> Option<Vec<u8>> {
    let key = compute_encryption_key(params, password);

    if params.revision == 2 {
        // Algorithm 4: RC4-encrypt the 32-byte padding with the key
        let encrypted = rc4_crypt(&key, &PADDING);
        if encrypted[..] == params.user_hash[..32.min(params.user_hash.len())] {
            return Some(key);
        }
    } else if params.revision >= 3 && params.revision <= 4 {
        // Algorithm 5: MD5(padding || fileID), then 20 rounds of RC4
        let mut hasher = Md5::new();
        hasher.update(PADDING);
        hasher.update(&params.file_id);
        let hash = hasher.finalize();

        let mut encrypted = hash.to_vec();
        encrypted = rc4_crypt(&key, &encrypted);

        // 19 additional RC4 passes with XOR-modified keys
        for i in 1..=19u8 {
            let modified_key: Vec<u8> = key.iter().map(|&b| b ^ i).collect();
            encrypted = rc4_crypt(&modified_key, &encrypted);
        }

        // Compare first 16 bytes only (rest is random padding)
        if encrypted.len() >= 16
            && params.user_hash.len() >= 16
            && encrypted[..16] == params.user_hash[..16]
        {
            return Some(key);
        }
    }

    None
}

/// RC4 encrypt/decrypt (symmetric operation). A key RC4 cannot take leaves the data as it is.
fn rc4_crypt(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut output = data.to_vec();
    if let Ok(mut cipher) = Rc4::new_from_slice(key) {
        cipher.apply_keystream(&mut output);
    }
    output
}

/// Authenticate `password` against a revision 5 or 6 document and return its file key
/// (ISO 32000-2, Algorithm 2.A): as the user password, then as the owner password.
///
/// The password is taken as UTF-8 bytes, at most 127 of them; the SASLprep normalization the
/// standard asks for is not applied, which matters only for passwords outside ASCII that
/// normalization would change.
pub fn authenticate_aes256(params: &EncryptionParams, password: &[u8]) -> Option<Vec<u8>> {
    let password = &password[..password.len().min(127)];
    let (u, o) = (&params.user_hash, &params.owner_hash);
    if u.len() < 48 || o.len() < 48 || params.user_key.len() < 32 || params.owner_key.len() < 32 {
        return None;
    }
    let hash = |salt: &[u8], udata: &[u8]| hardened_hash(params.revision, password, salt, udata);
    // User: /U is hash ‖ validation salt ‖ key salt, with no user data.
    if hash(&u[32..40], &[]) == u[..32] {
        return aes256_unwrap(&hash(&u[40..48], &[]), &params.user_key[..32]);
    }
    // Owner: the same over /O, with the 48 bytes of /U as user data.
    if hash(&o[32..40], &u[..48]) == o[..32] {
        return aes256_unwrap(&hash(&o[40..48], &u[..48]), &params.owner_key[..32]);
    }
    None
}

/// The password hash of revision 5 (SHA-256 of password ‖ salt ‖ user data) and revision 6
/// (ISO 32000-2, Algorithm 2.B — rounds of AES-128-CBC and SHA-2 until the last byte of the
/// round's output allows a stop, at least 64 rounds).
fn hardened_hash(revision: u32, password: &[u8], salt: &[u8], udata: &[u8]) -> [u8; 32] {
    let mut k: Vec<u8> = Sha256::new()
        .chain_update(password)
        .chain_update(salt)
        .chain_update(udata)
        .finalize()
        .to_vec();
    if revision == 6 {
        let mut round = 0u32;
        loop {
            let mut k1 = Vec::with_capacity(64 * (password.len() + k.len() + udata.len()));
            for _ in 0..64 {
                k1.extend_from_slice(password);
                k1.extend_from_slice(&k);
                k1.extend_from_slice(udata);
            }
            let e = aes128_cbc_encrypt(&k[..16], &k[16..32], &k1);
            // The first 16 bytes of E as a big-endian number, modulo 3: since 256 ≡ 1 (mod 3),
            // that is the sum of the bytes modulo 3.
            let selector = e[..16].iter().map(|&b| u32::from(b)).sum::<u32>() % 3;
            k = match selector {
                0 => Sha256::digest(&e).to_vec(),
                1 => Sha384::digest(&e).to_vec(),
                _ => Sha512::digest(&e).to_vec(),
            };
            round += 1;
            let last = u32::from(*e.last().unwrap_or(&0));
            if round >= 64 && last <= round - 32 {
                break;
            }
        }
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&k[..32]);
    out
}

/// AES-128-CBC encryption without padding; `data` is a whole number of blocks (Algorithm
/// 2.B builds it as 64 repetitions of a block-aligned sequence).
fn aes128_cbc_encrypt(key: &[u8], iv: &[u8], data: &[u8]) -> Vec<u8> {
    use aes::cipher::{BlockCipherEncrypt, KeyInit as _};
    use aes::Aes128;

    let Ok(cipher) = Aes128::new_from_slice(key) else {
        return Vec::new();
    };
    let mut chain = [0u8; 16];
    chain.copy_from_slice(&iv[..16]);
    let mut out = Vec::with_capacity(data.len());
    for block in data.chunks(16) {
        let mut buf = [0u8; 16];
        for (i, b) in block.iter().enumerate() {
            buf[i] = b ^ chain[i];
        }
        let mut arr = buf.into();
        cipher.encrypt_block(&mut arr);
        chain.copy_from_slice(&arr);
        out.extend_from_slice(&chain);
    }
    out
}

/// Decrypt `/UE` or `/OE` (32 bytes) with the 32-byte key derived from a password: AES-256
/// in CBC mode with a zero IV and no padding.
fn aes256_unwrap(key: &[u8; 32], wrapped: &[u8]) -> Option<Vec<u8>> {
    use aes::Aes256;
    use cbc::cipher::{block_padding, BlockModeDecrypt, KeyIvInit};

    let mut buf = wrapped.to_vec();
    cbc::Decryptor::<Aes256>::new_from_slices(key, &[0u8; 16])
        .ok()?
        .decrypt_padded::<block_padding::NoPadding>(&mut buf)
        .ok()
        .map(<[u8]>::to_vec)
}

/// Compute per-object decryption key (Algorithm 1 from the PDF spec).
///
/// file_key + obj_num (3 LE bytes) + gen_num (2 LE bytes) [+ "sAlT" for AES]
/// hashed with MD5, truncated to min(file_key.len()+5, 16).
pub fn object_key(file_key: &[u8], obj_num: u32, gen_num: u16, cipher: Cipher) -> Vec<u8> {
    // AES-256 encrypts every object with the file key itself (ISO 32000-2, 7.6.3.3).
    if cipher == Cipher::Aes256 {
        return file_key.to_vec();
    }
    let use_aes = cipher == Cipher::Aes128;
    let mut hasher = Md5::new();
    hasher.update(file_key);
    hasher.update(&obj_num.to_le_bytes()[..3]);
    hasher.update(&gen_num.to_le_bytes()[..2]);
    if use_aes {
        // AES salt per spec
        hasher.update(b"sAlT");
    }
    let hash = hasher.finalize();
    let key_len = (file_key.len() + 5).min(16);
    hash[..key_len].to_vec()
}

/// Decrypt a byte sequence using RC4.
pub fn decrypt_rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    rc4_crypt(key, data)
}

/// Decrypt a byte sequence using AES-CBC — AES-128 for a 16-byte key, AES-256 for a 32-byte
/// one. The first 16 bytes of `data` are the IV; the remainder is ciphertext.
pub fn decrypt_aes(key: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    if key.len() == 32 {
        return decrypt_aes256(key, data);
    }
    decrypt_aes128(key, data)
}

fn decrypt_aes256(key: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    use aes::Aes256;
    use cbc::cipher::{block_padding, BlockModeDecrypt, KeyIvInit};

    if data.len() < 16 || !data.len().is_multiple_of(16) {
        return None;
    }
    let (iv, ciphertext) = data.split_at(16);
    if ciphertext.is_empty() {
        return Some(vec![]);
    }
    let decrypt_with = |pkcs7: bool| -> Option<Vec<u8>> {
        let mut buf = ciphertext.to_vec();
        let decryptor = cbc::Decryptor::<Aes256>::new_from_slices(key, iv).ok()?;
        let plaintext = if pkcs7 {
            decryptor.decrypt_padded::<block_padding::Pkcs7>(&mut buf)
        } else {
            decryptor.decrypt_padded::<block_padding::NoPadding>(&mut buf)
        };
        plaintext.ok().map(<[u8]>::to_vec)
    };
    decrypt_with(true).or_else(|| decrypt_with(false))
}

/// Decrypt a byte sequence using AES-128-CBC.
/// The first 16 bytes of `data` are the IV; the remainder is ciphertext.
pub fn decrypt_aes128(key: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    use aes::Aes128;
    use cbc::cipher::{block_padding, BlockModeDecrypt, KeyIvInit};

    if data.len() < 16 || !data.len().is_multiple_of(16) {
        return None;
    }

    let iv = &data[..16];
    let ciphertext = &data[16..];

    if ciphertext.is_empty() {
        return Some(vec![]);
    }

    type Aes128CbcDec = cbc::Decryptor<Aes128>;

    // `object_key` truncates to `min(file_key.len() + 5, 16)`, so a document whose file key is
    // shorter than 11 bytes yields a key this cipher cannot take. Building the decryptor from
    // slices reports that as a length error, which this function already has a way to express;
    // converting the slice directly would panic on it instead, on a path fed by untrusted input.
    let decrypt_with = |padded: &mut Vec<u8>, pkcs7: bool| -> Option<Vec<u8>> {
        let decryptor = Aes128CbcDec::new_from_slices(key, iv).ok()?;
        let plaintext = if pkcs7 {
            decryptor.decrypt_padded::<block_padding::Pkcs7>(padded)
        } else {
            decryptor.decrypt_padded::<block_padding::NoPadding>(padded)
        };
        plaintext.ok().map(<[u8]>::to_vec)
    };

    // Try PKCS7 first
    let mut buf = ciphertext.to_vec();
    if let Some(plaintext) = decrypt_with(&mut buf, true) {
        return Some(plaintext);
    }

    // Fallback: no padding (some PDFs omit PKCS7)
    let mut buf2 = ciphertext.to_vec();
    decrypt_with(&mut buf2, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex digit pair"))
            .collect()
    }

    /// NIST SP 800-38A, F.2.2 (CBC-AES128.Decrypt), first two blocks.
    ///
    /// A known answer rather than a round-trip: encrypting and decrypting with the same crate
    /// agrees with itself even when both halves are wrong, which is exactly what a dependency
    /// upgrade could break without anything noticing.
    #[test]
    fn aes_128_cbc_matches_the_nist_known_answer() {
        let key = hex("2b7e151628aed2a6abf7158809cf4f3c");
        let iv = hex("000102030405060708090a0b0c0d0e0f");
        let ciphertext = hex("7649abac8119b246cee98e9b12e9197d\
             5086cb9b507219ee95db113a917678b2");
        let expected = hex("6bc1bee22e409f96e93d7e117393172a\
             ae2d8a571e03ac9c9eb76fac45af8e51");

        // The function takes the IV prepended to the ciphertext, as PDF stores it.
        let mut data = iv;
        data.extend_from_slice(&ciphertext);

        // These blocks carry no PKCS7 padding, so the no-padding fallback is what answers.
        assert_eq!(decrypt_aes128(&key, &data), Some(expected));
    }

    #[test]
    fn a_pkcs7_padded_payload_comes_back_without_its_padding() {
        // Same key/IV; ciphertext produced from "unparser" padded to one block under PKCS7.
        let key = hex("2b7e151628aed2a6abf7158809cf4f3c");
        let iv = hex("000102030405060708090a0b0c0d0e0f");

        use aes::Aes128;
        use cbc::cipher::{block_padding::Pkcs7, BlockModeEncrypt, KeyIvInit};
        let plaintext = b"unparser";
        let mut buf = [0u8; 16];
        buf[..plaintext.len()].copy_from_slice(plaintext);
        let ct = cbc::Encryptor::<Aes128>::new_from_slices(&key, &iv)
            .expect("16-byte key and iv")
            .encrypt_padded::<Pkcs7>(&mut buf, plaintext.len())
            .expect("one block of room")
            .to_vec();

        let mut data = iv;
        data.extend_from_slice(&ct);

        assert_eq!(decrypt_aes128(&key, &data).as_deref(), Some(&plaintext[..]));
    }

    /// `object_key` truncates to `min(file_key.len() + 5, 16)`, so a document whose file key is
    /// shorter than 11 bytes produces a key AES-128 cannot take. Before the cipher 0.5 migration
    /// this reached a slice-to-array conversion that panics on a length mismatch -- on a path
    /// whose input is an untrusted file.
    /// `/Length` is the document's word. A zero length made RC4 refuse an empty key with a
    /// panic; a length past 128 bits sliced past the 16 bytes MD5 gives. Both are untrusted
    /// input, so both come out as a key that simply does not authenticate.
    #[test]
    fn a_key_length_outside_40_to_128_bits_does_not_panic() {
        for bits in [0, 8, 256, 4096, u32::MAX] {
            let p = EncryptionParams {
                key_length: bits,
                ..params(3, true, vec![0; 32])
            };
            let key = compute_encryption_key(&p, b"");
            assert!((5..=16).contains(&key.len()), "{bits}: {}", key.len());
            assert!(authenticate_user_password(&p, b"").is_none());
        }
        assert_eq!(rc4_crypt(&[], b"abc"), b"abc");
    }

    #[test]
    fn a_key_the_cipher_cannot_take_is_refused_rather_than_panicking() {
        let short_key = object_key(&[0xAB; 4], 1, 0, Cipher::Aes128);
        assert!(
            short_key.len() < 16,
            "the truncation rule should yield 9 bytes here"
        );

        let data = vec![0u8; 32]; // a well-formed IV + one ciphertext block
        assert_eq!(decrypt_aes128(&short_key, &data), None);
    }

    #[test]
    fn a_payload_that_is_not_a_whole_number_of_blocks_is_refused() {
        let key = hex("2b7e151628aed2a6abf7158809cf4f3c");
        assert_eq!(decrypt_aes128(&key, &[0u8; 20]), None);
        assert_eq!(decrypt_aes128(&key, &[0u8; 8]), None);
    }

    #[test]
    fn an_iv_with_no_ciphertext_after_it_decrypts_to_nothing() {
        let key = hex("2b7e151628aed2a6abf7158809cf4f3c");
        assert_eq!(decrypt_aes128(&key, &[0u8; 16]), Some(vec![]));
    }

    /// Fixed inputs for the key-derivation known answers below: password `user`, a 128-bit
    /// key, `/O` = bytes 0..32, `/P` = -3904, first `/ID` element = bytes 0xA0..0xB0.
    fn params(revision: u32, encrypt_metadata: bool, user_hash: Vec<u8>) -> EncryptionParams {
        EncryptionParams {
            version: if revision >= 4 { 4 } else { 2 },
            revision,
            key_length: 128,
            owner_hash: (0u8..32).collect(),
            user_hash,
            permissions: -3904,
            file_id: (0xA0u8..0xB0).collect(),
            cipher: Cipher::Rc4,
            owner_key: Vec::new(),
            user_key: Vec::new(),
            encrypt_metadata,
        }
    }

    const KEY_WITH_METADATA: &str = "487b5108a900b1d3db085f6af0dcd409";
    const KEY_WITHOUT_METADATA: &str = "349c930e1743c5ba5db910fb4a2d4aab";

    /// Algorithm 2, step f: at revision 4, `/EncryptMetadata false` appends four 0xFF bytes to
    /// the hash input, so the file key differs from the default. The expected keys were
    /// computed from the spec with an independent implementation, not with this code.
    #[test]
    fn unencrypted_metadata_changes_the_revision_4_file_key() {
        assert_eq!(
            compute_encryption_key(&params(4, true, vec![]), b"user"),
            hex(KEY_WITH_METADATA)
        );
        assert_eq!(
            compute_encryption_key(&params(4, false, vec![]), b"user"),
            hex(KEY_WITHOUT_METADATA)
        );
    }

    /// Step f applies from revision 4 on; at revision 3 the flag changes nothing.
    #[test]
    fn revision_3_ignores_the_encrypt_metadata_flag() {
        for encrypt_metadata in [true, false] {
            assert_eq!(
                compute_encryption_key(&params(3, encrypt_metadata, vec![]), b"user"),
                hex(KEY_WITH_METADATA)
            );
        }
    }

    /// The password check runs on that key: a `/U` written for a document whose metadata is
    /// not encrypted authenticates only when the flag is honoured, and never with the wrong
    /// password.
    #[test]
    fn a_user_hash_for_unencrypted_metadata_needs_the_flag_to_authenticate() {
        let user_hash = hex("766ae16c276153e66056c89cd6d8d8da00000000000000000000000000000000");

        assert_eq!(
            authenticate_user_password(&params(4, false, user_hash.clone()), b"user"),
            Some(hex(KEY_WITHOUT_METADATA))
        );
        assert_eq!(
            authenticate_user_password(&params(4, true, user_hash.clone()), b"user"),
            None
        );
        assert_eq!(
            authenticate_user_password(&params(4, false, user_hash), b"wrong"),
            None
        );
    }
}
