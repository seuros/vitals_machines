//! Experimental private Android battery-properties protocol.
use std::ffi::{CStr, c_char, c_void};
use std::sync::OnceLock;

type Ptr = *mut c_void;
type Create = unsafe extern "C" fn(Ptr) -> Ptr;
type Destroy = unsafe extern "C" fn(Ptr);
type Handler = unsafe extern "C" fn(Ptr, u32, Ptr, Ptr) -> i32;

struct Api {
    class: usize,
    check: unsafe extern "C" fn(*const c_char) -> Ptr,
    associate: unsafe extern "C" fn(Ptr, Ptr) -> bool,
    prepare: unsafe extern "C" fn(Ptr, *mut Ptr) -> i32,
    transact: unsafe extern "C" fn(Ptr, u32, *mut Ptr, *mut Ptr, u32) -> i32,
    write: unsafe extern "C" fn(Ptr, i32) -> i32,
    read32: unsafe extern "C" fn(Ptr, *mut i32) -> i32,
    read64: unsafe extern "C" fn(Ptr, *mut i64) -> i32,
    delete: unsafe extern "C" fn(Ptr),
    release: unsafe extern "C" fn(Ptr),
}

const unsafe extern "C" fn create(data: Ptr) -> Ptr {
    data
}
const unsafe extern "C" fn destroy(_: Ptr) {}
const unsafe extern "C" fn handle(_: Ptr, _: u32, _: Ptr, _: Ptr) -> i32 {
    -74 // STATUS_UNKNOWN_TRANSACTION; this class is never used as a server.
}

impl Api {
    fn load() -> Option<Self> {
        // SAFETY: the library name is NUL-terminated. A successful load is
        // retained for process lifetime because Binder owns the class object.
        let lib = unsafe { libc::dlopen(c"libbinder_ndk.so".as_ptr(), libc::RTLD_NOW) };
        if lib.is_null() {
            return None;
        }
        macro_rules! symbol {
            ($name:expr, $ty:ty) => {{
                // SAFETY: lib is a live handle and the symbol name is terminated.
                let ptr = unsafe { libc::dlsym(lib, concat!($name, "\0").as_ptr().cast()) };
                if ptr.is_null() {
                    // SAFETY: no class was registered and lib was opened above.
                    unsafe { libc::dlclose(lib) };
                    return None;
                }
                // SAFETY: the type is the corresponding NDK function signature.
                unsafe { std::mem::transmute::<Ptr, $ty>(ptr) }
            }};
        }
        let check = symbol!(
            "AServiceManager_checkService",
            unsafe extern "C" fn(*const c_char) -> Ptr
        );
        let associate = symbol!(
            "AIBinder_associateClass",
            unsafe extern "C" fn(Ptr, Ptr) -> bool
        );
        let prepare = symbol!(
            "AIBinder_prepareTransaction",
            unsafe extern "C" fn(Ptr, *mut Ptr) -> i32
        );
        let transact = symbol!(
            "AIBinder_transact",
            unsafe extern "C" fn(Ptr, u32, *mut Ptr, *mut Ptr, u32) -> i32
        );
        let write = symbol!("AParcel_writeInt32", unsafe extern "C" fn(Ptr, i32) -> i32);
        let read32 = symbol!(
            "AParcel_readInt32",
            unsafe extern "C" fn(Ptr, *mut i32) -> i32
        );
        let read64 = symbol!(
            "AParcel_readInt64",
            unsafe extern "C" fn(Ptr, *mut i64) -> i32
        );
        let delete = symbol!("AParcel_delete", unsafe extern "C" fn(Ptr));
        let release = symbol!("AIBinder_decStrong", unsafe extern "C" fn(Ptr));
        let define = symbol!(
            "AIBinder_Class_define",
            unsafe extern "C" fn(*const c_char, Create, Destroy, Handler) -> Ptr
        );
        // SAFETY: callbacks have NDK signatures and the descriptor has static lifetime.
        let class = unsafe {
            define(
                c"android.os.IBatteryPropertiesRegistrar".as_ptr(),
                create,
                destroy,
                handle,
            )
        };
        if class.is_null() {
            // SAFETY: no class was registered.
            unsafe { libc::dlclose(lib) };
            return None;
        }
        Some(Self {
            class: class as usize,
            check,
            associate,
            prepare,
            transact,
            write,
            read32,
            read64,
            delete,
            release,
        })
    }
}

struct Transaction<'a> {
    api: &'a Api,
    binder: Ptr,
    input: Ptr,
    output: Ptr,
}

impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        // SAFETY: non-null parcels and binder are owned by this transaction.
        unsafe {
            if !self.input.is_null() {
                (self.api.delete)(self.input);
            }
            if !self.output.is_null() {
                (self.api.delete)(self.output);
            }
            if !self.binder.is_null() {
                (self.api.release)(self.binder);
            }
        }
    }
}

pub fn capacity() -> Option<i64> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    let api = API.get_or_init(Api::load).as_ref()?;
    let name: &CStr = c"batteryproperties";
    // SAFETY: service name is terminated; result is an owned strong reference.
    let binder = unsafe { (api.check)(name.as_ptr()) };
    if binder.is_null() {
        return None;
    }
    let mut tx = Transaction {
        api,
        binder,
        input: std::ptr::null_mut(),
        output: std::ptr::null_mut(),
    };
    // SAFETY: pointers are valid for the duration of the call, class is
    // registered once, and transact consumes input as specified by the NDK.
    let valid = unsafe {
        (api.associate)(tx.binder, api.class as Ptr)
            && (api.prepare)(tx.binder, &raw mut tx.input) == 0
            && (api.write)(tx.input, 4) == 0
            && (api.transact)(tx.binder, 1, &raw mut tx.input, &raw mut tx.output, 0) == 0
            && !tx.output.is_null()
    };
    if !valid {
        return None;
    }
    super::decode_battery_reply(
        || {
            let mut value = 0;
            // SAFETY: output is a live reply parcel, and value is writable.
            (unsafe { (api.read32)(tx.output, &raw mut value) } == 0).then_some(value)
        },
        || {
            let mut value = 0;
            // SAFETY: output is a live reply parcel, and value is writable.
            (unsafe { (api.read64)(tx.output, &raw mut value) } == 0).then_some(value)
        },
    )
}
