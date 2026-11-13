//! The machine's manifest: what is installed, probed once a day for every project. Workers
//! spent turns finding it out for themselves (`Get-Command`, `$env:Path +=`, `rc` off the PATH,
//! clang without the MIPS target), so the hub keeps one file, `machine.txt`, probed at the
//! first walk of the day and on `rc doctor`: each tool's resolved path and version, a
//! compiler's targets, the SDK roots, the WSL distributions and the tools each holds. A run's
//! harness gets each found tool's directory on its PATH, and the lines of the tools the card
//! names ride in its prompt.

use std::fmt::Write;
use std::io::Read;
use std::path::{Path, PathBuf};

/// The manifest's file in the hub.
pub const MANIFEST_FILE: &str = "machine.txt";

/// The lock one probe takes, so two walks starting at once do not probe twice.
const MANIFEST_LOCK: &str = "manifest";

/// The tools every manifest asks after, on top of a project's own `[worker] allow` programs:
/// the default allowlist's and the toolchains the cards of this machine's projects reach for.
pub const TOOLS: [&str; 20] = [
    "rc", "claude", "cargo", "rustc", "rustup", "rustfmt", "git", "python", "py", "node", "clang", "clang++", "gcc", "cmake", "ninja",
    "make", "java", "javac", "gradle", "wsl",
];

/// The environment variables that name an SDK root, recorded when they point at a directory.
const SDK_VARS: [&str; 10] = [
    "JAVA_HOME",
    "ANDROID_HOME",
    "ANDROID_SDK_ROOT",
    "ANDROID_NDK_HOME",
    "VULKAN_SDK",
    "WindowsSdkDir",
    "VCINSTALLDIR",
    "DOTNET_ROOT",
    "GOROOT",
    "LLVM_PATH",
];

/// The longest one probe of a program may take before it is stopped: a tool that hangs on
/// `--version` costs the walk that much once a day, not more.
const PROBE_WAIT: std::time::Duration = std::time::Duration::from_secs(20);

/// The longest a version line keeps, for the prompt's sake.
const VERSION_WIDTH: usize = 90;

/// One tool as the probe found it: where it resolved, what `--version` said, and a
/// compiler's targets.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tool {
    pub name: String,
    /// `None` when the tool is not on this machine.
    pub path: Option<PathBuf>,
    pub version: String,
    pub targets: Vec<String>,
}

/// What the machine holds, as of `day`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    /// The UTC day of the probe, `YYYY-MM-DD`.
    pub day: String,
    pub tools: Vec<Tool>,
    /// Each SDK root by its variable or name.
    pub sdks: Vec<(String, PathBuf)>,
    /// Each WSL distribution and the tools on its own PATH.
    pub wsl: Vec<(String, Vec<String>)>,
}

impl Manifest {
    /// The manifest as its file holds it: one tab-separated line a record.
    #[must_use]
    pub fn to_text(&self) -> String {
        let field = |text: &str| text.replace(['\t', '\n', '\r'], " ");
        let mut text = format!("day\t{}\n", self.day);
        for tool in &self.tools {
            let path = tool.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
            let _ = writeln!(text, "tool\t{}\t{}\t{}\t{}", field(&tool.name), field(&path), field(&tool.version), tool.targets.join(","));
        }
        for (name, root) in &self.sdks {
            let _ = writeln!(text, "sdk\t{}\t{}", field(name), field(&root.display().to_string()));
        }
        for (distro, holds) in &self.wsl {
            let _ = writeln!(text, "wsl\t{}\t{}", field(distro), holds.join(" "));
        }
        text
    }

