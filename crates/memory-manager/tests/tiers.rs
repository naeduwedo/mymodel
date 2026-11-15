//! Memory tier and pressure tests.

use gguf_engine_memory_manager::{
    PrefetchRequest, Pressure, Tier, TierBudget, grant_prefetch, host_memory,
};

const GIB: u64 = 1024 * 1024 * 1024;

#[test]
fn nvme_is_not_addressable_memory() {
    assert!(Tier::GpuVram.is_addressable_memory());
    assert!(Tier::HostRam.is_addressable_memory());
    assert!(
        !Tier::Nvme.is_addressable_memory(),
        "ADR-006: NVMe is a cold tier, never equivalent VRAM"
    );
}

#[test]
fn tier_budget_subtracts_the_reserve_and_never_underflows() {
    let budget = TierBudget::compute(Tier::HostRam, 64 * GIB, 40 * GIB, 8 * GIB);
    assert_eq!(budget.usable, 32 * GIB);

    // Available below the reserve must clamp to zero, not wrap around.
    let starved = TierBudget::compute(Tier::HostRam, 64 * GIB, 2 * GIB, 8 * GIB);
    assert_eq!(starved.usable, 0);
}

#[test]
fn pressure_crosses_into_critical_at_the_reserve_floor() {
    let reserve = 8 * GIB;
    assert_eq!(Pressure::classify(40 * GIB, reserve), Pressure::Nominal);
    assert_eq!(Pressure::classify(10 * GIB, reserve), Pressure::Elevated);
    assert_eq!(Pressure::classify(8 * GIB, reserve), Pressure::Critical);
    assert_eq!(Pressure::classify(GIB, reserve), Pressure::Critical);
}

#[test]
fn only_nominal_pressure_allows_a_new_load() {
    assert!(Pressure::Nominal.allows_new_load());
    assert!(!Pressure::Elevated.allows_new_load());
    assert!(!Pressure::Critical.allows_new_load());
}

#[test]
fn prefetch_is_capped_and_truncated_rather_than_refused() {
    let cap = 2 * GIB;
    // A small request is granted whole.
    assert_eq!(
        grant_prefetch(
            PrefetchRequest {
                bytes: 256 * 1024 * 1024
            },
            cap,
            Pressure::Nominal,
            0
        )
        .unwrap(),
        256 * 1024 * 1024
    );

    // An oversized request is truncated to the remaining headroom.
    assert_eq!(
        grant_prefetch(
            PrefetchRequest { bytes: 8 * GIB },
            cap,
            Pressure::Nominal,
            GIB / 2
        )
        .unwrap(),
        cap - GIB / 2
    );
}

#[test]
fn prefetch_stops_completely_under_pressure() {
    let cap = 2 * GIB;
    let err =
        grant_prefetch(PrefetchRequest { bytes: GIB }, cap, Pressure::Critical, 0).unwrap_err();
    assert!(err.0.contains("critical"), "{}", err.0);

    let err =
        grant_prefetch(PrefetchRequest { bytes: GIB }, cap, Pressure::Elevated, 0).unwrap_err();
    assert!(err.0.contains("elevated"), "{}", err.0);
}

#[test]
fn prefetch_refuses_when_the_cap_is_already_saturated() {
    let cap = 2 * GIB;
    let err =
        grant_prefetch(PrefetchRequest { bytes: GIB }, cap, Pressure::Nominal, cap).unwrap_err();
    assert!(err.0.contains("in flight"), "{}", err.0);
}

#[test]
fn live_host_memory_is_plausible_on_any_machine() {
    let (total, available) = host_memory();
    assert!(total > 512 * 1024 * 1024, "total: {total}");
    assert!(available <= total);
}
