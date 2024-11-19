#[allow(unused_imports)]
#[cfg(test)]
mod tests {
    // Some of these imports are consumed by the injected tests
    use assert_cmd::prelude::*;
    use predicates::prelude::*;

    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    use super::*;

    test_binary::build_test_binary_once!(mock_backend, "../backend_interface/test-binaries");

    // Utilities to keep the test matrix labels more intuitive.
    #[derive(Debug, Clone, Copy)]
    struct ForceBrillig(pub bool);
    #[derive(Debug, Clone, Copy)]
    struct Inliner(pub i64);

    // include tests generated by `build.rs`
    include!(concat!(env!("OUT_DIR"), "/execute.rs"));
}
