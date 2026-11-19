use super::*;

#[test]
fn a_line_of_a_list_names_a_file_on_this_machine_or_none() {
    let named = |line| path(line).map(|path| path.display().to_string());
    assert_eq!(named("file:///home/me/a.png\r").as_deref(), Some("/home/me/a.png"));
    assert_eq!(
        named("file:///tmp/My%20Photo%20%281%29.png").as_deref(),
        Some("/tmp/My Photo (1).png")
    );
    assert_eq!(named("file://localhost/etc/hosts").as_deref(), Some("/etc/hosts"));
    // Not one on another machine, nor what is no file, nor a note among the lines.
    assert_eq!(named("file://elsewhere/etc/hosts"), None);
    assert_eq!(named("https://example.com/a.png"), None);
    assert_eq!(named("# dragged from Files"), None);
}

#[test]
fn what_was_copied_is_asked_for_as_text_before_a_picture() {
    let asked = |ways: &[&str]| way(&ways.iter().map(|way| way.to_string()).collect::<Vec<_>>());
    assert_eq!(asked(&["image/png"]), Some(("image/png", Some("clipboard.png"))));
    assert_eq!(asked(&["image/bmp", "image/jpeg"]), Some(("image/jpeg", Some("clipboard.jpg"))));
    // A browser offers a picture as a page too, which is no text to type.
    assert_eq!(asked(&["text/html", "image/png"]), Some(("image/png", Some("clipboard.png"))));
    // Whatever offers both is taken at its word.
    assert_eq!(asked(&["image/png", "text/plain", "UTF8_STRING"]), Some(("UTF8_STRING", None)));
    assert_eq!(asked(&["text/html", "application/x-thing"]), None);
    assert_eq!(asked(&[]), None);
}
