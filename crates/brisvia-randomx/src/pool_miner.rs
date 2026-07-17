//! Mine a pool job: search a nonce whose RandomX hash of the pool-provided header is <= target.
//! Reuses the SAME RandomX engine as solo mining (Cache/Vm) — only the work source differs. In
//! "approach D" the header comes ready from the pool; here we just vary the 4 nonce bytes (offset 76).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::worksource::{MiningJob, Solution};
use crate::{Cache, Vm};

/// Returned when a pool asks to rebuild the RandomX seed cache more often than the policy allows.
/// In Brisvia the seed rotates per epoch (not per job), so a legitimate pool never trips this; it only
/// fires under an abnormal burst of distinct seeds — the shape of a hostile pool trying to pin CPU/RAM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedChurn;

/// Guards RandomX seed handling across successive pool jobs. It reuses the current Cache while the seed is
/// unchanged (the common case — rebuilding the ~256 MB light cache every job would waste CPU) and rate-limits
/// rebuilds so a hostile pool cannot force a rebuild loop. Lives in the miner loop; state persists across jobs
/// and reconnections.
pub struct SeedGuard {
    current: Option<([u8; 32], Arc<Cache>)>,
    rebuilds: VecDeque<Instant>,
    max_rebuilds: usize,
    window: Duration,
}

impl Default for SeedGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl SeedGuard {
    /// Default policy: at most 8 seed-cache rebuilds per 60 s. Never reached in normal operation (the seed
    /// changes only every epoch), but caps a burst of distinct seeds from a defective or hostile pool.
    pub fn new() -> Self {
        Self::with_policy(8, Duration::from_secs(60))
    }

    pub fn with_policy(max_rebuilds: usize, window: Duration) -> Self {
        SeedGuard { current: None, rebuilds: VecDeque::new(), max_rebuilds, window }
    }

    /// Return a Cache for `seed`, reusing the current one if the seed is unchanged. A new seed triggers a
    /// rate-limited rebuild; exceeding `max_rebuilds` within `window` returns `SeedChurn` and the caller must
    /// skip mining that job rather than rebuild.
    pub fn cache_for(&mut self, seed: &[u8; 32]) -> Result<Arc<Cache>, SeedChurn> {
        if let Some((cur_seed, cache)) = &self.current {
            if cur_seed == seed {
                return Ok(cache.clone());
            }
        }
        let now = Instant::now();
        while let Some(&front) = self.rebuilds.front() {
            if now.duration_since(front) > self.window {
                self.rebuilds.pop_front();
            } else {
                break;
            }
        }
        if self.rebuilds.len() >= self.max_rebuilds {
            return Err(SeedChurn);
        }
        self.rebuilds.push_back(now);
        let cache = Cache::new(seed);
        self.current = Some((*seed, cache.clone()));
        Ok(cache)
    }
}

/// Search for a winning nonce across `threads` threads, up to `max_nonces`. `stop` aborts early (new job /
/// mode change). Uses light RandomX (no 2 GB dataset) — the same hash the pool's verifier computes.
/// Returns a Solution (job_id + nonce) or None if the range is exhausted or aborted. Builds the seed cache
/// itself; the pool worker calls `mine_with_cache` through a `SeedGuard` to reuse it across jobs.
pub fn mine_job(job: &MiningJob, threads: usize, nonce_start: u64, max_nonces: u64, stop: &AtomicBool) -> Option<Solution> {
    if job.header80.len() != 80 {
        return None;
    }
    let cache = Cache::new(&job.seed_key);
    mine_with_cache(job, &cache, threads, nonce_start, max_nonces, stop)
}

/// Same search as `mine_job` but with a pre-built Cache supplied by the caller (so it can be reused across
/// jobs that share a seed). The Cache MUST correspond to `job.seed_key`.
/// Does this RandomX hash meet the big-endian target? Single source of truth for the share check, and
/// it mirrors the pool's verifier EXACTLY: the raw hash comes out in RandomX's native (little-endian)
/// order, so we reverse it to big-endian and compare `<=` against the big-endian target. The pool
/// (rx_verifier returns the pow hash big-endian; the server compares int.from_bytes(h,"big") <= target)
/// applies the identical rule, so a share the miner accepts is a share the pool accepts.
#[inline]
pub fn hash_meets_target(raw_hash: &[u8; 32], target_be: &[u8; 32]) -> bool {
    let mut be = *raw_hash;
    be.reverse(); // native (LE) -> big-endian, to compare against the big-endian target
    &be <= target_be
}

