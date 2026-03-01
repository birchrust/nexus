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
