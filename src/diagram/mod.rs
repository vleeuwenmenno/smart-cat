//! Diagram sources drawn as images: mermaid, graphviz (dot) and PlantUML.
//!
//! Every engine turns the source into SVG, which the image code then draws with resvg
//! under the same limits as SVG files. Engines, in the order they are tried:
//!
//! - mermaid: the built-in `mermaid-rs-renderer`, then `mmdc` (mermaid-cli) on `PATH`.
//! - graphviz: `dot -Tsvg` on `PATH`, then the built-in `layout-rs` (a dot subset).
//! - PlantUML: `plantuml -tsvg -pipe` on `PATH` only. The PlantUML web server is
//!   never used; nothing here touches the network.
//!
//! The built-in engines parse untrusted text and may panic (release builds abort on
//! panic) or loop, so they run in a child process: scat re-runs itself with the hidden
//! first argument [`helper::FLAG`] and the source on standard input. External tools get
//! the source on standard input as well, an argv array, a timeout and an output cap.
//!
//! Nothing here runs unless a diagram is shown, so startup time is unaffected.

pub mod helper;
mod run;

use std::time::Duration;

/// Larger diagram sources are shown as text.
pub const MAX_SOURCE_BYTES: u64 = 1 << 20;

/// Most SVG read back from an engine.
const MAX_SVG_BYTES: u64 = 32 << 20;

/// Time limit for `dot` and the built-in engines.
const TIMEOUT: Duration = Duration::from_secs(10);

/// Time limit for `plantuml` and `mmdc`, which start a JVM or a browser first.
const SLOW_TIMEOUT: Duration = Duration::from_secs(30);

/// A diagram language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Mermaid,
    Graphviz,
    PlantUml,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Mermaid => "mermaid",
            Kind::Graphviz => "graphviz",
            Kind::PlantUml => "PlantUML",
        }
    }

    /// What to install when no engine could draw a diagram of this kind.
    pub fn install_hint(self) -> &'static str {
        match self {
            Kind::Mermaid => "mermaid-cli (mmdc) may draw it",
            Kind::Graphviz => "install graphviz (dot) for full dot support",
            Kind::PlantUml => "install plantuml to draw it",
        }
    }
}

/// A program that turns diagram source into SVG.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    /// `mermaid-rs-renderer`, in the helper process.
    MermaidBuiltin,
    /// mermaid-cli.
    Mmdc,
    /// graphviz `dot`.
    Dot,
    /// `layout-rs`, in the helper process.
    DotBuiltin,
    PlantUml,
}

impl Engine {
    /// Shown in the caption.
    pub fn name(self) -> &'static str {
        match self {
            Engine::MermaidBuiltin | Engine::DotBuiltin => "built in",
            Engine::Mmdc => "mmdc",
            Engine::Dot => "dot",
            Engine::PlantUml => "plantuml",
        }
    }

    /// The external program, `None` for built-in engines.
    pub fn program(self) -> Option<&'static str> {
        match self {
            Engine::MermaidBuiltin | Engine::DotBuiltin => None,
            Engine::Mmdc => Some("mmdc"),
            Engine::Dot => Some("dot"),
            Engine::PlantUml => Some("plantuml"),
        }
    }
}

/// The engines to try for `kind`, best first. `on_path` says whether a program is
/// installed.
pub fn engines(kind: Kind, on_path: impl Fn(&str) -> bool) -> Vec<Engine> {
    let candidates: &[Engine] = match kind {
        Kind::Mermaid => &[Engine::MermaidBuiltin, Engine::Mmdc],
        Kind::Graphviz => &[Engine::Dot, Engine::DotBuiltin],
        Kind::PlantUml => &[Engine::PlantUml],
    };
    candidates
        .iter()
        .copied()
        .filter(|e| e.program().is_none_or(&on_path))
        .collect()
}

/// The diagram kind for a file, by extension or by `--language` / a rule's language.
pub fn detect(file_name: Option<&str>, language: Option<&str>) -> Option<Kind> {
    if let Some(language) = language {
        return by_name(language);
    }
    let ext = file_name?.rsplit_once('.')?.1.to_ascii_lowercase();
    match ext.as_str() {
        "mmd" | "mermaid" => Some(Kind::Mermaid),
        "dot" | "gv" => Some(Kind::Graphviz),
        "puml" | "plantuml" | "pu" => Some(Kind::PlantUml),
        _ => None,
    }
}

