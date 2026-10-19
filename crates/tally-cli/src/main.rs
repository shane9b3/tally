//! `tally-cli`: command-line front end for the Tally engine.
//!
//! Will provide `eval`, `convert`, `recalc` and `diff`, used by the
//! conformance oracle and power users (WP1). Gate G1 runs
//! `tally-cli eval --cells 'A1=2,B1=3' '=SUM(A1:B1)*2'`.

// TODO(WP1): implement the subcommands on top of `tally_api::DocumentService`.

fn main() {
    eprintln!("tally-cli: not implemented");
    std::process::exit(2);
}