    /// The manifest of a file's text; a line that does not read is skipped.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut manifest = Self::default();
        for line in text.lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            match fields.as_slice() {
                ["day", day, ..] => (*day).clone_into(&mut manifest.day),
                ["tool", name, path, version, rest @ ..] => manifest.tools.push(Tool {
                    name: (*name).to_owned(),
                    path: Some(PathBuf::from(path)).filter(|_| !path.is_empty()),
                    version: (*version).to_owned(),
                    targets: rest.first().map(|t| t.split(',').filter(|t| !t.is_empty()).map(str::to_owned).collect()).unwrap_or_default(),
                }),
                ["sdk", name, root, ..] => manifest.sdks.push(((*name).to_owned(), PathBuf::from(root))),
                ["wsl", distro, rest @ ..] => manifest.wsl.push((
                    (*distro).to_owned(),
                    rest.first().map(|h| h.split_whitespace().map(str::to_owned).collect()).unwrap_or_default(),
                )),
                _ => {}
            }
        }
        manifest
    }

    /// The tool of `name`, when the manifest asked after it.
    #[must_use]
    pub fn tool(&self, name: &str) -> Option<&Tool> {
        self.tools.iter().find(|t| t.name.eq_ignore_ascii_case(name))
    }

    /// The directories of the tools found, each once, in the manifest's order: what a run's
    /// PATH gets.
    #[must_use]
    pub fn dirs(&self) -> Vec<PathBuf> {
        let mut dirs: Vec<PathBuf> = Vec::new();
        for dir in self.tools.iter().filter_map(|t| t.path.as_deref()?.parent()) {
            if !dirs.iter().any(|d| d == dir) {
                dirs.push(dir.to_path_buf());
            }
        }
        dirs
    }

    /// Every line of the manifest, as `rc doctor` prints it.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let mut lines: Vec<String> = self.tools.iter().map(tool_line).collect();
        lines.extend(self.sdks.iter().map(|(name, root)| sdk_line(name, root)));
        lines.extend(self.wsl.iter().map(|(distro, holds)| wsl_line(distro, holds)));
        lines
    }

    /// The lines of what `text` names by word: a tool by its name, an SDK by its variable or
    /// the word before its `_HOME`, a WSL distribution by its name or by `wsl`.
    #[must_use]
    pub fn lines_for(&self, text: &str) -> Vec<String> {
        let words: std::collections::BTreeSet<String> =
            text.split(|c: char| !(c.is_alphanumeric() || matches!(c, '+' | '_' | '-'))).map(str::to_lowercase).collect();
        let named = |name: &str| words.contains(&name.to_lowercase());
        let mut lines: Vec<String> = self.tools.iter().filter(|t| named(&t.name)).map(tool_line).collect();
        lines.extend(
            self.sdks
                .iter()
                .filter(|(name, _)| named(name) || name.split_once('_').is_some_and(|(stem, _)| named(stem)))
                .map(|(name, root)| sdk_line(name, root)),
        );
        lines.extend(self.wsl.iter().filter(|(distro, _)| named("wsl") || named(distro)).map(|(distro, holds)| wsl_line(distro, holds)));
        lines
    }

    /// The block of a run's prompt for the card's `text`: the lines of what it names, under a
    /// line saying the PATH holds them; empty when it names nothing the manifest knows.
    #[must_use]
    pub fn block_for(&self, text: &str) -> String {
        let lines = self.lines_for(text);
        if lines.is_empty() {
            return String::new();
        }
        let shell = if cfg!(windows) {
            "; Git Bash runs with MSYS_NO_PATHCONV=1, so a `/c` or `C:/…` argument reaches the program as written"
        } else {
            ""
        };
        let lines: Vec<String> = lines.iter().map(|l| format!("- {l}")).collect();
        format!(
            "The tools the card names, from the machine's manifest of {} (each found one's directory is on your PATH{shell}):\n{}",
            self.day,
            lines.join("\n")
        )
    }
}

/// A tool's line: `clang: <path> · <version> · targets: mips, x86`, or `not found`.
fn tool_line(tool: &Tool) -> String {
    let Some(path) = &tool.path else { return format!("{}: not found on this machine", tool.name) };
    let mut line = format!("{}: {}", tool.name, path.display());
    if !tool.version.is_empty() {
        let _ = write!(line, " · {}", tool.version);
    }
    if !tool.targets.is_empty() {
        let _ = write!(line, " · targets: {}", tool.targets.join(", "));
    }
    line
}

fn sdk_line(name: &str, root: &Path) -> String {
    format!("sdk {name}: {}", root.display())
}

