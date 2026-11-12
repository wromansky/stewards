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

/// Prints `v` as enveloped JSON, or its markdown rendering with --human.
pub fn emit<T: Serialize + Human>(v: &T, human: bool) {
    if human {
        println!("{}", v.render());
    } else {
        let envelope = Envelope {
            stewards: Meta {
                version: env!("CARGO_PKG_VERSION"),
                schema: 1,
            },
            payload: v,
        };
        println!(
            "{}",
            serde_json::to_string(&envelope).expect("payload serializes")
        );
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CliError {
    pub code: &'static str,
    pub message: String,
    pub fix: String,
}

impl CliError {
    pub fn new(code: &'static str, message: impl Into<String>, fix: impl Into<String>) -> Self {
        CliError {
            code,
            message: message.into(),
            fix: fix.into(),
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
        format!("error: {}\nfix: {}", self.error.message, self.error.fix)
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
