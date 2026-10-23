//! Diagram rendering through the binary: mermaid and graphviz drawn as images,
//! fenced diagram blocks in markdown, fallbacks to source, the cat contract.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SCAT: &str = env!("CARGO_BIN_EXE_scat");

const FLOW: &str = "flowchart LR\n  A[Start] --> B{Decision}\n  B -->|Yes| C[OK]\n";

/// Written once per test binary: tests run in parallel and must not rewrite files
/// another test is reading.
fn fixtures() -> PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(make_fixtures).clone()
}

fn make_fixtures() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("diagrams");
    std::fs::create_dir_all(&dir).unwrap();
    let files: &[(&str, &str)] = &[
        ("flow.mmd", FLOW),
        ("broken.mmd", "flowchart LR\n  A -->\n"),
        ("graph.dot", "digraph G { a -> b; b -> c; }\n"),
        ("broken.dot", "digraph { a -> ; }\n"),
        ("seq.puml", "@startuml\nAlice -> Bob: hi\n@enduml\n"),
        (
            "doc.md",
            "# Doc\n\nBefore.\n\n```mermaid\ngraph TD\n  A --> B\n```\n\nAfter.\n",
        ),
    ];
    for (name, text) in files {
        std::fs::write(dir.join(name), text).unwrap();
    }
    dir
}

fn run_with(args: &[&str], dir: &Path, path: Option<&str>) -> Output {
    let mut cmd = Command::new(SCAT);
    cmd.args(args)
        .current_dir(dir)
        .env_remove("SCAT_CONFIG")
        .env_remove("SCAT_PROTOCOL")
        .env("HOME", dir)
        .env_remove("XDG_CONFIG_HOME")
        .env("COLUMNS", "60")
        .env("LINES", "40")
        .env("COLORTERM", "truecolor")
        .env("LC_ALL", "C.UTF-8");
    if let Some(path) = path {
        cmd.env("PATH", path);
    }
    cmd.output().unwrap()
}

fn run(args: &[&str], dir: &Path) -> Output {
    run_with(args, dir, None)
}

fn has_dot() -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join("dot").is_file()))
}

#[test]
fn mermaid_draws_half_blocks_and_a_caption() {
    let dir = fixtures();
    let out = run(
        &["-f", "--protocol=blocks", "--color=always", "flow.mmd"],
        &dir,
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains('▀'), "{text}");
    let last = text.lines().last().unwrap();
    assert_eq!(last, "\x1b[2mflow.mmd  mermaid diagram (built in)\x1b[22m");
    assert!(out.stderr.is_empty());
}

#[test]
fn dot_draws_with_graphviz_or_the_built_in_layout() {
    let dir = fixtures();
    // Without dot on PATH the built-in layout draws it.
    let out = run_with(
        &["-f", "--protocol=blocks", "--color=always", "graph.dot"],
        &dir,
        Some("/nonexistent"),
    );
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains('▀'), "{text}");
    assert!(text.ends_with("graph.dot  graphviz diagram (built in)\x1b[22m\n"));
    if !has_dot() {
        return;
    }
    let out = run(
        &["-f", "--protocol=blocks", "--color=always", "graph.dot"],
        &dir,
    );
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains('▀'), "{text}");
    assert!(text.ends_with("graph.dot  graphviz diagram (dot)\x1b[22m\n"));
}

#[test]
fn markdown_draws_mermaid_blocks() {
    let dir = fixtures();
    let out = run(
        &["-f", "--protocol=blocks", "--color=always", "doc.md"],
        &dir,
    );
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains('▀'), "{text}");
    assert!(text.contains("mermaid diagram"), "{text}");
    assert!(!text.contains("A --> B"), "{text}");
    assert!(text.contains("After."));
    // Without colours the block stays code.
    let out = run(&["-f", "--color=never", "doc.md"], &dir);
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("A --> B"), "{text}");
    assert!(!text.contains('▀'));
}

#[test]
fn piped_output_is_the_file() {
    let dir = fixtures();
    for name in ["flow.mmd", "graph.dot", "seq.puml", "doc.md"] {
        let out = run(&[name], &dir);
        assert_eq!(out.stdout, std::fs::read(dir.join(name)).unwrap(), "{name}");
    }
}

#[test]
fn forced_without_protocol_into_a_pipe_shows_source() {
    let dir = fixtures();
    let out = run(&["-f", "--color=never", "flow.mmd"], &dir);
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("A[Start] --> B{Decision}"), "{text}");
    assert!(out.stderr.is_empty());
}

#[test]
fn broken_diagrams_fall_back_to_source_and_scat_goes_on() {
    let dir = fixtures();
    let out = run_with(
        &[
            "-f",
            "--protocol=kitty",
            "--color=never",
            "broken.mmd",
            "broken.dot",
            "flow.mmd",
        ],
        &dir,
        Some("/nonexistent"),
    );
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(text.contains("A -->"), "{text}");
    assert!(text.contains("digraph { a -> ; }"), "{text}");
    assert!(
        text.contains("flow.mmd  mermaid diagram (built in)"),
        "{text}"
    );
    assert!(
        stderr.contains("broken.mmd: cannot draw mermaid diagram: "),
        "{stderr}"
    );
    assert!(
        stderr.contains("broken.dot: cannot draw graphviz diagram: "),
        "{stderr}"
    );
    assert!(stderr.contains("install graphviz"), "{stderr}");
    assert_eq!(stderr.lines().count(), 2, "{stderr}");
}

#[test]
fn plantuml_without_the_tool_shows_source_and_a_hint() {
    let dir = fixtures();
    let out = run_with(
        &["-f", "--protocol=kitty", "--color=never", "seq.puml"],
        &dir,
        Some("/nonexistent"),
    );
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("Alice -> Bob: hi"), "{text}");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("install plantuml"), "{stderr}");
}

#[test]
fn code_renderer_shows_the_source() {
    let dir = fixtures();
    std::fs::create_dir_all(dir.join("code/scat")).unwrap();
    std::fs::write(
        dir.join("code/scat/config.toml"),
        "[[rule]]\nmatch = { ext = \"mmd\" }\nrenderer = \"code\"\n",
    )
    .unwrap();
    let out = Command::new(SCAT)
        .args(["-f", "--protocol=kitty", "--color=never", "flow.mmd"])
        .current_dir(&dir)
        .env_remove("SCAT_PROTOCOL")
        .env("SCAT_CONFIG", dir.join("code/scat/config.toml"))
        .env("LC_ALL", "C.UTF-8")
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("A[Start] --> B{Decision}"), "{text}");
    assert!(!text.contains("\x1b_G"));
}
