//! Source boundaries supplement the behavioral lifecycle and recovery tests.
#[test]
fn graph_dependencies_remain_generic_and_default_features_remain_empty() {
    let manifest = include_str!("../Cargo.toml");
    assert!(manifest.lines().any(|line| line.trim() == "default = []"));
    for line in include_str!("../Cargo.lock").lines() {
        let Some(package) = line
            .strip_prefix("name = \"")
            .and_then(|s| s.strip_suffix('"'))
        else {
            continue;
        };
        for product in [
            "hatter",
            "hat-specifications",
            "sem-lang",
            "nerp",
            "tpar",
            "crowsi",
        ] {
            assert!(
                package != product && !package.starts_with(&format!("{product}-")),
                "generic graph must not depend on product package {package}"
            );
        }
    }
}

#[test]
fn storage_does_not_own_product_meaning_or_a_second_commit_engine() {
    let sources = [
        ("backend", include_str!("../src/backend/mod.rs")),
        ("memory", include_str!("../src/backend/memory.rs")),
        ("redb", include_str!("../src/backend/redb.rs")),
    ];
    for (name, source) in sources {
        let code = source
            .lines()
            .map(|line| line.split("//").next().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in [
            "SemanticBinding",
            "ExecutionGrant",
            "RuntimeRole",
            "hatter::",
            "sem_lang::",
            "remove_file(",
            "remove_dir(",
            "remove_dir_all(",
            "unlink(",
            "fn commit(",
            "commit_prepared(",
        ] {
            assert!(
                !code.contains(forbidden),
                "{name}: forbidden storage responsibility {forbidden}"
            );
        }
    }
}
