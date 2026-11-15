//! The programs that have an icon of their own, and the names they go by. The
//! icons are Superlogical's Rex icon set, which is fetched, not kept here: see
//! `assets/icons/NOTICE.md`.

/// A program's icon. A shell's stands for every program without one of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Program(&'static str);

/// Every icon of the set, by name.
#[rustfmt::skip]
const ICONS: [&str; 156] = [
    "amp", "android", "antigravity", "apple-container", "azure", "bat", "bazel", "boundary",
    "browser", "btop", "buck", "buildkite", "bun", "caddy", "charm", "clang", "claude-code",
    "cloudflare", "coder", "codex", "consul", "copilot", "crush", "cua", "cursor", "d2", "dart",
    "datadog", "dbcli", "delta", "depot", "devin", "docker", "droid", "duckdb", "e2b", "elixir",
    "emacs", "expo", "flutter", "foundation-models", "fx", "gemini", "git", "git-butler",
    "git-changes", "github", "gitlab", "gnu", "go", "goose", "gradle", "grok", "helix", "hermes",
    "heroku", "homebrew", "homey", "htop", "hugging-face", "hunk", "ibm-bob", "inspector", "jj",
    "julia", "junie", "jupyter", "k9s", "kakoune", "kimi", "kiro", "kubernetes", "llama", "maki",
    "mastra", "meta", "micro", "minimal", "mise", "mistral", "modal", "mole", "moon", "nano", "nanoc",
    "neovim", "net", "nextflow", "nix", "node", "nomad", "notion", "ollama", "omp", "openclaw",
    "opencode", "opscope", "orbstack", "oxide", "pack", "packer", "php", "pi", "pixi", "pnpm",
    "podman", "poolside", "porter", "postgres", "prime-agent", "proto", "proton-drive",
    "proton-pass", "python", "railway", "rclone", "redis", "render", "rex", "ruby", "rust", "sail",
    "sanity", "shell", "shell-caffeinate", "shell-container", "shell-log", "shell-sleep",
    "shell-top", "shopify", "snowflake", "sofka", "sqlite", "stripe", "sublime", "swift",
    "tailscale", "tart", "task", "terraform", "tessl", "tilt", "turborepo", "unreal-engine", "uv",
    "vagrant", "vakna", "vault", "vercel", "vim", "vite", "vite-2", "waypoint", "xcode", "yazi",
    "zig",
];

/// The drawings of those that were at hand when this was built.
static DRAWINGS: &[(&str, &str)] = &include!(concat!(env!("OUT_DIR"), "/icons.rs"));

/// The names an icon goes by besides its own.
const ALIASES: &str = "\
android: adb
antigravity: agy
apple-container: container
azure: az
bazel: bazelisk
buck: buck2
buildkite: bk buildkite-agent
bun: bunx
charm: gum glow vhs mods
clang: clang++ clangd clang-cpp clang-cl
claude-code: claude
cloudflare: wrangler cloudflared workerd
copilot: gh-copilot
cua: lume
cursor: agent cursor-agent
dart: pub
datadog: pup
dbcli: pgcli mycli litecli iredis mssql-cli athenacli vcli
docker: docker-compose lazydocker
elixir: iex mix elixirc
emacs: emacsclient
flutter: fvm
git-butler: but gitbutler
git-changes: lazygit gitui tig tuicr
github: gh
gitlab: glab glab-tui duo
gnu: gcc g++ cpp make gmake configure libtool
go: gopls
gradle: gradlew
grok: grok-build
helix: hx
homebrew: brew
hugging-face: hf huggingface-cli
ibm-bob: bob
jj: jjui lazyjj
jupyter: jupyter-lab jupyter-notebook jupyter-console jupyter-qtconsole
kakoune: kak
kiro: kiro-cli
kubernetes: kubectl kubectx kubens helm minikube
llama: llama-cli llama-server llamafile
mastra: mastracode
mistral: vibe-rs vibe
nano: pico
neovim: nvim nvim-qt neovide
net: dotnet
nix: nix-build nix-shell nix-env nix-store nixos-rebuild nom home-manager
node: nodejs npm npx tsx ts-node nodemon
notion: ntn
orbstack: orb orbctl
php: composer artisan
pi: pi-coding-agent
pnpm: pnpx
podman: podman-compose
poolside: pool
postgres: psql pg_dump
proton-pass: pass-cli
python: ipython pip pytest py
redis: redis-cli redis-server
ruby: irb bundle bundler rails rake gem
rust: cargo rustc rustup rust-analyzer
shell-caffeinate: caffeinate
shell-container: distrobox toolbox
shell-log: log tail head journalctl lnav
shell-sleep: sleep
shell-top: top atop gtop gotop btm glances nvtop
snowflake: snow snowsql
sqlite: sqlite3
sublime: subl sublime_text
swift: swiftc
tailscale: tailscaled
terraform: tofu
turborepo: turbo
unreal-engine: unrealeditor unrealbuildtool ue4 ue4editor
uv: uvx
vim: vi view vimdiff gvim mvim
vite-2: vite-plus
xcode: xcodebuild xcrun xcbuild";