fn wsl_line(distro: &str, holds: &[String]) -> String {
    if holds.is_empty() { format!("wsl {distro}: none of the tools") } else { format!("wsl {distro}: {}", holds.join(", ")) }
}

/// The manifest in the hub, when one was probed.
#[must_use]
pub fn load() -> Option<Manifest> {
    load_in(&super::dir())
}

/// [`load`] from the directory named, for the tests.
#[must_use]
pub fn load_in(dir: &Path) -> Option<Manifest> {
    std::fs::read_to_string(dir.join(MANIFEST_FILE)).ok().map(|text| Manifest::parse(&text))
}

/// Writes the manifest into `dir`.
///
/// # Errors
/// When the directory or the file cannot be written.
pub fn save_in(dir: &Path, manifest: &Manifest) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(MANIFEST_FILE), manifest.to_text())
}

/// The tools a manifest for the project at `root` asks after: [`TOOLS`], then the programs of
/// its `[worker] allow` and its check; a project's own script (`.\gw.ps1`) is no tool of the
/// machine's.
#[must_use]
pub fn names_of(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = TOOLS.map(str::to_owned).to_vec();
    if let Ok(router) = crate::router::Router::load(&super::project_root_of(root)) {
        let worker = router.worker;
        for program in worker.allow.iter().map(String::as_str).chain(worker.check()).filter_map(program_of) {
            if !names.iter().any(|n| n.eq_ignore_ascii_case(&program)) {
                names.push(program);
            }
        }
    }
    names
}

/// The program an allow entry or a command starts with: `ninja` of `Bash(ninja *)`.
fn program_of(entry: &str) -> Option<String> {
    let inner = entry.split_once('(').map_or(entry, |(_, rest)| rest.trim_end_matches(')'));
    let word = inner.split_whitespace().find(|w| *w != "&")?.trim_end_matches('*').trim_matches(['"', '\'']);
    (!word.is_empty() && !word.contains(['/', '\\'])).then(|| word.to_owned())
}

/// The manifest for a walk in `root`: the hub's when it was probed today and asked after
/// every tool the project names, else probed again and saved. A walk's first of the day pays
/// for the probe; the rest of the day's walks, of every project, read the file. The tests
/// never probe the machine.
#[must_use]
pub fn daily(root: &Path) -> Option<Manifest> {
    if cfg!(test) {
        return load();
    }
    let names = names_of(root);
    let today = super::day_of(super::unix_now());
    let fresh = |m: &Manifest| m.day == today && names.iter().all(|n| m.tool(n).is_some());
    if let Some(manifest) = load().filter(&fresh) {
        return Some(manifest);
    }
    // one probe across the projects: a walk that waited reads the one just written
    let _probe = super::guard(MANIFEST_LOCK);
    let old = load();
    if let Some(manifest) = old.clone().filter(&fresh) {
        return Some(manifest);
    }
    // the tools another project asked after stay in the file
    let mut all = names;
    for tool in old.iter().flat_map(|m| &m.tools) {
        if !all.iter().any(|n| n.eq_ignore_ascii_case(&tool.name)) {
            all.push(tool.name.clone());
        }
    }
    Some(probe_and_save(&all, &today))
}

/// Probes the machine for the tools `root` names and saves the manifest, whatever day the
/// last was: `rc doctor`'s.
#[must_use]
pub fn refresh(root: &Path) -> Manifest {
    let mut names = names_of(root);
    for tool in load().iter().flat_map(|m| &m.tools) {
        if !names.iter().any(|n| n.eq_ignore_ascii_case(&tool.name)) {
            names.push(tool.name.clone());
        }
    }
    let _probe = super::guard(MANIFEST_LOCK);
    probe_and_save(&names, &super::day_of(super::unix_now()))
}

fn probe_and_save(names: &[String], today: &str) -> Manifest {
    let manifest = probe_with(names, &search_dirs(), &ask, today);
    if let Err(e) = save_in(&super::dir(), &manifest) {
        tracing::warn!("the machine's manifest was not saved: {e}");
    }
    manifest
}

