//! Asks in the terminal for arguments left out of a command. Without a terminal
//! (scripts, pipes) a missing argument is an error, so nothing ever hangs.
//! Prompts go to stderr, keeping stdout clean for the command's output.

use std::fmt::Display;
use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{Result, bail};

use crate::errors::UsageError;

pub fn is_interactive() -> bool {
    std::io::stdin().is_terminal()
}

/// The given value, or the answer to a prompt.
pub fn required<T>(label: &str, arg: &str, given: Option<T>) -> Result<T>
where
    T: FromStr,
    T::Err: Display,
{
    if let Some(value) = given {
        return Ok(value);
    }
    if !is_interactive() {
        return Err(UsageError(format!("missing {arg}")).into());
    }
    let answer = answer(
        &mut std::io::stdin().lock(),
        &mut std::io::stderr(),
        label,
        None,
    )?;
    answer.ok_or_else(|| UsageError(format!("missing {arg}")).into())
}

/// The given value; else, when `prompt` is set, an answer with a default.
pub fn optional<T>(label: &str, given: Option<T>, default: T, prompt: bool) -> Result<T>
where
    T: FromStr + Display,
    T::Err: Display,
{
    match given {
        Some(value) => Ok(value),
        None if !prompt => Ok(default),
        None => {
            let shown = default.to_string();
            let answer = answer(
                &mut std::io::stdin().lock(),
                &mut std::io::stderr(),
                label,
                Some(&shown),
            )?;
            Ok(answer.unwrap_or(default))
        }
    }
}

/// A path; a typed `~` is expanded, since no shell sees the answer.
pub fn path(label: &str, arg: &str, given: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = given {
        return Ok(path);
    }
    let typed: String = required(label, arg, None)?;
    Ok(expand_home(&typed, dirs::home_dir()))
}

/// Several values, typed separated by spaces or commas.
pub fn list(label: &str, arg: &str, given: Vec<String>) -> Result<Vec<String>> {
    if !given.is_empty() {
        return Ok(given);
    }
    let words: Words = required(label, arg, None)?;
    Ok(words.0)
}

struct Words(Vec<String>);

impl FromStr for Words {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let words: Vec<String> = s
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|w| !w.is_empty())
            .map(str::to_owned)
            .collect();
        if words.is_empty() {
            return Err("give at least one value");
        }
        Ok(Words(words))
    }
}

fn expand_home(typed: &str, home: Option<PathBuf>) -> PathBuf {
    match (typed.strip_prefix('~'), home) {
        (Some(""), Some(home)) => home,
        (Some(rest), Some(home)) if rest.starts_with('/') => {
            home.join(rest.trim_start_matches('/'))
        }
        _ => typed.into(),
    }
}

/// Asks until the answer parses. `None` means a blank answer accepting `default`.
fn answer<T>(
    input: &mut impl BufRead,
    out: &mut impl Write,
    label: &str,
    default: Option<&str>,
) -> Result<Option<T>>
where
    T: FromStr,
    T::Err: Display,
{
    loop {
        match default {
            Some(d) => write!(out, "{label} [{d}]: ")?,
            None => write!(out, "{label}: ")?,
        }
        out.flush()?;
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            bail!("no answer given");
        }
        let line = line.trim();
        if line.is_empty() {
            if default.is_some() {
                return Ok(None);
            }
            writeln!(out, "A value is required.")?;
            continue;
        }
        match line.parse() {
            Ok(value) => return Ok(Some(value)),
            Err(e) => writeln!(out, "Invalid value: {e}")?,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask<T>(typed: &str, default: Option<&str>) -> (Result<Option<T>>, String)
    where
        T: FromStr,
        T::Err: Display,
    {
        let mut out = Vec::new();
        let got = answer(&mut typed.as_bytes(), &mut out, "Minutes", default);
        (got, String::from_utf8(out).unwrap())
    }

    #[test]
    fn asks_again_after_a_blank_or_invalid_answer() {
        let (got, shown) = ask::<u32>("\nabc\n10\n", None);
        assert_eq!(got.unwrap(), Some(10));
        assert!(shown.contains("A value is required."));
        assert!(shown.contains("Invalid value"));
    }

    #[test]
    fn blank_answer_accepts_the_default() {
        let (got, shown) = ask::<u32>("\n", Some("30"));
        assert_eq!(got.unwrap(), None);
        assert_eq!(shown, "Minutes [30]: ");
    }

    #[test]
    fn end_of_input_stops_asking() {
        assert!(ask::<u32>("", None).0.is_err());
        assert!(ask::<u32>("abc\n", None).0.is_err());
    }

    #[test]
    fn lists_split_on_spaces_and_commas() {
        let words: Words = "sum, two  three,".parse().unwrap();
        assert_eq!(words.0, ["sum", "two", "three"]);
        assert!(" , ".parse::<Words>().is_err());
    }

    #[test]
    fn home_is_expanded_only_as_a_prefix() {
        let home = || Some(PathBuf::from("/home/u"));
        assert_eq!(expand_home("~", home()), PathBuf::from("/home/u"));
        assert_eq!(expand_home("~/a/b", home()), PathBuf::from("/home/u/a/b"));
        assert_eq!(expand_home("~other/a", home()), PathBuf::from("~other/a"));
        assert_eq!(expand_home("a/~/b", home()), PathBuf::from("a/~/b"));
    }
}