/// Scripts known by where they live, not by their names.
const SCRIPTS: [(&str, &str); 4] = [
    ("copilot/npm-loader.js", "copilot"),
    ("pi-coding-agent/dist/cli.js", "pi"),
    ("pi-coding-agent/dist/bundle/cli.js", "pi"),
    ("vibe/_bin/vibe-rs", "mistral"),
];

impl Default for Program {
    fn default() -> Self {
        Self("shell")
    }
}

impl Program {
    /// The icon of the first of a program's names to have one, if any has.
    pub fn named<'a>(names: impl IntoIterator<Item = &'a str>) -> Option<Self> {
        names.into_iter().find_map(Self::known)
    }

    /// A name may be a path, in any case, and may end in a version, as `python3.12` does.
    fn known(name: &str) -> Option<Self> {
        let script = |(end, _): &&(&str, &str)| {
            let before = name.strip_suffix(end);
            before.is_some_and(|before| before.is_empty() || before.ends_with('/'))
        };
        if let Some((_, icon)) = SCRIPTS.iter().find(script) {
            return Self::icon(icon);
        }
        let name = name.rsplit(['/', '\\']).next()?;
        // A login shell is started as its name after a dash.
        let name = name
            .trim_start_matches('-')
            .to_lowercase()
            .replace(' ', "-");
        let name = name.strip_suffix(".exe").unwrap_or(&name);
        let bare = name.trim_end_matches(|c: char| c.is_ascii_digit() || ".-".contains(c));
        [name, bare].into_iter().find_map(|name| {
            let alias = ALIASES.lines().find_map(|line| {
                let (icon, names) = line.split_once(": ")?;
                names.split(' ').any(|alias| alias == name).then_some(icon)
            });
            // The shell's icons, and .NET's, are not named for commands.
            let named = !(name.starts_with("shell") || name == "net");
            Self::icon(alias.or(named.then_some(name))?)
        })
    }

    fn icon(name: &str) -> Option<Self> {
        let icon = ICONS.iter().find(|icon| **icon == name)?;
        Some(Self(icon))
    }

    pub fn name(self) -> &'static str {
        self.0
    }

    /// Whether it is a coding agent: a program worth watching while it is at
    /// rest, since it will be at work again.
    pub fn agent(self) -> bool {
        #[rustfmt::skip]
        const AGENTS: [&str; 19] = [
            "amp", "claude-code", "codex", "copilot", "crush", "cursor", "devin", "droid", "gemini",
            "goose", "grok", "ibm-bob", "junie", "kimi", "kiro", "mistral", "opencode", "pi", "poolside",
        ];
        AGENTS.contains(&self.0)
    }

    /// The icon as an SVG, if it was at hand.
    pub fn drawing(self) -> Option<&'static str> {
        let icon = DRAWINGS.iter().find(|(icon, _)| *icon == self.0)?;
        Some(icon.1)
    }
}

#[cfg(test)]
#[path = "../tests/unit/program.rs"]
mod tests;
