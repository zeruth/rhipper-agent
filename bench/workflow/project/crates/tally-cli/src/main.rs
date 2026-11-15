//! `tally-cli <file>` prints the report of a ledger file.

use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: tally-cli <file>");
        return ExitCode::from(2);
    };
    let text = match std::fs::read_to_string(&path) {
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

/// The report of a ledger's text, or the first line that is not an entry.
fn run(text: &str) -> Result<String, String> {
    let mut entries = Vec::new();
    for (index, line) in text.lines().enumerate() {
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
}
