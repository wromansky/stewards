use std::env;
use std::path::{Path, PathBuf};
use std::process::{exit, Command};

fn api_script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("cli has a parent directory")
        .join("api")
        .join("items.py")
}

fn run_api(args: Vec<String>) -> i32 {
    match Command::new("python3")
        .arg(api_script())
        .args(args)
        .status()
    {
        Ok(status) => status.code().unwrap_or(1),
        Err(err) => {
            eprintln!("items-cli: failed to run api: {err}");
            1
        }
    }
}

fn usage() {
    eprintln!("usage: items-cli <create <name>|list [--page N --page-size N]|get <id>>");
}

fn main() {
    let mut args = env::args().skip(1);
    let Some(sub) = args.next() else {
        usage();
        exit(2);
    };

    match sub.as_str() {
        "create" => {
            let name = args.next().unwrap();
            exit(run_api(vec!["create".to_string(), name]));
        }
        "list" => {
            let mut api_args = vec!["list".to_string()];
            api_args.extend(args);
            exit(run_api(api_args));
        }
        "get" => {
            let Some(id) = args.next() else {
                eprintln!("items-cli get: missing id");
                exit(2);
            };
            exit(run_api(vec!["get".to_string(), id]));
        }
        _ => {
            usage();
            exit(2);
        }
    }
}