/// The manifest of `names` found under `search`, in order, with `ask(program, args)` for what
/// a program prints: its version, a compiler's targets, the WSL distributions and what each
/// holds. The SDK roots come from the environment and the usual places.
#[must_use]
pub fn probe_with(names: &[String], search: &[PathBuf], ask: &dyn Fn(&Path, &[&str]) -> Option<String>, today: &str) -> Manifest {
    let mut tools: Vec<Tool> = names.iter().map(|name| probe_tool(name, search, ask)).collect();
    // rustc's targets are the ones rustup installed, not every one it knows
    let rustup = tools.iter().find(|t| t.name == "rustup").and_then(|t| t.path.clone());
    if let Some(rustup) = rustup
        && let Some(rustc) = tools.iter_mut().find(|t| t.name == "rustc" && t.path.is_some())
    {
        rustc.targets = ask(&rustup, &["target", "list", "--installed"])
            .map(|text| text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect())
            .unwrap_or_default();
    }
    let wsl =
        tools.iter().find(|t| t.name == "wsl").and_then(|t| t.path.clone()).map(|wsl| wsl_distros(&wsl, names, ask)).unwrap_or_default();
    Manifest { day: today.to_owned(), tools, sdks: sdk_roots(), wsl }
}

fn probe_tool(name: &str, search: &[PathBuf], ask: &dyn Fn(&Path, &[&str]) -> Option<String>) -> Tool {
    let Some(path) = resolve(name, search) else { return Tool { name: name.to_owned(), ..Tool::default() } };
    // `wsl --version` starts the machine's VM; its distributions are asked once, below
    let version = if name == "wsl" { None } else { ask(&path, &["--version"]) };
    let version = version
        .as_deref()
        .and_then(|text| text.lines().map(str::trim).find(|l| !l.is_empty()))
        .map(|line| clip(line, VERSION_WIDTH))
        .unwrap_or_default();
    // a clang lists the back ends it was built with: the MIPS card needs to know before it compiles
    let targets = if name.starts_with("clang") {
        ask(&path, &["-print-targets"]).map(|text| clang_targets(&text)).unwrap_or_default()
    } else {
        Vec::new()
    };
    Tool { name: name.to_owned(), path: Some(path), version, targets }
}

/// The back ends of `clang -print-targets`: the first word of each line under its header.
fn clang_targets(text: &str) -> Vec<String> {
    let mut targets: Vec<String> = text
        .lines()
        .skip_while(|l| !l.contains("Registered Targets"))
        .skip(1)
        .filter_map(|l| l.split_whitespace().next())
        .map(str::to_owned)
        .collect();
    targets.sort();
    targets.dedup();
    targets
}

/// The WSL distributions and, in each, which of `names` its own PATH finds.
fn wsl_distros(wsl: &Path, names: &[String], ask: &dyn Fn(&Path, &[&str]) -> Option<String>) -> Vec<(String, Vec<String>)> {
    let Some(list) = ask(wsl, &["-l", "-q"]) else { return Vec::new() };
    // a Windows wrapper (`py`, `rc`) is no tool inside a distribution
    let inside: Vec<&str> = names.iter().map(String::as_str).filter(|n| !matches!(*n, "wsl" | "py" | "rc" | "claude")).collect();
    let script = format!("for t in {}; do command -v \"$t\" >/dev/null 2>&1 && printf '%s ' \"$t\"; done", inside.join(" "));
    list.lines()
        .map(str::trim)
        .filter(|d| !d.is_empty() && !d.contains(' '))
        .map(|distro| {
            let holds = ask(wsl, &["-d", distro, "-e", "sh", "-c", &script])
                .map(|t| t.split_whitespace().map(str::to_owned).collect())
                .unwrap_or_default();
            (distro.to_owned(), holds)
        })
        .collect()
}

