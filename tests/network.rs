//! Explicit, opt-in network smoke tests; normal CI remains deterministic.
#[cfg(windows)]
#[test]
#[ignore = "Contacts ipwho.is with the public Google DNS address"]
fn live_geoip_over_winhttp() {
    use eft_region_watcher::geo::{GeoProvider, IpWhoIs};
    let result = IpWhoIs.lookup("8.8.8.8".parse().unwrap()).unwrap();
    assert!(!result.country.is_empty());
    println!("GeoIP HTTPS succeeded: {}", result.label());
}
#[cfg(windows)]
#[test]
#[ignore = "Reads a public Rust release; never downloads or installs an update"]
fn live_github_release_over_winhttp() {
    let release = eft_region_watcher::update::check("rust-lang/rust").unwrap();
    assert!(!release.draft);
    eft_region_watcher::update::version(&release.tag_name).unwrap();
    println!("GitHub HTTPS succeeded: {}", release.tag_name);
}
