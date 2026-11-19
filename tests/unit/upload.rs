use super::*;

#[test]
fn a_path_is_one_word_to_a_shell() {
    assert_eq!(quoted("/home/me/notes-2.txt"), "/home/me/notes-2.txt");
    assert_eq!(quoted("/tmp/My Photo (1).png"), "'/tmp/My Photo (1).png'");
    assert_eq!(quoted("it's $HOME"), r"'it'\''s $HOME'");
    assert_eq!(quoted(""), "''");
}

#[test]
fn only_a_file_can_be_sent_and_under_its_own_name() {
    let folder = std::env::temp_dir();
    let error = File::Path(folder).open().err().unwrap().to_string();
    assert!(error.ends_with("is not a file, and only files are sent"), "{error}");
    assert!(File::Path("/nowhere/at all".into()).open().is_err());
    let (name, size, mut bytes) = File::Pasted("clipboard.png", Arc::from(*b"png")).open().unwrap();
    let mut read = Vec::new();
    bytes.read_to_end(&mut read).unwrap();
    assert_eq!((name.as_str(), size, read.as_slice()), ("clipboard.png", 3, &b"png"[..]));
}
