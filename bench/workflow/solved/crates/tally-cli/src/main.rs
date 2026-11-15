//! `tally-cli <file>` prints the report of a ledger file; `tally-cli -` reads the ledger from
//! standard input.

use std::io;
use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: tally-cli <file>|-");
        return ExitCode::from(2);
    };
    let read = if path == "-" {
        io::read_to_string(io::stdin())
    } else {
        std::fs::read_to_string(&path)
    };
    let text = match read {
        Ok(text) => text,
        Err(error) => {
            eprintln!("{path}: {error}");
            return ExitCode::from(2);
        }
    };
    match run(&text) {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

/// The report of a ledger's text, or the first line that is not an entry. Comments and blank
/// lines are skipped and still counted.
fn run(text: &str) -> Result<String, String> {
    let mut entries = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let start = line.trim_start();
        if start.is_empty() || start.starts_with('#') {
            continue;
        }
        match tally::parse_line(line) {
            Ok(entry) => entries.push(entry),
            Err(error) => return Err(format!("line {}: {error:?}", index + 1)),
        }
    }
    Ok(tally::render(&entries))
}

#[cfg(test)]
mod tests {
    use super::run;

    #[test]
    fn a_ledger_becomes_its_report() {
        assert_eq!(
            run("food 1\nfood 2.50\n"),
            Ok("food 3.50\ntotal 3.50\n".to_owned())
        );
    }

    #[test]
    fn a_bad_line_is_named_by_its_number() {
        assert_eq!(run("food 1\nfood\n"), Err("line 2: NoAmount".to_owned()));
    }

    #[test]
    fn skips_comments_and_blank_lines() {
        assert_eq!(
            run("# week\n\nfood 1\n  # aside\n   \nfood 2\n"),
            Ok("food 3.00\ntotal 3.00\n".to_owned())
        );
    }

    #[test]
    fn skips_lines_and_still_counts_them() {
        assert_eq!(
            run("food 1\n\n# note\nfood\n"),
            Err("line 4: NoAmount".to_owned())
        );
    }

    #[test]
    fn skips_no_line_with_a_hash_after_its_start() {
        assert!(run("food 1#2\n").is_err());
    }
}
