//! Cargo and gradle test output read into what ran.

#[cfg(test)]
mod tests;

/// What a check's output says about the tests, summed over every test binary of the run.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Ran {
    /// Tests that ran: passed, failed and ignored are not counted apart, filtered-out ones are not counted.
    pub tests: usize,
    /// Names of the tests marked `FAILED`, each once.
    pub failed: Vec<String>,
    /// The first compile error's line, where the build did not get as far as running tests.
    pub compile_error: Option<String>,
}

/// Reads one command's output: cargo's `test result:` lines and `FAILED` markers, gradle's
/// `Class > method FAILED`, and the first `error[E…]` / `error:` / `e: file.kt` line.
#[must_use]
pub fn parse(output: &str) -> Ran {
    let mut ran = Ran::default();
    for line in output.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("test result:") {
            ran.tests += summed(rest);
        } else if let Some(name) = failed_name(line)
            && !ran.failed.contains(&name)
        {
            ran.failed.push(name);
        } else if ran.compile_error.is_none() && is_compile_error(line) {
            ran.compile_error = Some(line.to_owned());
        }
    }
    ran
}

/// The tests a `test result: ok. 3 passed; 1 failed; 2 ignored; …` line ran.
fn summed(rest: &str) -> usize {
    rest.split(';')
        .filter_map(|part| {
            let words: Vec<&str> = part.split_whitespace().collect();
            words.windows(2).find_map(|pair| match (pair[0].parse::<usize>(), pair[1]) {
                (Ok(count), "passed" | "failed" | "ignored") => Some(count),
                _ => None,
            })
        })
        .sum()
}

/// The name a `test x ... FAILED`, `x --- FAILED` or gradle `Class > method FAILED` line gives.
fn failed_name(line: &str) -> Option<String> {
    let rest = line.strip_suffix(" FAILED")?;
    if let Some(name) = rest.strip_suffix(" ...") {
        return name.strip_prefix("test ").map(str::to_owned);
    }
    if let Some(name) = rest.strip_suffix(" ---") {
        return Some(name.to_owned());
    }
    let (class, method) = rest.split_once(" > ")?;
    Some(format!("{class}.{method}"))
}

/// The names of the test functions a unified diff adds: an added `#[test]`-style attribute
/// followed by an added `fn`.
#[must_use]
pub fn new_tests(diff: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut pending = false;
    for line in diff.lines() {
        let Some(added) = line.strip_prefix('+').filter(|_| !line.starts_with("+++")) else {
            // a context or removed line breaks the attribute from its function
            pending = false;
            continue;
        };
        let added = added.trim();
        if added.starts_with("#[test") || added.starts_with("#[tokio::test") {
            pending = true;
        } else if pending && added.starts_with("#[") {
            // another attribute between the test attribute and its function
        } else if pending {
            pending = false;
            let rest = added.strip_prefix("async ").unwrap_or(added);
            if let Some(name) = rest.strip_prefix("fn ").and_then(|r| r.split('(').next()) {
                names.push(name.trim().to_owned());
            }
        }
    }
    names
}

/// The tests among `names` that the output printed a `test path::name ... ` line for.
#[must_use]
pub fn ran_of(names: &[String], output: &str) -> Vec<String> {
    let printed: Vec<&str> = output
        .lines()
        .filter_map(|line| line.trim().strip_prefix("test "))
        .filter_map(|rest| rest.split_once(" ... ").map(|(name, _)| name))
        .collect();
    names.iter().filter(|name| printed.iter().any(|p| *p == name.as_str() || p.ends_with(&format!("::{name}")))).cloned().collect()
}

fn is_compile_error(line: &str) -> bool {
    if line.starts_with("error[") || line.starts_with("e: ") {
        return true;
    }
    // cargo's closing `error:` lines restate a failure already named
    line.starts_with("error:") && !["could not compile", "test failed", "target failed", "aborting"].iter().any(|s| line.contains(s))
}
