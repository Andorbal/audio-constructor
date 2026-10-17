//! Compile reports: problems found while turning a project into a schedule.
//! Errors never stop the rest of the graph from running; the offending wire
//! or node is left out and reported here.

use std::fmt;

use ac_core::NodePath;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        })
    }
}

/// One problem, attached to a node (by flattened path) and optionally a socket.
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub node: Option<NodePath>,
    pub socket: Option<String>,
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.severity)?;
        if let Some(n) = &self.node {
            write!(f, " [{n}")?;
            if let Some(s) = &self.socket {
                write!(f, ":{s}")?;
            }
            write!(f, "]")?;
        }
        write!(f, ": {}", self.message)
    }
}

/// Summary numbers for a compiled graph.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompileStats {
    pub nodes: usize,
    pub steps: usize,
    pub sample_buffers: usize,
    pub params: usize,
    /// Nodes fed by the outside world (AudioIn, MidiIn).
    pub live_sources: Vec<NodePath>,
    /// Total latency reported by nodes (Phase 2 compensates it).
    pub latency: u32,
}

/// Everything the compiler has to say about a project.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompileReport {
    pub diagnostics: Vec<Diagnostic>,
    pub stats: CompileStats,
}

impl CompileReport {
    pub fn push(
        &mut self,
        severity: Severity,
        node: Option<&NodePath>,
        socket: Option<&str>,
        message: impl Into<String>,
    ) {
        self.diagnostics.push(Diagnostic {
            severity,
            node: node.cloned(),
            socket: socket.map(str::to_string),
            message: message.into(),
        });
    }

    pub fn error(&mut self, node: Option<&NodePath>, socket: Option<&str>, msg: impl Into<String>) {
        self.push(Severity::Error, node, socket, msg);
    }

    pub fn warning(
        &mut self,
        node: Option<&NodePath>,
        socket: Option<&str>,
        msg: impl Into<String>,
    ) {
        self.push(Severity::Warning, node, socket, msg);
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }

    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
    }

    /// Diagnostics attached to a node or anything inside it (for group badges).
    pub fn for_node<'a>(&'a self, path: &'a NodePath) -> impl Iterator<Item = &'a Diagnostic> + 'a {
        self.diagnostics.iter().filter(move |d| {
            d.node
                .as_ref()
                .is_some_and(|n| n.0.len() >= path.0.len() && n.0[..path.0.len()] == path.0[..])
        })
    }
}

impl fmt::Display for CompileReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for d in &self.diagnostics {
            writeln!(f, "{d}")?;
        }
        Ok(())
    }
}