/// `mine_with_cache_counted` with no hash accounting — used by tests and callers that do not report speed.
pub fn mine_with_cache(job: &MiningJob, cache: &Arc<Cache>, threads: usize, nonce_start: u64, max_nonces: u64, stop: &AtomicBool) -> Option<Solution> {
    mine_with_cache_counted(job, cache, threads, nonce_start, max_nonces, stop, &AtomicU64::new(0))
}

/// Same search, but every thread adds the hashes it computed to `hashes` (batched by 512 to avoid contention)
/// so a watcher can report a real per-second speed in pool mode. Showing 0 H/s while the CPU is working reads
/// as a failure, so the pool UI needs a real number.
pub fn mine_with_cache_counted(job: &MiningJob, cache: &Arc<Cache>, threads: usize, nonce_start: u64, max_nonces: u64, stop: &AtomicBool, hashes: &AtomicU64) -> Option<Solution> {
    if job.header80.len() != 80 {
        return None;
    }
    let winner = AtomicU32::new(0);
    let found = AtomicBool::new(false);
    let threads = threads.max(1);
    std::thread::scope(|s| {
        for t in 0..threads {
            let (cache, found, winner, hashes) = (cache.clone(), &found, &winner, hashes);
            s.spawn(move || {
                let vm = Vm::new_light(cache);
                let mut local = job.header80.clone();
                let mut nonce = nonce_start + t as u64; // sweep from where THIS job's previous round left off
                let mut count = 0u64;
                while nonce <= max_nonces && nonce <= u32::MAX as u64 {
                    if count % 256 == 0 && (found.load(Ordering::Relaxed) || stop.load(Ordering::Relaxed)) {
                        break;
                    }
                    local[76..80].copy_from_slice(&(nonce as u32).to_le_bytes());
                    let h = vm.hash(&local);
                    count += 1;
                    if count & 0x1ff == 0 { hashes.fetch_add(512, Ordering::Relaxed); } // batch every 512 hashes
                    if hash_meets_target(&h, &job.target_be) {
                        winner.store(nonce as u32, Ordering::SeqCst);
                        found.store(true, Ordering::SeqCst);
                        break;
                    }
                    nonce += threads as u64;
                }
                hashes.fetch_add(count & 0x1ff, Ordering::Relaxed); // the leftover below a full batch
            });
        }
    });
    if found.load(Ordering::SeqCst) {
        Some(Solution {
            job_id: job.job_id.clone(),
            nonce: winner.load(Ordering::SeqCst),
            ntime: 0,            // approach D: the pool holds ntime; submit carries only {job_id, nonce}
            extranonce2: String::new(),
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // THE CROSS-CHECK. The test after this one proves the Rust rule is self-consistent, but despite its name
    // ("match_pool_rule") it never runs the pool: it assumes what the pool does. If the pool ever read the bytes
    // the other way round, both sides would stay green and the miner would submit work the pool discards --
    // CPU burned, nobody paid.
    //
    // This reads the SHARED vector file that the pool's own test also reads (test_share_rule_vectors.py). One
    // written truth, two implementations, no assumptions.
    #[test]
    fn the_shared_vectors_decide_exactly_as_the_pool() {
        let raw = include_str!("../share_rule_vectors.json");
        let doc: serde_json::Value = serde_json::from_str(raw).expect("vector file must be valid JSON");
        let vectors = doc["vectors"].as_array().expect("vectors must be a list");
        assert!(vectors.len() >= 12, "the vector file lost cases: {}", vectors.len());

        for v in vectors {
            let name = v["name"].as_str().unwrap();
            let hash: [u8; 32] = hex::decode(v["hash_raw_le"].as_str().unwrap())
                .unwrap()
                .try_into()
                .expect("hash must be 32 bytes");
            let target: [u8; 32] = hex::decode(v["target_be"].as_str().unwrap())
                .unwrap()
                .try_into()
                .expect("target must be 32 bytes");
            let expected = v["accept"].as_bool().unwrap();
            assert_eq!(
                hash_meets_target(&hash, &target),
                expected,
                "the miner disagrees with the shared rule -- the pool would throw this share away: {name}"
            );
        }
    }

    // Pins down the share-check endianness in code, next to the shared-vector cross-check above.
    // Boundary vectors, no RandomX needed: reverse(raw_hash) <= target_be, big-endian.
    #[test]
    fn endianness_boundary_vectors_match_pool_rule() {
        let mut target_be = [0u8; 32]; target_be[31] = 0x10;      // target = 16 (big-endian)
        let mut eq = [0u8; 32]; eq[0] = 0x10;                     // reverse -> 16 == target -> accept
        assert!(hash_meets_target(&eq, &target_be));
        let mut less = [0u8; 32]; less[0] = 0x0f;                 // reverse -> 15 < 16 -> accept
        assert!(hash_meets_target(&less, &target_be));
        let mut more = [0u8; 32]; more[0] = 0x11;                 // reverse -> 17 > 16 -> reject
        assert!(!hash_meets_target(&more, &target_be));
        assert!(hash_meets_target(&[0u8; 32], &target_be));       // zero hash always qualifies
        assert!(hash_meets_target(&[0xff; 32], &[0xff; 32]));     // max hash vs max target -> equal -> accept
        let mut hard = [0u8; 32]; hard[0] = 0x01;                 // hardest target (0x01 at the MSB)
        assert!(!hash_meets_target(&[0xff; 32], &hard));          // max hash never meets the hardest target
    }

    #[test]
    fn finds_a_nonce_for_an_easy_target() {
        // target all-0xFF => any hash qualifies => the first nonce tried wins. Exercises the real RandomX path.
        let job = MiningJob {
            header80: vec![0u8; 80],
            seed_key: [0x54u8; 32],
            target_be: [0xffu8; 32],
            job_id: "t".into(),
            height: 1,
        };
        let stop = AtomicBool::new(false);
        let sol = mine_job(&job, 2, 0, 10_000, &stop).expect("should find a nonce for an all-ones target");
        assert_eq!(sol.job_id, "t");
        assert!(sol.extranonce2.is_empty()); // approach D: nothing but job_id + nonce goes back
    }

    #[test]
    fn stop_flag_aborts() {
        // target all-zero => essentially unreachable => without stop it would scan forever; stop ends it.
        let job = MiningJob {
            header80: vec![0u8; 80],
            seed_key: [0x54u8; 32],
            target_be: [0x00u8; 32],
            job_id: "t".into(),
            height: 1,
        };
        let stop = AtomicBool::new(true); // already stopped → returns quickly with None
        assert!(mine_job(&job, 1, 0, 1_000_000, &stop).is_none());
    }

    // The SeedGuard reuses the cache while the seed is unchanged and only rebuilds on a real change.
    #[test]
    fn seed_guard_reuses_cache_for_same_seed() {
        let mut g = SeedGuard::with_policy(2, Duration::from_secs(60));
        let seed = [0x11u8; 32];
        // First call builds (consumes one rebuild slot); second call with the SAME seed must NOT rebuild.
        g.cache_for(&seed).expect("first build ok");
        g.cache_for(&seed).expect("same seed reuses, no rebuild");
        // With max_rebuilds=2 and only one real build so far, a DIFFERENT seed still fits.
        g.cache_for(&[0x22u8; 32]).expect("second distinct seed still within policy");
        // A third distinct seed exceeds the 2-rebuild ceiling → churn is refused.
        assert!(matches!(g.cache_for(&[0x33u8; 32]), Err(SeedChurn)), "third distinct seed exceeds the ceiling");
    }

    // A burst of distinct seeds beyond the ceiling is refused (the hostile-pool DoS guard).
    #[test]
    fn seed_guard_rate_limits_a_seed_burst() {
        let mut g = SeedGuard::with_policy(1, Duration::from_secs(60));
        g.cache_for(&[1u8; 32]).expect("first build ok");
        assert!(matches!(g.cache_for(&[2u8; 32]), Err(SeedChurn)), "second distinct seed exceeds ceiling of 1");
        // …but the already-built seed is still served from cache without counting as a rebuild.
        g.cache_for(&[1u8; 32]).expect("cached seed still served");
    }
}
