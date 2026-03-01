use std::ptr;

#[allow(dead_code)]
trait Probe {
    fn probe(&self);
}

struct ProbeImpl;
impl Probe for ProbeImpl {
    fn probe(&self) {}
}

#[repr(C)]
struct FatPtr {
    data: *const u8,
    meta: *const u8,
}

fn main() {
    // This assertion runs on the host. Warn if cross-compiling, since
    // host layout may differ from target layout.
    let host = std::env::var("HOST").unwrap_or_default();
    let target = std::env::var("TARGET").unwrap_or_default();
    if host != target {
        panic!(
            "nexus-smartptr: cross-compilation detected (host={host}, target={target}). \
             Fat pointer layout assertion only validates the host. \
             Cross-compiled targets are not verified."
        );
    }

    let val = ProbeImpl;
    let data_ptr: *const u8 = &raw const val as *const u8;
    let trait_ptr: *const dyn Probe = &val as &dyn Probe;

    let decomposed: FatPtr = unsafe {
        ptr::read(ptr::addr_of!(trait_ptr).cast::<FatPtr>())
    };

    assert!(
        decomposed.data == data_ptr,
        "nexus-smartptr: fat pointer layout is not (data, metadata). \
         This platform is not supported."
    );
}
