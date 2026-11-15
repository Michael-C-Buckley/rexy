//! Bundles whichever program icons are in `assets/icons`. They are not kept in
//! this repository, and the app builds without them: see the notice there.
use std::{env, fs, path::Path};

fn main() {
    println!("cargo::rerun-if-changed=assets/icons");
    let folder = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("assets/icons");
    let files = fs::read_dir(&folder).into_iter().flatten().flatten();
    let files = files.map(|file| file.path());
    let mut icons: Vec<_> = files.filter(|file| file.extension() == Some("svg".as_ref())).collect();
    icons.sort();
    let row = |icon: &std::path::PathBuf| {
        let name = icon.file_stem().unwrap().to_str().unwrap();
        format!("({name:?}, include_str!({icon:?})),\n")
    };
    let rows: String = icons.iter().map(row).collect();
    let out = Path::new(&env::var("OUT_DIR").unwrap()).join("icons.rs");
    fs::write(out, format!("[{rows}]")).unwrap();
}
