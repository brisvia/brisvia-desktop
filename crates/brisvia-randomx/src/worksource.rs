//! WorkSource — the single abstraction that lets the miner get work from either the local node (solo)
//! or a pool (stratum), while the RandomX engine stays the same. Per PLAN-integracion-minero-pool.md
//! and ChatGPT's review: "adapt around, don't refactor inside" — the audited solo hashing loop is
//! untouched; only the SOURCE of the header and the DESTINATION of the result change.
//!
//! This file defines the common types and the trait. The two implementations live next to it:
//! - SoloWorkSource: wraps today's getblocktemplate → build candidate → submitblock flow.
//! - PoolWorkSource: the stratum client (`stratum.rs`) reconstructs the header from the pool's job.
//!   (Blocked until the pool's `job` carries enough to rebuild the header — see the plan's design note.)

/// One unit of work handed to the RandomX engine: the exact 80 header bytes to hash (the engine
/// varies bytes 76..80 = the nonce), the RandomX seed key for this height, and the target to beat.
/// `job_id` ties a found solution back to the job it came from (mandatory for pool submit; unused solo).
#[derive(Debug, Clone, PartialEq)]
pub struct MiningJob {
    pub header80: Vec<u8>,     // serialized 80-byte header; nonce occupies bytes 76..80
    pub seed_key: [u8; 32],    // RandomX cache key for this height
    pub target_be: [u8; 32],   // big-endian target the hash must be <= (share target for pool, block target for solo)
    pub job_id: String,        // pool job id (empty for solo)
    pub height: u64,
}

/// A found nonce for a given job. What the source does with it differs (solo: submitblock; pool: submit share).
#[derive(Debug, Clone, PartialEq)]
pub struct Solution {
    pub job_id: String,
    pub nonce: u32,
    pub ntime: u32,            // the ntime the header was hashed with (pool may vary it; solo uses curtime)
    pub extranonce2: String,   // miner's extranonce2 (empty for solo)
}

/// The source of mining work. One RandomX engine consumes jobs from whichever source is active.
pub trait WorkSource {
    /// Get the current job to mine, if any. `Ok(None)` = no work right now (e.g. pool between jobs);
    /// the caller waits and asks again. Errors are transient/fatal per the implementation.
    fn next_job(&mut self) -> Result<Option<MiningJob>, String>;

    /// Report a found solution back to the source (solo → submitblock; pool → submit share).
    fn submit(&mut self, solution: Solution) -> Result<(), String>;

    /// A short label for logs/UI ("solo", "pool:official", "pool:custom").
    fn label(&self) -> &str;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_and_solution_roundtrip_fields() {
        let job = MiningJob {
            header80: vec![0u8; 80],
            seed_key: [0x54; 32],
            target_be: [0xff; 32],
            job_id: "j1".into(),
            height: 3000,
        };
        assert_eq!(job.header80.len(), 80);
        assert_eq!(job.height, 3000);
        let sol = Solution { job_id: "j1".into(), nonce: 42, ntime: 1785596400, extranonce2: "0001".into() };
        assert_eq!(sol.job_id, job.job_id);
        assert_eq!(sol.nonce, 42);
    }
}
