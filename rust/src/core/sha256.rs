//! Hand-rolled SHA-256 (FIPS 180-4), no TS counterpart - see
//! `docs/rust-port.md`. Replaces `node:crypto`'s `createHash("sha256")`,
//! used by trust hashing (`core/trust.ts`), tree fingerprints
//! (`core/git.ts`), plan content hashes (`artifacts/plan.ts`), and gitignore
//! state (`core/gitignoreState.ts`). Every call site formats the digest as
//! `"sha256:" + hex` (lowercase); `sha256_prefixed` below does that directly.
//!
//! Pinned against the FIPS 180-4 published test vectors ("abc",
//! two-block message) and against real `node -e
//! 'require("crypto").createHash("sha256")...'` output for the empty
//! string, a >64-byte input, and a multi-block input.

const H0: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// One block's worth of compression, mutating `state` in place.
fn compress(state: &mut [u32; 8], block: &[u8]) {
    debug_assert_eq!(block.len(), 64);
    let mut w = [0u32; 64];
    for (i, chunk) in block.as_chunks::<4>().0.iter().enumerate() {
        w[i] = u32::from_be_bytes(*chunk);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }

    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;

    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let temp1 = h
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(K[i])
            .wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(maj);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }

    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
    state[5] = state[5].wrapping_add(f);
    state[6] = state[6].wrapping_add(g);
    state[7] = state[7].wrapping_add(h);
}

/// Raw 32-byte SHA-256 digest of `data`.
pub fn digest(data: &[u8]) -> [u8; 32] {
    let mut state = H0;
    let bit_len: u64 = (data.len() as u64).wrapping_mul(8);

    // Padding: 0x80, then zeros, then the 64-bit big-endian bit length, so
    // the total length is a multiple of 64 bytes.
    let mut padded = Vec::with_capacity(data.len() + 72);
    padded.extend_from_slice(data);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for block in padded.as_chunks::<64>().0 {
        compress(&mut state, block);
    }

    let mut out = [0u8; 32];
    for (i, word) in state.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// Lowercase hex of `digest(data)`, matching `Buffer#digest("hex")`.
pub fn hex_digest(data: &[u8]) -> String {
    let d = digest(data);
    let mut out = String::with_capacity(64);
    for byte in d {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// `"sha256:" + hex`, the format every gate call site uses (trust hashing,
/// tree fingerprints, plan content hashes, gitignore state).
pub fn sha256_prefixed(data: &[u8]) -> String {
    format!("sha256:{}", hex_digest(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_fips_180_4_empty_string_vector() {
        assert_eq!(
            hex_digest(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn matches_fips_180_4_abc_vector() {
        assert_eq!(
            hex_digest(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn matches_fips_180_4_two_block_message_vector() {
        // "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
        assert_eq!(
            hex_digest(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn matches_node_crypto_for_a_greater_than_64_byte_input() {
        // node -e 'console.log(require("crypto").createHash("sha256").update("a".repeat(65)).digest("hex"))'
        let input = "a".repeat(65);
        assert_eq!(
            hex_digest(input.as_bytes()),
            "635361c48bb9eab14198e76ea8ab7f1a41685d6ad62aa9146d301d4f17eb0ae0"
        );
    }

    #[test]
    fn matches_node_crypto_for_exactly_64_bytes() {
        let input = "a".repeat(64);
        assert_eq!(
            hex_digest(input.as_bytes()),
            "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb"
        );
    }

    #[test]
    fn matches_node_crypto_for_a_multi_block_input() {
        // node -e 'console.log(require("crypto").createHash("sha256")
        //   .update("The quick brown fox jumps over the lazy dog. ".repeat(10))
        //   .digest("hex"))'
        let input = "The quick brown fox jumps over the lazy dog. ".repeat(10);
        assert_eq!(
            hex_digest(input.as_bytes()),
            "67e8e9c79772f865398c51be8822e35fe17a35131d81d78392a2c35b45384d4b"
        );
    }

    #[test]
    fn matches_node_crypto_for_unicode_input_hashed_as_utf8() {
        // node -e 'console.log(require("crypto").createHash("sha256").update("héllo 日本語", "utf8").digest("hex"))'
        let input = "héllo 日本語";
        assert_eq!(
            hex_digest(input.as_bytes()),
            "d92882823120aba062255c254504244cc53894c0134283318a27637baa5ac174"
        );
    }

    #[test]
    fn sha256_prefixed_matches_every_call_sites_format() {
        // node -e 'console.log("sha256:" + require("crypto").createHash("sha256").update("hello world").digest("hex"))'
        assert_eq!(
            sha256_prefixed(b"hello world"),
            "sha256:b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }
}
