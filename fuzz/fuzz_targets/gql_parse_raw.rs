//! Raw-bytes fuzz target for the GQL parser.
//!
//! Feeds arbitrary byte sequences to the parser. Tests that the parser
//! never panics, even on completely invalid input.
//!
//! Run: cargo +nightly fuzz run gql_parse_raw

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(query) = std::str::from_utf8(data) {
        // The parser must never panic, only return Ok or Err
        let _ = grafeo_adapters::query::gql::parse(query);
    }
});