/// The diagram kind for a language name (`-l`, a rule).
fn by_name(name: &str) -> Option<Kind> {
    match name.to_ascii_lowercase().as_str() {
        "mermaid" | "mmd" => Some(Kind::Mermaid),
        "dot" | "gv" | "graphviz" => Some(Kind::Graphviz),
        "plantuml" | "puml" => Some(Kind::PlantUml),
        _ => None,
    }
}

/// The diagram kind for the info string of a fenced code block in markdown. Only
/// mermaid and graphviz are drawn there; PlantUML would start a JVM per block.
pub fn fence_kind(lang: &str) -> Option<Kind> {
    match lang.to_ascii_lowercase().as_str() {
        "mermaid" => Some(Kind::Mermaid),
        "dot" | "graphviz" => Some(Kind::Graphviz),
        _ => None,
    }
}

/// Mermaid diagram keywords that may start a document.
const MERMAID_STARTS: &[&str] = &[
    "flowchart",
    "graph",
    "sequenceDiagram",
    "classDiagram",
    "classDiagram-v2",
    "stateDiagram",
    "stateDiagram-v2",
    "erDiagram",
    "journey",
    "gantt",
    "pie",
    "quadrantChart",
    "requirementDiagram",
    "gitGraph",
    "C4Context",
    "C4Container",
    "C4Component",
    "C4Dynamic",
    "C4Deployment",
    "mindmap",
    "timeline",
    "zenuml",
    "sankey-beta",
    "sankey",
    "xychart-beta",
    "xychart",
    "block-beta",
    "block",
    "packet-beta",
    "packet",
    "kanban",
    "architecture-beta",
    "architecture",
    "radar-beta",
    "treemap-beta",
];

/// Guesses the kind from the source text, for `renderer = "diagram"` on a file
/// without a diagram extension.
pub fn sniff(text: &str) -> Option<Kind> {
    let mut in_block_comment = false;
    for line in text.lines() {
        let line = line.trim();
        if in_block_comment {
            in_block_comment = !line.contains("*/");
            continue;
        }
        if line.is_empty()
            || line.starts_with("%%")
            || line.starts_with("//")
            || line.starts_with('#')
            || line.starts_with('\'')
        {
            continue;
        }
        if line.starts_with("/*") {
            in_block_comment = !line.contains("*/");
            continue;
        }
        if line.starts_with("@start") {
            return Some(Kind::PlantUml);
        }
        // Mermaid front matter (`---` ... `---`) comes before the diagram.
        if line == "---" {
            return Some(Kind::Mermaid);
        }
        let word = line
            .split(|c: char| c.is_whitespace() || c == ';' || c == '{')
            .next()
            .unwrap_or_default();
        let lower = word.to_ascii_lowercase();
        if lower == "digraph" || lower == "strict" {
            return Some(Kind::Graphviz);
        }
        // `graph {` and `graph name {` are dot; `graph TD` is mermaid.
        if lower == "graph" && line.contains('{') {
            return Some(Kind::Graphviz);
        }
        if MERMAID_STARTS.contains(&word) {
            return Some(Kind::Mermaid);
        }
        return None;
    }
    None
}

/// A diagram turned into SVG.
#[derive(Debug)]
pub struct Svg {
    pub data: Vec<u8>,
    pub engine: Engine,
}

/// Why a diagram could not be drawn.
#[derive(Debug)]
pub struct Failure {
    /// The first engine's error, or `None` when no engine is available.
    pub error: Option<String>,
    /// What to install, when an engine for this kind is missing.
    pub hint: Option<&'static str>,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.error, self.hint) {
            (Some(error), Some(hint)) => write!(f, "{error}; {hint}"),
            (Some(error), None) => f.write_str(error),
            (None, Some(hint)) => write!(f, "no renderer available; {hint}"),
            (None, None) => f.write_str("no renderer available"),
        }
    }
}

/// Turns `source` into SVG with the first engine that succeeds.
pub fn render(kind: Kind, source: &str) -> Result<Svg, Failure> {
    let mut first_error = None;
    let available = engines(kind, |name| crate::util::which(name).is_some());
    let missing = available.len() < engines(kind, |_| true).len();
    for engine in available {
        match run_engine(engine, source) {
            Ok(data) => return Ok(Svg { data, engine }),
            Err(err) => {
                // Built-in errors read best without a prefix; tools are named.
                first_error.get_or_insert(match engine.program() {
                    Some(program) => format!("{program}: {err}"),
                    None => err,
                });
            }
        }
    }
    Err(Failure {
        error: first_error,
        hint: missing.then(|| kind.install_hint()),
    })
}

