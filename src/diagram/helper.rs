//! The diagram helper process: `scat --internal-diagram mermaid|dot`, source on
//! standard input, SVG on standard output.
//!
//! Only this code runs `mermaid-rs-renderer` and `layout-rs`. It is started by
//! [`super::run`] with resource limits; errors go to stderr as one line and exit with
//! status 2.

use std::io::{Read, Write};
use std::process::Command;

/// Hidden first argument that turns scat into the diagram helper process.
pub const FLAG: &str = "--internal-diagram";

/// Runs the helper when scat was started as one; `None` for a normal run.
pub fn internal_main() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next()? != FLAG {
        return None;
    }
    let kind = args.next().and_then(|a| a.into_string().ok());
    Some(main(kind.as_deref().unwrap_or_default()))
}

/// A command that runs the helper for `kind` (`mermaid` or `dot`).
pub fn command(kind: &str) -> Command {
    // /proc/self/exe still works when the binary was replaced or deleted since start.
    let exe = if std::path::Path::new("/proc/self/exe").exists() {
        std::path::PathBuf::from("/proc/self/exe")
    } else {
        std::env::current_exe().unwrap_or_else(|_| "scat".into())
    };
    let mut cmd = Command::new(exe);
    cmd.arg(FLAG).arg(kind);
    cmd
}

fn main(kind: &str) -> i32 {
    let mut source = Vec::new();
    if std::io::stdin()
        .take(super::MAX_SOURCE_BYTES + 1)
        .read_to_end(&mut source)
        .is_err()
        || source.len() as u64 > super::MAX_SOURCE_BYTES
    {
        eprintln!("cannot read the diagram source");
        return 2;
    }
    let source = String::from_utf8_lossy(&source);
    match render(kind, &source) {
        Ok(svg) => {
            let mut stdout = std::io::stdout().lock();
            if stdout
                .write_all(svg.as_bytes())
                .and_then(|()| stdout.flush())
                .is_err()
            {
                return 1;
            }
            0
        }
        Err(message) => {
            eprintln!("{message}");
            2
        }
    }
}

/// Renders `source` with a built-in engine. Runs in the helper process only (and in
/// tests): the engines may panic on malformed input.
pub fn render(kind: &str, source: &str) -> Result<String, String> {
    match kind {
        "mermaid" => mermaid(source),
        "dot" => dot(source),
        _ => Err("usage error".into()),
    }
}

fn mermaid(source: &str) -> Result<String, String> {
    mermaid_rs_renderer::render(source).map_err(|err| {
        let text = err.to_string();
        // "unexpected token 'flowchart LR' at 1:1; expected invalid flowchart edge
        // syntax: A -->" reads better from the part after "expected".
        let text = text
            .split_once("; expected ")
            .map_or(text.as_str(), |(_, rest)| rest)
            .to_string();
        one_line(&text)
    })
}

fn dot(source: &str) -> Result<String, String> {
    use layout::backends::svg::SVGWriter;
    use layout::gv::{DotParser, GraphBuilder};

    let mut parser = DotParser::new(source);
    let graph = parser
        .process()
        .map_err(|err| format!("cannot parse: {}", one_line(&err)))?;
    let mut builder = GraphBuilder::new();
    builder.visit_graph(&graph);
    let mut visual = builder.get();
    if visual.num_nodes() == 0 {
        return Err("the graph has no nodes".into());
    }
    let mut svg = SVGWriter::new();
    visual.do_it(false, false, false, &mut svg);
    Ok(svg.finalize())
}

fn one_line(text: &str) -> String {
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or(text);
    line.trim().chars().take(200).collect()
}
