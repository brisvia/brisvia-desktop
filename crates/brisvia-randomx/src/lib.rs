//! Rust binding to RandomX v1.2.2 for the Brisvia miner.
//! `Cache` (seed) is shared across threads. `Dataset` (fast, ~2.1GB) speeds up mining ~10x and yields THE SAME hash
//! as the node's light mode. Each thread creates its own `Vm`. FFI (unsafe) is isolated; the public API is safe.

use std::os::raw::{c_int, c_ulong, c_void};
use std::ptr;
use std::sync::Arc;

// Pool (stratum) mining support. `blockhdr` rebuilds the exact 80-byte header the pool verifies;
// `stratum` is the async client that feeds jobs into the SAME RandomX engine as solo mining.
pub mod blockhdr;
pub mod stratum;
pub mod worksource;
pub mod pool_miner;
pub mod pool_worker;

#[repr(C)] pub struct RandomxCache { _p: [u8; 0] }
#[repr(C)] pub struct RandomxDataset { _p: [u8; 0] }
#[repr(C)] pub struct RandomxVm { _p: [u8; 0] }

type RandomxFlags = c_int;
const FLAG_FULL_MEM: RandomxFlags = 4; // uses dataset (fast)

extern "C" {
    fn randomx_get_flags() -> RandomxFlags;
    fn randomx_alloc_cache(flags: RandomxFlags) -> *mut RandomxCache;
    fn randomx_init_cache(cache: *mut RandomxCache, key: *const c_void, key_size: usize);
    fn randomx_release_cache(cache: *mut RandomxCache);
    fn randomx_alloc_dataset(flags: RandomxFlags) -> *mut RandomxDataset;
    fn randomx_dataset_item_count() -> c_ulong;
    fn randomx_init_dataset(dataset: *mut RandomxDataset, cache: *mut RandomxCache, start_item: c_ulong, item_count: c_ulong);
    fn randomx_release_dataset(dataset: *mut RandomxDataset);
    fn randomx_create_vm(flags: RandomxFlags, cache: *mut RandomxCache, dataset: *mut RandomxDataset) -> *mut RandomxVm;
    fn randomx_destroy_vm(machine: *mut RandomxVm);
    fn randomx_calculate_hash(machine: *mut RandomxVm, input: *const c_void, input_size: usize, output: *mut c_void);
}

/// RandomX cache for a seed (read-only during hashing -> shareable across threads).
pub struct Cache { ptr: *mut RandomxCache, flags: RandomxFlags }
unsafe impl Send for Cache {}
unsafe impl Sync for Cache {}

impl Cache {
    pub fn new(seed: &[u8]) -> Arc<Cache> {
        unsafe {
            let flags = randomx_get_flags();
            let ptr = randomx_alloc_cache(flags);
            assert!(!ptr.is_null(), "randomx_alloc_cache null");
            randomx_init_cache(ptr, seed.as_ptr() as *const c_void, seed.len());
            Arc::new(Cache { ptr, flags })
        }
    }
}
impl Drop for Cache {
    fn drop(&mut self) { unsafe { if !self.ptr.is_null() { randomx_release_cache(self.ptr); } } }
}

/// RandomX dataset (fast). Initialized from a Cache, in parallel. Read-only while mining -> shareable.
pub struct Dataset { ptr: *mut RandomxDataset }
unsafe impl Send for Dataset {}
unsafe impl Sync for Dataset {}

impl Dataset {
    /// Allocates and initializes the dataset from `cache` using `threads` threads (non-overlapping ranges).
    /// Panics if the ~2.1 GB allocation fails; prefer `try_new` when a light-mode fallback is possible.
    pub fn new(cache: &Cache, threads: usize) -> Arc<Dataset> {
        Self::try_new(cache, threads).expect("randomx_alloc_dataset null (insufficient RAM?)")
    }

