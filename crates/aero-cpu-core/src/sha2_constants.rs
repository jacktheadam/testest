//! SHA-2 round constants.
//!
//! These are the published FIPS 180-4 constants: the first 32 (SHA-256) or 64
//! (SHA-512) bits of the fractional parts of the cube roots of the first 64 or
//! 80 primes. They are written out here rather than shipped as binary blobs so
//! that the tree carries no opaque data files for something this well-defined —
//! and the accompanying test re-derives them from that definition, so a typo
//! cannot hide.
//!
//! The `_LE` tables are the same values as little-endian bytes, which is the
//! form the guest-code differential tests load into emulated memory.

/// SHA-256 round constants, one per round.
pub const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// SHA-512 round constants, one per round.
pub const SHA512_K: [u64; 80] = [
    0x428a2f98d728ae22,
    0x7137449123ef65cd,
    0xb5c0fbcfec4d3b2f,
    0xe9b5dba58189dbbc,
    0x3956c25bf348b538,
    0x59f111f1b605d019,
    0x923f82a4af194f9b,
    0xab1c5ed5da6d8118,
    0xd807aa98a3030242,
    0x12835b0145706fbe,
    0x243185be4ee4b28c,
    0x550c7dc3d5ffb4e2,
    0x72be5d74f27b896f,
    0x80deb1fe3b1696b1,
    0x9bdc06a725c71235,
    0xc19bf174cf692694,
    0xe49b69c19ef14ad2,
    0xefbe4786384f25e3,
    0x0fc19dc68b8cd5b5,
    0x240ca1cc77ac9c65,
    0x2de92c6f592b0275,
    0x4a7484aa6ea6e483,
    0x5cb0a9dcbd41fbd4,
    0x76f988da831153b5,
    0x983e5152ee66dfab,
    0xa831c66d2db43210,
    0xb00327c898fb213f,
    0xbf597fc7beef0ee4,
    0xc6e00bf33da88fc2,
    0xd5a79147930aa725,
    0x06ca6351e003826f,
    0x142929670a0e6e70,
    0x27b70a8546d22ffc,
    0x2e1b21385c26c926,
    0x4d2c6dfc5ac42aed,
    0x53380d139d95b3df,
    0x650a73548baf63de,
    0x766a0abb3c77b2a8,
    0x81c2c92e47edaee6,
    0x92722c851482353b,
    0xa2bfe8a14cf10364,
    0xa81a664bbc423001,
    0xc24b8b70d0f89791,
    0xc76c51a30654be30,
    0xd192e819d6ef5218,
    0xd69906245565a910,
    0xf40e35855771202a,
    0x106aa07032bbd1b8,
    0x19a4c116b8d2d0c8,
    0x1e376c085141ab53,
    0x2748774cdf8eeb99,
    0x34b0bcb5e19b48a8,
    0x391c0cb3c5c95a63,
    0x4ed8aa4ae3418acb,
    0x5b9cca4f7763e373,
    0x682e6ff3d6b2b8a3,
    0x748f82ee5defb2fc,
    0x78a5636f43172f60,
    0x84c87814a1f0ab72,
    0x8cc702081a6439ec,
    0x90befffa23631e28,
    0xa4506cebde82bde9,
    0xbef9a3f7b2c67915,
    0xc67178f2e372532b,
    0xca273eceea26619c,
    0xd186b8c721c0c207,
    0xeada7dd6cde0eb1e,
    0xf57d4f7fee6ed178,
    0x06f067aa72176fba,
    0x0a637dc5a2c898a6,
    0x113f9804bef90dae,
    0x1b710b35131c471b,
    0x28db77f523047d84,
    0x32caab7b40c72493,
    0x3c9ebe0a15c9bebc,
    0x431d67c49c100d4c,
    0x4cc5d4becb3e42b6,
    0x597f299cfc657e2a,
    0x5fcb6fab3ad6faec,
    0x6c44198c4a475817,
];

const fn u32_table_le(k: &[u32; 64]) -> [u8; 256] {
    let mut out = [0u8; 256];
    let mut i = 0;
    while i < 64 {
        let b = k[i].to_le_bytes();
        out[i * 4] = b[0];
        out[i * 4 + 1] = b[1];
        out[i * 4 + 2] = b[2];
        out[i * 4 + 3] = b[3];
        i += 1;
    }
    out
}

