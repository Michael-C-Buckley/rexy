use super::*;

#[test]
fn an_address_names_its_host_by_its_machine() {
    assert_eq!(named("ssh://studio").as_deref(), Some("studio"));
    assert_eq!(named("ssh://me@studio.example.net:2222").as_deref(), Some("studio"));
    assert_eq!(named("wss://attic.example.net/rex?token=a.b").as_deref(), Some("attic"));
    assert_eq!(named("studio:7297").as_deref(), Some("studio"));
    // One in numbers is not cut at its dots, nor at its colons.
    assert_eq!(named("192.168.1.20:7297").as_deref(), Some("192.168.1.20"));
    assert_eq!(named("ws://[fd7a::1]:7297").as_deref(), Some("fd7a::1"));
    // A socket's path names no machine.
    assert_eq!(named("unix:///tmp/rex.sock"), None);
    assert_eq!(named("/tmp/rex.sock"), None);
}

#[test]
fn a_host_is_typed_as_its_name_and_its_addresses() {
    let host = |id: &str, label: &str, endpoints: &[&str]| Host {
        id: id.into(),
        label: label.into(),
        endpoints: endpoints.iter().map(|&endpoint| endpoint.to_owned()).collect(),
        ..Host::default()
    };
    assert_eq!(typed("  ", None), None);
    assert_eq!(typed("ssh://studio", None), Some(host("", "studio", &["ssh://studio"])));
    let both = typed(" loft  ssh://studio  studio.example.net ", None);
    assert_eq!(both, Some(host("", "loft", &["ssh://studio", "studio.example.net"])));
    // A socket has to be named.
    assert_eq!(typed("unix:///tmp/rex.sock", None), None);
    // A change keeps the host it changes, which opens as it would be typed.
    let was = Host { source: "init.lua:4".into(), ..host("host:7", "loft", &["ssh://studio"]) };
    assert_eq!(line(&was), "loft ssh://studio");
    assert_eq!(
        typed("attic ssh://attic", Some(&was)),
        Some(host("host:7", "attic", &["ssh://attic"]))
    );
}
