//! Assembly audit helper. Compile and objdump to inspect codegen.
//!
//! ```bash
//! cargo build --release --example asm_audit
//! objdump -d --disassemble-symbols=asm_audit::audit_try_from_32 target/release/examples/asm_audit
//! ```

use nexus_ascii::AsciiString;
use std::hash::{Hash, Hasher};
use std::hint::black_box;

#[unsafe(no_mangle)]
#[inline(never)]
pub fn audit_try_from_32(bytes: &[u8]) -> Option<AsciiString<32>> {
    AsciiString::<32>::try_from_bytes(bytes).ok()
}

#[unsafe(no_mangle)]
#[inline(never)]
pub fn audit_try_from_8(bytes: &[u8]) -> Option<AsciiString<8>> {
    AsciiString::<8>::try_from_bytes(bytes).ok()
}

#[unsafe(no_mangle)]
#[inline(never)]
pub fn audit_hash_32(s: &AsciiString<32>) -> u64 {
    let mut hasher = nohash_hasher::NoHashHasher::<u64>::default();
    s.hash(&mut hasher);
    hasher.finish()
}

#[unsafe(no_mangle)]
#[inline(never)]
pub fn audit_eq_32(a: &AsciiString<32>, b: &AsciiString<32>) -> bool {
    a == b
}

#[unsafe(no_mangle)]
#[inline(never)]
pub fn audit_cmp_32(a: &AsciiString<32>, b: &AsciiString<32>) -> std::cmp::Ordering {
    a.cmp(b)
}

fn main() {
    let input = black_box(b"BTC-USD" as &[u8]);
    let s = black_box(audit_try_from_32(input));
    if let Some(ref s) = s {
        let h = black_box(audit_hash_32(s));
        black_box(h);
        let s2 = audit_try_from_32(b"ETH-USD").unwrap();
        black_box(audit_eq_32(s, &s2));
        black_box(audit_cmp_32(s, &s2));
    }
}
