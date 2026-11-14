use super::*;

#[test]
fn a_program_gets_the_icon_of_the_first_of_its_names_to_have_one() {
    let icon = |names: &[&str]| {
        let program = Program::named(names.iter().copied());
        program.unwrap_or_default().name()
    };
    assert_eq!(icon(&["codex"]), "codex");
    assert_eq!(icon(&["/usr/local/bin/nvim"]), "neovim");
    assert_eq!(icon(&["Claude Code"]), "claude-code");
    assert_eq!(icon(&["python3.12"]), "python");
    assert_eq!(icon(&["Docker.EXE"]), "docker");
    assert_eq!(icon(&["vite-2"]), "vite-2");
    // Claude Code's executable is a file named for its version.
    assert_eq!(icon(&["claude", "2.1.325"]), "claude-code");
    assert_eq!(icon(&["2.1.325", "claude"]), "claude-code");
    assert_eq!(icon(&["build.py", "python3", "python3.12"]), "python");
    let script = "/opt/lib/node_modules/pi-coding-agent/dist/cli.js";
    assert_eq!(icon(&[script, "node", "node"]), "pi");
    assert_eq!(icon(&["sleep"]), "shell-sleep");
    // A shell's icon stands for shells and for whatever has none.
    for names in [
        &["-zsh", "zsh"][..],
        &["frobnicate"],
        &["shell-top"],
        &["net"],
        &[],
    ] {
        assert_eq!(icon(names), "shell");
    }
    assert_eq!(icon(&["dotnet"]), "net");
}

/// The icons are fetched, not kept here, so there may be none, or all of them.
#[test]
fn every_icon_at_hand_is_listed_and_can_be_drawn() {
    for (name, drawing) in DRAWINGS {
        assert!(ICONS.contains(name), "{name}");
        let tree = resvg::usvg::Tree::from_str(drawing, &Default::default());
        assert_eq!(tree.expect(name).size().width(), 31.25, "{name}");
    }
    for line in ALIASES.lines() {
        let (icon, names) = line.split_once(": ").expect(line);
        assert!(Program::icon(icon).is_some() && !names.is_empty(), "{line}");
    }
}