const fn u64_table_le(k: &[u64; 80]) -> [u8; 640] {
    let mut out = [0u8; 640];
    let mut i = 0;
    while i < 80 {
        let b = k[i].to_le_bytes();
        let mut j = 0;
        while j < 8 {
            out[i * 8 + j] = b[j];
            j += 1;
        }
        i += 1;
    }
    out
}

/// `SHA256_K` as little-endian bytes.
pub const SHA256_K_LE: [u8; 256] = u32_table_le(&SHA256_K);

/// `SHA512_K` as little-endian bytes.
pub const SHA512_K_LE: [u8; 640] = u64_table_le(&SHA512_K);

#[cfg(test)]
mod tests {
    use super::*;
    use core::cmp::Ordering;

    /// 256-bit unsigned integer, little-endian limbs. Wide enough for the cube
    /// of a 68-bit value, which is what re-deriving a 64-bit constant needs —
    /// `u128` is not (the SHA-512 case scales by 2^192 before taking a root).
    type U256 = [u64; 4];

    fn mul(a: U256, b: U256) -> U256 {
        let mut out = [0u64; 4];
        for (i, &ai) in a.iter().enumerate() {
            if ai == 0 {
                continue;
            }
            let mut carry = 0u128;
            for (j, &bj) in b.iter().enumerate() {
                if i + j >= 4 {
                    break;
                }
                let cur = out[i + j] as u128 + (ai as u128) * (bj as u128) + carry;
                out[i + j] = cur as u64;
                carry = cur >> 64;
            }
        }
        out
    }

    fn cmp(a: U256, b: U256) -> Ordering {
        for i in (0..4).rev() {
            match a[i].cmp(&b[i]) {
                Ordering::Equal => {}
                other => return other,
            }
        }
        Ordering::Equal
    }

    fn from_u128(v: u128) -> U256 {
        [v as u64, (v >> 64) as u64, 0, 0]
    }

    /// The first `bits` bits of the fractional part of the cube root of `p`,
    /// by integer binary search — no floating point, so the check cannot
    /// inherit a rounding error from the thing it is verifying.
    fn frac_cbrt(p: u64, bits: u32) -> u128 {
        // target = p * 2^(3*bits); for bits = 64 that is p in the top limb.
        let mut target = [0u64; 4];
        let word = (3 * bits) / 64;
        let shift = (3 * bits) % 64;
        target[word as usize] = p << shift;

        let (mut lo, mut hi) = (0u128, 1u128 << (bits.min(63) + 4));
        while lo < hi {
            let mid = (lo + hi + 1) / 2;
            let m = from_u128(mid);
            let cube = mul(mul(m, m), m);
            if cmp(cube, target) != Ordering::Greater {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        if bits == 128 {
            lo
        } else {
            lo & ((1u128 << bits) - 1)
        }
    }

    fn first_primes(n: usize) -> Vec<u64> {
        let mut ps: Vec<u64> = Vec::with_capacity(n);
        let mut c: u64 = 2;
        while ps.len() < n {
            if ps.iter().take_while(|p| *p * *p <= c).all(|p| c % p != 0) {
                ps.push(c);
            }
            c += 1;
        }
        ps
    }

    #[test]
    fn sha256_constants_match_their_definition() {
        for (i, p) in first_primes(64).into_iter().enumerate() {
            assert_eq!(SHA256_K[i], frac_cbrt(p, 32) as u32, "K[{i}] for prime {p}");
        }
    }

    #[test]
    fn sha512_constants_match_their_definition() {
        for (i, p) in first_primes(80).into_iter().enumerate() {
            assert_eq!(SHA512_K[i], frac_cbrt(p, 64) as u64, "K[{i}] for prime {p}");
        }
    }

    #[test]
    fn little_endian_tables_agree_with_the_word_tables() {
        for (i, w) in SHA256_K.iter().enumerate() {
            assert_eq!(&SHA256_K_LE[i * 4..i * 4 + 4], &w.to_le_bytes());
        }
        for (i, w) in SHA512_K.iter().enumerate() {
            assert_eq!(&SHA512_K_LE[i * 8..i * 8 + 8], &w.to_le_bytes());
        }
    }
}
