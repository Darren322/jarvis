use std::sync::Once;

use rusqlite::ffi;

/// Number of dimensions in every stored memory vector (BGE-small-en-v1.5).
pub(crate) const EMBEDDING_DIMENSIONS: usize = 384;

type ExtensionInit = unsafe extern "C" fn(
    *mut ffi::sqlite3,
    *mut *mut std::os::raw::c_char,
    *const ffi::sqlite3_api_routines,
) -> std::os::raw::c_int;

/// Registers the statically linked sqlite-vec extension for every SQLite
/// connection this process opens afterwards. Safe to call repeatedly.
pub(super) fn register() {
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| {
        // SAFETY: `sqlite3_vec_init` is sqlite-vec's documented extension entry
        // point. sqlite-vec exposes it without its C signature, so it is cast to
        // the signature `sqlite3_auto_extension` expects; SQLite then calls it
        // once per new connection with valid arguments.
        unsafe {
            let init = std::mem::transmute::<*const (), ExtensionInit>(
                sqlite_vec::sqlite3_vec_init as *const (),
            );
            ffi::sqlite3_auto_extension(Some(init));
        }
    });
}

/// Encodes a vector in the little-endian f32 layout `vec0` columns accept.
pub(super) fn vector_blob(vector: &[f32]) -> Vec<u8> {
    vector
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}
