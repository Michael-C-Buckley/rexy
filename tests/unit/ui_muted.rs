use super::*;

#[test]
fn muted_tabs_are_remembered_until_they_close() {
    let directory = std::env::temp_dir().join(format!("rex-lite-muted-{}", std::process::id()));
    let file = directory.join("state/muted");
    let mut muted = Muted::open(file.clone());
    assert!(!muted.has("window:a"));
    for tab in ["window:a", "window:b", "window:c"] {
        muted.toggle(tab).unwrap();
    }
    muted.toggle("window:b").unwrap();
    let mut again = Muted::open(file.clone());
    assert!(again.has("window:a") && !again.has("window:b") && again.has("window:c"));
    again.keep(|tab| tab == "window:c").unwrap();
    let last = Muted::open(file);
    assert!(!last.has("window:a") && last.has("window:c"));
    std::fs::remove_dir_all(directory).unwrap();
}
