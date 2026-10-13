//! Command line for the document quality test suite.
//!
//! ```text
//! zacess-quality score [--fixtures DIR] [--out FILE]
//! zacess-quality check [--fixtures DIR] --baseline FILE [--baseline FILE ...]
//! ```
//!
//! `score` prints the scores and can write them to a file, which is how `baseline.json` is updated
//! after an improvement. `check` exits with an error when any score is lower than in any baseline.

use std::path::PathBuf;
use std::process::ExitCode;
use zacess_quality::{Report, compare, load_suite, markdown, run_suite};

const USAGE: &str = "usage: zacess-quality score [--fixtures DIR] [--out FILE]\n       \
                     zacess-quality check [--fixtures DIR] --baseline FILE [--baseline FILE ...]";

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let mut args = std::env::args().skip(1);
    let command = args.next().ok_or(USAGE)?;
    let mut fixtures = PathBuf::from("tests/fixtures");
    let mut out = None;
    let mut baselines = Vec::new();
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("{flag} needs a value\n{USAGE}"))?;
        match flag.as_str() {
            "--fixtures" => fixtures = PathBuf::from(value),
            "--out" => out = Some(PathBuf::from(value)),
            "--baseline" => baselines.push(PathBuf::from(value)),
            _ => return Err(format!("unknown option {flag}\n{USAGE}")),
        }
    }

    let suite = load_suite(&fixtures)?;
    let report = run_suite(&fixtures, &suite);
    let table = markdown(&report);
    println!("{table}");
    if let Ok(summary) = std::env::var("GITHUB_STEP_SUMMARY") {
        // the job summary is a convenience, so failing to write it never fails the check
        let _ = std::fs::write(summary, format!("## Document quality\n\n{table}"));
    }

    match command.as_str() {
        "score" => {
            if let Some(path) = out {
                let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
                std::fs::write(&path, json + "\n")
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                println!("scores written to {}", path.display());
            }
            Ok(ExitCode::SUCCESS)
        }
        "check" => {
            if baselines.is_empty() {
                return Err(format!("check needs at least one --baseline\n{USAGE}"));
            }
            let mut failed = false;
            for path in &baselines {
                let text = std::fs::read_to_string(path)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                let baseline: Report =
                    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
                let drops = compare(&baseline, &report);
                if drops.is_empty() {
                    println!("no score is lower than in {}", path.display());
                } else {
                    failed = true;
                    println!("scores lower than in {}:", path.display());
                    for drop in drops {
                        println!("  {drop}");
                    }
                }
            }
            Ok(if failed {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            })
        }
        _ => Err(USAGE.to_string()),
    }
}