/// The SDK roots of this machine: the variables of [`SDK_VARS`] that name a directory, then on
/// Windows the usual places a kit installs to.
fn sdk_roots() -> Vec<(String, PathBuf)> {
    let mut roots: Vec<(String, PathBuf)> = SDK_VARS
        .iter()
        .filter_map(|var| Some(((*var).to_owned(), PathBuf::from(std::env::var_os(var)?))))
        .filter(|(_, p)| p.is_dir())
        .collect();
    if cfg!(windows) {
        let under = |var: &str, rest: &str| std::env::var_os(var).map(|d| PathBuf::from(d).join(rest));
        let places = [
            ("Windows Kits", under("ProgramFiles(x86)", r"Windows Kits\10")),
            ("Visual Studio", under("ProgramFiles", "Microsoft Visual Studio")),
            ("Android SDK", under("LOCALAPPDATA", r"Android\Sdk")),
        ];
        for (name, place) in places {
            if let Some(place) = place.filter(|p| p.is_dir())
                && !roots.iter().any(|(_, r)| r == &place)
            {
                roots.push((name.to_owned(), place));
            }
        }
    }
    roots
}

/// Where a tool is looked for: the PATH, then the places a tool installs to without putting
/// itself on it, and this binary's own directory, the installed `rc`'s.
fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if let Some(own) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)) {
        dirs.push(own);
    }
    dirs.push(crate::install::default_dir());
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    if let Some(home) = &home {
        dirs.push(home.join(".cargo").join("bin"));
    }
    if cfg!(windows) {
        if let Some(program_files) = std::env::var_os("ProgramFiles").map(PathBuf::from) {
            for rest in [r"LLVM\bin", r"Git\cmd", r"Git\bin", r"Git\usr\bin", r"CMake\bin", "nodejs"] {
                dirs.push(program_files.join(rest));
            }
        }
        if let Some(java) = std::env::var_os("JAVA_HOME") {
            dirs.push(PathBuf::from(java).join("bin"));
        }
        // the python.org installer's per-user place, one directory a version
        if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
            && let Ok(entries) = std::fs::read_dir(local.join("Programs").join("Python"))
        {
            let mut pythons: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
            pythons.sort();
            dirs.extend(pythons.into_iter().rev());
        }
    } else {
        dirs.extend(["/usr/local/bin", "/opt/homebrew/bin", "/usr/bin"].map(PathBuf::from));
    }
    dirs
}

/// The first file under `search` that runs as `name`: on Windows with one of the executable
/// extensions, elsewhere as named.
#[must_use]
pub fn resolve(name: &str, search: &[PathBuf]) -> Option<PathBuf> {
    let extensions: &[&str] = if cfg!(windows) { &[".exe", ".cmd", ".bat", ".com", ""] } else { &[""] };
    search.iter().flat_map(|dir| extensions.iter().map(move |ext| dir.join(format!("{name}{ext}")))).find(|path| path.is_file())
}

/// What `program args` prints, stdout or else stderr, within [`PROBE_WAIT`]; `None` when it
/// does not start. A UTF-16 output (`wsl -l`) is read as such.
fn ask(program: &Path, args: &[&str]) -> Option<String> {
    let mut command = std::process::Command::new(program);
    command.args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // a probe from a walk with no console opens no window of its own
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().ok()?;
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < PROBE_WAIT => std::thread::sleep(std::time::Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
    let stdout = decode(&stdout.join().unwrap_or_default());
    let text = if stdout.trim().is_empty() { decode(&stderr.join().unwrap_or_default()) } else { stdout };
    Some(text)
}

/// Reads a pipe to its end on a thread of its own, so a chatty program cannot stall on a full one.
fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut bytes);
        }
        bytes
    })
}

/// Bytes a program printed as text: UTF-16LE when every other byte is zero, as `wsl.exe`
/// writes, else UTF-8.
fn decode(bytes: &[u8]) -> String {
    let wide = bytes.starts_with(&[0xff, 0xfe]) || (bytes.len() >= 2 && bytes.iter().skip(1).step_by(2).take(16).all(|b| *b == 0));
    let text = if wide {
        let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    text.replace(['\u{feff}', '\0'], "")
}

fn clip(text: &str, width: usize) -> String {
    if text.chars().count() <= width { text.to_owned() } else { format!("{}…", text.chars().take(width - 1).collect::<String>()) }
}

#[cfg(test)]
mod tests;
