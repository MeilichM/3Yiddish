//! A Windows graphics-card build that still opens on a computer without a graphics driver.
//!
//! `vulkan-1.dll` comes with the graphics driver. build.rs delay-loads it, so Windows loads it
//! on first use and not when the program starts. Without this file, a computer without the DLL
//! would stop at that first use instead. The hook below runs at that point: it hands ggml a
//! `vkGetInstanceProcAddr` that answers "no Vulkan", ggml's Vulkan start-up fails the normal
//! way (an error it catches), no GPU is listed, and the app greys out GPU and uses the CPU.
//!
//! ggml calls only `vkGetInstanceProcAddr` directly from the DLL. It looks up every other
//! Vulkan function through that one, and the first one it asks for is
//! `vkEnumerateInstanceVersion`.

use std::ffi::{CStr, c_char, c_void};

/// delayimp.h's `DelayLoadInfo`.
#[repr(C)]
struct DelayLoadInfo {
    cb: u32,
    pidd: *const c_void,
    ppfn: *mut *const c_void,
    sz_dll: *const c_char,
    /// `DelayLoadProc`: whether by name, then the name (or the ordinal).
    import_by_name: i32,
    proc_name: *const c_char,
    hmod_cur: *mut c_void,
    pfn_cur: *const c_void,
    last_error: u32,
}

const DLI_FAIL_LOAD_LIB: u32 = 3;
const DLI_FAIL_GET_PROC: u32 = 4;
const VK_ERROR_INITIALIZATION_FAILED: i32 = -3;

type Hook = unsafe extern "system" fn(u32, *const DelayLoadInfo) -> *const c_void;

/// Read by the delay-load helper (delayimp.lib) when a DLL or a function in it is missing.
#[unsafe(no_mangle)]
#[used]
static __pfnDliFailureHook2: Option<Hook> = Some(failure_hook);

unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
}

unsafe extern "system" fn failure_hook(notify: u32, info: *const DelayLoadInfo) -> *const c_void {
    // SAFETY: the helper passes a valid DelayLoadInfo whose DLL name is a C string.
    let info = unsafe { &*info };
    let dll = unsafe { CStr::from_ptr(info.sz_dll) };
    if !dll.to_bytes().eq_ignore_ascii_case(b"vulkan-1.dll") {
        return std::ptr::null(); // anything else fails as it always did
    }
    match notify {
        // Stand this program in for the DLL; looking up the function in it then fails, below.
        // SAFETY: a null name asks for this program's own module, which is always loaded.
        DLI_FAIL_LOAD_LIB => unsafe { GetModuleHandleW(std::ptr::null()) },
        // The other functions ggml takes straight from the DLL run only on a device that
        // Vulkan found, so they are never reached here.
        // SAFETY: when importing by name, the name is a C string.
        DLI_FAIL_GET_PROC
            if info.import_by_name != 0
                && unsafe { CStr::from_ptr(info.proc_name) }.to_bytes()
                    == b"vkGetInstanceProcAddr" =>
        {
            get_instance_proc_addr as *const c_void
        }
        _ => std::ptr::null(),
    }
}

/// `vkGetInstanceProcAddr` on a computer without Vulkan: only the version query, which fails.
unsafe extern "system" fn get_instance_proc_addr(
    _instance: *const c_void,
    name: *const c_char,
) -> *const c_void {
    // SAFETY: Vulkan passes function names as C strings.
    if !name.is_null()
        && unsafe { CStr::from_ptr(name) }.to_bytes() == b"vkEnumerateInstanceVersion"
    {
        enumerate_instance_version as *const c_void
    } else {
        std::ptr::null()
    }
}

unsafe extern "system" fn enumerate_instance_version(_version: *mut u32) -> i32 {
    VK_ERROR_INITIALIZATION_FAILED
}
