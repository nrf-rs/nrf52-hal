use std::{fs, process::Command};

pub static HALS: &[(&str, &str)] = &[
    ("nrf51-hal", "thumbv6m-none-eabi"),
    ("nrf9160-hal", "thumbv8m.main-none-eabihf"),
    ("nrf52805-hal", "thumbv7em-none-eabi"),
    ("nrf52810-hal", "thumbv7em-none-eabi"),
    ("nrf52811-hal", "thumbv7em-none-eabi"),
    ("nrf52832-hal", "thumbv7em-none-eabihf"),
    ("nrf52833-hal", "thumbv7em-none-eabihf"),
    ("nrf52840-hal", "thumbv7em-none-eabihf"),
];

pub static EXAMPLES: &[(&str, &[&str])] = &[
    ("blinky-button-demo", &[]),
    ("ccm-demo", &["52805", "52810", "52811", "52832", "52833", "52840"]),
    ("comp-demo", &[]),
    (
        "ecb-demo",
        &["51", "52805", "52810", "52811", "52832", "52833", "52840"],
    ),
    ("hello-world", &["52840", "9160"]),
    ("gpiote-demo", &[]),
    ("i2s-controller-demo", &[]),
    ("i2s-peripheral-demo", &[]),
    ("lpcomp-demo", &[]),
    (
        "ppi-demo",
        &["51", "52810", "52811", "52832", "52833", "52840"],
    ),
    ("pwm-blinky-demo", &["52840"]),
    ("pwm-demo", &["52840"]),
    ("qdec-demo", &[]),
    ("rtc-demo", &[]),
    ("rtic-demo", &["51", "52805", "52810", "52811", "52832", "52840"]),
    ("spi-demo", &[]),
    ("spis-demo", &[]),
    ("twi-ssd1306", &["52832", "52840"]),
    ("twim-demo", &[]),
    ("twis-demo", &[]),
    ("twis-dma-demo", &[]),
    ("usb", &[]),
    ("wdt-demo", &[]),
];

pub fn feature_to_target(feat: &str) -> &str {
    match feat {
        "51" => "thumbv6m-none-eabi",
        "52810" | "52811" => "thumbv7em-none-eabi",
        "9160" => "thumbv8m.main-none-eabihf",
        _ if feat.starts_with("52") => "thumbv7em-none-eabihf",
        _ => panic!("unknown Cargo feature `{}`", feat),
    }
}

pub fn install_targets() {
    let mut targets = HALS
        .iter()
        .map(|(_, target)| *target)
        .chain(
            EXAMPLES
                .iter()
                .flat_map(|(_, features)| features.iter().map(|feat| feature_to_target(feat))),
        )
        .collect::<Vec<_>>();
    targets.sort();
    targets.dedup();

    let mut cmd = Command::new("rustup");
    cmd.args(&["target", "add"]).args(&targets);
    let status = cmd
        .status()
        .map_err(|e| format!("couldn't execute {:?}: {}", cmd, e))
        .unwrap();
    assert!(
        status.success(),
        "failed to install targets with rustup: {:?}",
        cmd
    );
}

fn file_replace(path: &str, from: &str, to: &str, dry_run: bool) {
    let old_contents = fs::read_to_string(path).unwrap();
    let new_contents = old_contents.replacen(from, to, 1);
    if old_contents == new_contents {
        panic!("failed to replace `{}` -> `{}` in `{}`", from, to, path);
    }

    if !dry_run {
        fs::write(path, new_contents).unwrap();
    }
}

/// Bumps the versions of all HAL crates and the changelog to `new_version`.
///
/// Dependency declarations are updated automatically. `html_root_url` is updated automatically.
pub fn bump_versions(new_version: &str, dry_run: bool) {
    let common_toml_path = "nrf-hal-common/Cargo.toml";
    let toml = fs::read_to_string(common_toml_path).unwrap();

    let needle = "version = \"";
    let version_pos = toml.find(needle).unwrap() + needle.len();
    let version_rest = &toml[version_pos..];
    let end_pos = version_rest.find('"').unwrap();
    let old_version = &version_rest[..end_pos];

    {
        // Bump the changelog first, also check that it isn't empty.
        let changelog_path = "CHANGELOG.md";
        let changelog = fs::read_to_string(changelog_path).unwrap();
        // (ignore empty changelog when this is a dry_run, since that runs in normal CI)
        assert!(
            dry_run || !changelog.contains("(no entries)"),
            "changelog contains `(no entries)`; please fill it"
        );

        // Prepend empty "Unreleased" section, promote the current one.
        let from = String::from("## Unreleased");
        let to = format!("## Unreleased\n\n(no changes)\n\n## [{}]", new_version);
        file_replace(changelog_path, &from, &to, dry_run);

        // Append release link at the end.
        let mut changelog = fs::read_to_string(changelog_path).unwrap();
        changelog.push_str(&format!(
            "[{vers}]: https://github.com/nrf-rs/nrf-hal/releases/tag/v{vers}\n",
            vers = new_version
        ));
        if !dry_run {
            fs::write(changelog_path, changelog).unwrap();
        }
    }

    {
        println!("nrf-hal-common: {} -> {}", old_version, new_version);

        // Bump `nrf-hal-common`'s version.
        let from = format!(r#"version = "{}""#, old_version);
        let to = format!(r#"version = "{}""#, new_version);
        file_replace("nrf-hal-common/Cargo.toml", &from, &to, dry_run);

        // Bump the `html_root_url`.
        let from = format!(
            r#"#![doc(html_root_url = "https://docs.rs/nrf-hal-common/{old_version}")]"#,
            old_version = old_version
        );
        let to = format!(
            r#"#![doc(html_root_url = "https://docs.rs/nrf-hal-common/{new_version}")]"#,
            new_version = new_version
        );
        let librs_path = "nrf-hal-common/src/lib.rs";
        file_replace(librs_path, &from, &to, dry_run);
    }

    for (hal, _) in HALS {
        println!("{}: {} -> {}", hal, old_version, new_version);
        let toml_path = format!("{}/Cargo.toml", hal);

        // Bump the HAL's version.
        let from = format!(r#"version = "{}""#, old_version);
        let to = format!(r#"version = "{}""#, new_version);
        file_replace(&toml_path, &from, &to, dry_run);

        // Bump the HAL's dependency on `nrf-hal-common`.
        let from = format!(r#"version = "={}""#, old_version);
        let to = format!(r#"version = "={}""#, new_version);
        file_replace(&toml_path, &from, &to, dry_run);

        // Bump the HAL's `html_root_url`.
        let from = format!(
            r#"#![doc(html_root_url = "https://docs.rs/{crate}/{old_version}")]"#,
            crate = hal,
            old_version = old_version
        );
        let to = format!(
            r#"#![doc(html_root_url = "https://docs.rs/{crate}/{new_version}")]"#,
            crate = hal,
            new_version = new_version
        );
        let librs_path = format!("{}/src/lib.rs", hal);
        file_replace(&librs_path, &from, &to, dry_run);
    }
}
