//! usage_telemetry — the Usage Telemetry context. Tracks token consumption from
//! two sources (our worker stream-json tail via the kernel UsageSink seam, and
//! Claude Code transcript JSONL), computes the rolling 5h window meter + burn
//! rate + threshold band, and produces the auto-meter brake decision. It is a
//! SUPPLIER: it depends on `agent_bus_core` + `workspace` only — never on
//! `runtime` or `runners` (would invert Customer-Supplier and risk a cycle).
//! The brake STATE stays in Runtime; this crate only decides.

pub mod event;
pub mod worker_log;
pub mod cc_log;
pub mod ingest;
pub mod transcript;
pub mod window;
pub mod brake_policy;
pub mod snapshot;
pub mod api;

#[cfg(test)]
mod crate_smoke {
    #[test]
    fn crate_compiles() {
        assert_eq!(2 + 2, 4);
    }
}

#[cfg(test)]
mod contract_tests;
