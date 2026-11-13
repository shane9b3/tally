#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| tally_fuzz_harness::ods(data));