    /// Like `new`, but returns `None` if the ~2.1 GB dataset cannot be allocated (not enough free RAM)
    /// instead of panicking, so the miner can fall back to light mode on a small machine. The build (fast
    /// init across `threads`) is otherwise identical, and the resulting hashes match light/node exactly.
    pub fn try_new(cache: &Cache, threads: usize) -> Option<Arc<Dataset>> {
        unsafe {
            let ptr = randomx_alloc_dataset(0);
            if ptr.is_null() {
                return None; // not enough RAM for the 2.1 GB dataset -> caller uses light mode
            }
            let count = randomx_dataset_item_count();
            let threads = threads.max(1) as c_ulong;
            let per = count / threads;
            let ds = ptr as usize;
            let ca = cache.ptr as usize;
            std::thread::scope(|s| {
                for t in 0..threads {
                    let start = t * per;
                    let cnt = if t == threads - 1 { count - start } else { per };
                    s.spawn(move || unsafe {
                        randomx_init_dataset(ds as *mut RandomxDataset, ca as *mut RandomxCache, start, cnt);
                    });
                }
            });
            Some(Arc::new(Dataset { ptr }))
        }
    }
}
impl Drop for Dataset {
    fn drop(&mut self) { unsafe { if !self.ptr.is_null() { randomx_release_dataset(self.ptr); } } }
}

/// RandomX machine (one per thread). Keeps its Cache and (if fast) its Dataset alive.
pub struct Vm {
    ptr: *mut RandomxVm,
    _cache: Arc<Cache>,
    _dataset: Option<Arc<Dataset>>,
}

impl Vm {
    /// Light VM (cache, ~256MB, slow). Same hash as fast; for verification / startup.
    pub fn new_light(cache: Arc<Cache>) -> Self {
        unsafe {
            let ptr = randomx_create_vm(cache.flags, cache.ptr, ptr::null_mut());
            assert!(!ptr.is_null(), "randomx_create_vm(light) null");
            Vm { ptr, _cache: cache, _dataset: None }
        }
    }
    /// Fast VM (dataset, ~2.1GB, fast). For mining.
    pub fn new_fast(cache: Arc<Cache>, dataset: Arc<Dataset>) -> Self {
        unsafe {
            let ptr = randomx_create_vm(cache.flags | FLAG_FULL_MEM, ptr::null_mut(), dataset.ptr);
            assert!(!ptr.is_null(), "randomx_create_vm(fast) null");
            Vm { ptr, _cache: cache, _dataset: Some(dataset) }
        }
    }
    #[inline]
    pub fn hash(&self, input: &[u8]) -> [u8; 32] {
        let mut out = [0u8; 32];
        unsafe {
            randomx_calculate_hash(self.ptr, input.as_ptr() as *const c_void, input.len(), out.as_mut_ptr() as *mut c_void);
        }
        out
    }
}
impl Drop for Vm {
    fn drop(&mut self) { unsafe { if !self.ptr.is_null() { randomx_destroy_vm(self.ptr); } } }
}

/// One-shot light hash (creates and destroys the VM). Only for tests/one-off verification.
pub fn calculate_hash_light(key: &[u8], input: &[u8]) -> [u8; 32] {
    Vm::new_light(Cache::new(key)).hash(input)
}

pub fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes { s.push_str(&format!("{:02x}", b)); }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    const VEC: &str = "79b0e1d9d4115b18f6b17067db151d1c9afafaca770c2157b516bf398e497580";

    #[test]
    fn vector_light_matches_the_node() {
        let key: Vec<u8> = (0u8..32).collect();
        let input: Vec<u8> = (0u8..80).collect();
        assert_eq!(to_hex(&calculate_hash_light(&key, &input)), VEC);
    }

    #[test]
    fn vector_fast_equals_light() {
        let key: Vec<u8> = (0u8..32).collect();
        let input: Vec<u8> = (0u8..80).collect();
        let cache = Cache::new(&key);
        let dataset = Dataset::new(&cache, 2);
        let fast = Vm::new_fast(cache.clone(), dataset).hash(&input);
        assert_eq!(to_hex(&fast), VEC, "fast must produce the SAME hash as light/node");
    }
}
