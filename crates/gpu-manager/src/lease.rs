//! Resource leases (spec 5.5 step 8: reserve, then verify, then release).
//!
//! A plan is only a proposal. Before a worker starts, the reserved bytes are
//! taken from the ledger as a lease. A lease that is never released (because a
//! worker crashed) must not pin a GPU forever, so every lease has a deadline and
//! the ledger reports leases it considers abandoned.

use std::collections::BTreeMap;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

/// A reservation of VRAM on one device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    /// Lease id.
    pub lease_id: String,
    /// Device the bytes are reserved on.
    pub device_id: String,
    /// Bytes reserved.
    pub bytes: u64,
    /// Who holds it (model instance id, task id, ...).
    pub holder: String,
    /// UTC epoch milliseconds when the lease expires.
    pub expires_at_unix_ms: u64,
}

/// Why a reservation was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct LeaseError(pub String);

#[derive(Debug, Default)]
struct Ledger {
    by_device: BTreeMap<String, u64>,
    leases: BTreeMap<String, Lease>,
}

/// A reservation request.
///
/// Bundled into a struct rather than passed as seven positional arguments: two
/// adjacent `u64` sizes and two `u64` times are easy to swap at a call site, and
/// swapping them silently reserves the wrong amount.
#[derive(Debug, Clone, Copy)]
pub struct ReserveRequest<'a> {
    /// Lease id.
    pub lease_id: &'a str,
    /// Device the bytes are reserved on.
    pub device_id: &'a str,
    /// Bytes to reserve.
    pub bytes: u64,
    /// Who holds it.
    pub holder: &'a str,
    /// What the device can use in total.
    pub usable_bytes: u64,
    /// Current time, UTC epoch milliseconds.
    pub now_ms: u64,
    /// Lease lifetime in milliseconds.
    pub ttl_ms: u64,
}

/// The reservation ledger shared by the planner and the workers.
#[derive(Debug, Clone, Default)]
pub struct GpuLedger {
    inner: Arc<Mutex<Ledger>>,
}

impl GpuLedger {
    /// Reserve bytes on a device, refusing when more is already reserved than
    /// the device can use.
    pub fn reserve(&self, request: ReserveRequest<'_>) -> Result<Lease, LeaseError> {
        let ReserveRequest {
            lease_id,
            device_id,
            bytes,
            holder,
            usable_bytes,
            now_ms,
            ttl_ms,
        } = request;
        let mut ledger = self.inner.lock();
        let already = ledger.by_device.get(device_id).copied().unwrap_or(0);

        if already.saturating_add(bytes) > usable_bytes {
            return Err(LeaseError(format!(
                "{device_id}: {:.1} GiB requested, {:.1} GiB already reserved, {:.1} GiB usable",
                gib(bytes),
                gib(already),
                gib(usable_bytes)
            )));
        }

        let lease = Lease {
            lease_id: lease_id.to_owned(),
            device_id: device_id.to_owned(),
            bytes,
            holder: holder.to_owned(),
            expires_at_unix_ms: now_ms.saturating_add(ttl_ms),
        };

        *ledger.by_device.entry(device_id.to_owned()).or_insert(0) += bytes;
        ledger.leases.insert(lease.lease_id.clone(), lease.clone());
        Ok(lease)
    }

    /// Release a lease. Releasing an unknown lease is not an error: a worker
    /// may have been killed twice.
    pub fn release(&self, lease_id: &str) -> Option<Lease> {
        let mut ledger = self.inner.lock();
        let lease = ledger.leases.remove(lease_id)?;
        if let Some(total) = ledger.by_device.get_mut(&lease.device_id) {
            *total = total.saturating_sub(lease.bytes);
        }
        Some(lease)
    }

    /// Bytes currently reserved on a device.
    pub fn reserved_on(&self, device_id: &str) -> u64 {
        self.inner
            .lock()
            .by_device
            .get(device_id)
            .copied()
            .unwrap_or(0)
    }

    /// Every live lease.
    pub fn leases(&self) -> Vec<Lease> {
        self.inner.lock().leases.values().cloned().collect()
    }

    /// Leases whose deadline has passed.
    pub fn expired(&self, now_ms: u64) -> Vec<Lease> {
        self.inner
            .lock()
            .leases
            .values()
            .filter(|lease| lease.expires_at_unix_ms <= now_ms)
            .cloned()
            .collect()
    }

    /// Release every expired lease, returning how many were reclaimed.
    pub fn reap_expired(&self, now_ms: u64) -> usize {
        let expired = self.expired(now_ms);
        for lease in &expired {
            self.release(&lease.lease_id);
        }
        expired.len()
    }
}

fn gib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0 * 1024.0)
}
