use std::io::Write;

use serde::Serialize;

/// Global flags every command receives; `-C` is applied by main before dispatch.
pub struct Global {
    pub human: bool,
    pub repo: Option<String>,
    pub direct: bool,
}

pub trait Human {
    fn render(&self) -> String;
}

#[derive(Serialize)]
struct Meta {
    version: &'static str,
    schema: u32,
}

#[derive(Serialize)]
struct Envelope<'a, T> {
    stewards: Meta,
    #[serde(flatten)]
    payload: &'a T,
}

/// Prints `v` as enveloped JSON, or its markdown rendering with --human. Never panics on a
/// failed write: a closed pipe (`stewards check | head`) is dropped quietly so the command
/// still exits with its own code; any other write error is reported on stderr.
pub fn emit<T: Serialize + Human>(v: &T, human: bool) {
    let text = if human {
        v.render()
    } else {
        let envelope = Envelope {
            stewards: Meta {
                version: env!("CARGO_PKG_VERSION"),
                schema: 1,
            },
            payload: v,
        };
        serde_json::to_string(&envelope).expect("payload serializes")
    };
    let mut out = std::io::stdout().lock();
    if let Err(e) = writeln!(out, "{text}").and_then(|()| out.flush())
        && e.kind() != std::io::ErrorKind::BrokenPipe
    {
        let _ = writeln!(std::io::stderr(), "stewards: writing output: {e}");
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CliError {
    pub code: &'static str,
    pub message: String,
    pub fix: String,
    /// The answer's field errors from a failed `submit`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<crate::llm::FieldError>,
}

impl CliError {
    pub fn new(code: &'static str, message: impl Into<String>, fix: impl Into<String>) -> Self {
        CliError {
            code,
            message: message.into(),
            fix: fix.into(),
            errors: vec![],
        }
    }

    /// Prints the error on stdout; the caller exits 2.
    pub fn emit(&self, human: bool) {
        emit(&ErrorPayload { error: self }, human);
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CliError {}

#[derive(Serialize)]
struct ErrorPayload<'a> {
    error: &'a CliError,
}

impl Human for ErrorPayload<'_> {
    fn render(&self) -> String {
        let mut out = format!("error: {}\nfix: {}", self.error.message, self.error.fix);
        for e in &self.error.errors {
            out += &format!("\n- {}: {} (example: {})", e.field, e.problem, e.example);
        }
        out
    }
}

/// The stub every unbuilt command returns.
pub fn not_implemented(command: &str) -> anyhow::Result<std::process::ExitCode> {
    Err(CliError::new(
        "not_implemented",
        format!("`stewards {command}` is not implemented yet"),
        "use a released stewards that has this command",
    )
    .into())
}