fn run_engine(engine: Engine, source: &str) -> Result<Vec<u8>, String> {
    use std::process::Command;
    let input = source.as_bytes();
    let output = match engine {
        Engine::MermaidBuiltin => run::run(
            helper::command("mermaid"),
            input,
            TIMEOUT,
            MAX_SVG_BYTES,
            true,
        ),
        Engine::DotBuiltin => run::run(helper::command("dot"), input, TIMEOUT, MAX_SVG_BYTES, true),
        Engine::Dot => {
            let mut cmd = Command::new("dot");
            cmd.arg("-Tsvg");
            run::run(cmd, input, TIMEOUT, MAX_SVG_BYTES, false)
        }
        Engine::PlantUml => {
            let mut cmd = Command::new("plantuml");
            cmd.args(["-tsvg", "-pipe", "-charset", "UTF-8"])
                // No file includes and no URL fetches (`!include`, `!includeurl`).
                .env("PLANTUML_SECURITY_PROFILE", "SANDBOX");
            run::run(cmd, input, SLOW_TIMEOUT, MAX_SVG_BYTES, false)
        }
        Engine::Mmdc => return run_mmdc(input),
    };
    let output = output.map_err(|e| e.to_string())?;
    svg_part(output)
}

/// mermaid-cli with labels as SVG text: resvg cannot draw the HTML labels (in
/// `<foreignObject>`) that mmdc uses by default.
fn run_mmdc(input: &[u8]) -> Result<Vec<u8>, String> {
    let config = std::env::temp_dir().join(format!("scat-mmdc-{}.json", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&config)
        .map_err(|e| crate::util::io_message(&e))?;
    let written = std::io::Write::write_all(
        &mut file,
        br#"{"htmlLabels":false,"flowchart":{"htmlLabels":false},"class":{"htmlLabels":false}}"#,
    );
    drop(file);
    let result = written
        .map_err(|e| crate::util::io_message(&e))
        .and_then(|()| {
            let mut cmd = std::process::Command::new("mmdc");
            cmd.args([
                "--quiet",
                "--input",
                "-",
                "--output",
                "-",
                "--outputFormat",
                "svg",
            ])
            .arg("--configFile")
            .arg(&config);
            run::run(cmd, input, SLOW_TIMEOUT, MAX_SVG_BYTES, false).map_err(|e| e.to_string())
        });
    let _ = std::fs::remove_file(&config);
    svg_part(result?)
}

/// The SVG document in an engine's output, from its `<?xml` or `<svg` on.
fn svg_part(output: Vec<u8>) -> Result<Vec<u8>, String> {
    let start = find(&output, b"<?xml").or_else(|| find(&output, b"<svg"));
    match start {
        Some(start) if find(&output[start..], b"<svg").is_some() => Ok(output[start..].to_vec()),
        _ => Err("no SVG in the output".into()),
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Whether a markdown document has a fenced mermaid or graphviz block, which is
/// drawn as an image.
pub fn markdown_has_diagrams(text: &[u8]) -> bool {
    text.split(|&b| b == b'\n').any(|line| {
        let line = String::from_utf8_lossy(line);
        let line = line.trim_start();
        let Some(rest) = line
            .strip_prefix("```")
            .or_else(|| line.strip_prefix("~~~"))
        else {
            return false;
        };
        let rest = rest.trim_start_matches(['`', '~']).trim_start();
        let lang = rest
            .split(|c: char| c.is_whitespace() || c == ',' || c == '{')
            .next()
            .unwrap_or_default();
        fence_kind(lang).is_some()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_by_extension_and_language() {
        assert_eq!(detect(Some("flow.mmd"), None), Some(Kind::Mermaid));
        assert_eq!(detect(Some("a.MERMAID"), None), Some(Kind::Mermaid));
        assert_eq!(detect(Some("g.dot"), None), Some(Kind::Graphviz));
        assert_eq!(detect(Some("g.gv"), None), Some(Kind::Graphviz));
        assert_eq!(detect(Some("u.puml"), None), Some(Kind::PlantUml));
        assert_eq!(detect(Some("u.plantuml"), None), Some(Kind::PlantUml));
        assert_eq!(detect(Some("u.pu"), None), Some(Kind::PlantUml));
        assert_eq!(detect(Some("notes.txt"), None), None);
        assert_eq!(detect(None, None), None);
        assert_eq!(detect(Some("x.txt"), Some("mermaid")), Some(Kind::Mermaid));
        assert_eq!(
            detect(Some("x.txt"), Some("graphviz")),
            Some(Kind::Graphviz)
        );
        // A language that is not a diagram wins over the extension.
        assert_eq!(detect(Some("g.dot"), Some("rust")), None);
    }

    #[test]
    fn fenced_block_languages() {
        assert_eq!(fence_kind("mermaid"), Some(Kind::Mermaid));
        assert_eq!(fence_kind("dot"), Some(Kind::Graphviz));
        assert_eq!(fence_kind("graphviz"), Some(Kind::Graphviz));
        assert_eq!(fence_kind("plantuml"), None);
        assert_eq!(fence_kind("rust"), None);
    }

    #[test]
    fn sniffs_the_kind_from_the_source() {
        assert_eq!(sniff("graph TD\n  A --> B\n"), Some(Kind::Mermaid));
        assert_eq!(sniff("%% c\nsequenceDiagram\n"), Some(Kind::Mermaid));
        assert_eq!(
            sniff("---\ntitle: x\n---\nflowchart LR\n"),
            Some(Kind::Mermaid)
        );
        assert_eq!(sniff("digraph G { a -> b }"), Some(Kind::Graphviz));
        assert_eq!(sniff("// c\ngraph { a -- b }"), Some(Kind::Graphviz));
        assert_eq!(
            sniff("/* x\n y */\nstrict digraph {}"),
            Some(Kind::Graphviz)
        );
        assert_eq!(sniff("@startuml\nA -> B\n@enduml\n"), Some(Kind::PlantUml));
        assert_eq!(sniff("hello world\n"), None);
        assert_eq!(sniff(""), None);
    }

    #[test]
    fn fallback_order() {
        let all = |_: &str| true;
        let none = |_: &str| false;
        assert_eq!(
            engines(Kind::Mermaid, all),
            [Engine::MermaidBuiltin, Engine::Mmdc]
        );
        assert_eq!(engines(Kind::Mermaid, none), [Engine::MermaidBuiltin]);
        assert_eq!(
            engines(Kind::Graphviz, all),
            [Engine::Dot, Engine::DotBuiltin]
        );
        assert_eq!(engines(Kind::Graphviz, none), [Engine::DotBuiltin]);
        assert_eq!(engines(Kind::PlantUml, all), [Engine::PlantUml]);
        assert!(engines(Kind::PlantUml, none).is_empty());
        // Only the program asked about counts.
        assert_eq!(
            engines(Kind::Graphviz, |p| p == "mmdc"),
            [Engine::DotBuiltin]
        );
    }

    #[test]
    fn failure_messages() {
        let none = Failure {
            error: None,
            hint: Some(Kind::PlantUml.install_hint()),
        };
        assert_eq!(
            none.to_string(),
            "no renderer available; install plantuml to draw it"
        );
        let failed = Failure {
            error: Some("built in: cannot parse: x".into()),
            hint: Some(Kind::Graphviz.install_hint()),
        };
        assert_eq!(
            failed.to_string(),
            "built in: cannot parse: x; install graphviz (dot) for full dot support"
        );
    }

    #[test]
    fn svg_is_cut_from_the_output() {
        assert_eq!(
            svg_part(b"noise\n<svg x='1'></svg>".to_vec()).unwrap(),
            b"<svg x='1'></svg>"
        );
        assert!(svg_part(b"<?xml version='1.0'?><svg/>".to_vec()).is_ok());
        assert!(svg_part(b"error".to_vec()).is_err());
    }

    #[test]
    fn finds_fenced_diagrams_in_markdown() {
        assert!(markdown_has_diagrams(b"# x\n\n```mermaid\ngraph TD\n```\n"));
        assert!(markdown_has_diagrams(b"  ~~~ dot\ndigraph {}\n~~~\n"));
        assert!(markdown_has_diagrams(b"````graphviz {.x}\n"));
        assert!(!markdown_has_diagrams(b"```rust\nfn main() {}\n```\n"));
        assert!(!markdown_has_diagrams(b"mermaid\n"));
    }

    #[test]
    fn builtin_engines_make_svg() {
        let svg = helper::render("mermaid", "flowchart LR\n  A --> B\n").unwrap();
        assert!(svg.contains("<svg"), "{svg}");
        let svg = helper::render("dot", "digraph { a -> b }").unwrap();
        assert!(svg.contains("<svg"), "{svg}");
        assert!(helper::render("dot", "digraph { a -> ; }").is_err());
        assert!(helper::render("mermaid", "flowchart LR\n A -->\n").is_err());
    }
}
