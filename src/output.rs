//! Command output: results on standard output, `clai: ` lines on standard
//! error, and tables.

use std::io::{self, Write};

use crate::error::{Error, Result};

/// Writes `text` to standard output. A closed pipe (`clai models | head -1`)
/// is not an error: the reader got what it wanted.
pub fn write_stdout(text: &str) -> Result<()> {
    let mut stdout = io::stdout().lock();
    match stdout
        .write_all(text.as_bytes())
        .and_then(|()| stdout.flush())
    {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(source) => Err(Error::Io {
            context: "standard output".to_owned(),
            source,
        }),
    }
}

/// Writes `clai: {text}` on standard error. A failure is ignored: there is
/// nowhere left to report it.
pub fn note(text: &str) {
    let mut stderr = io::stderr().lock();
    let _ = writeln!(stderr, "clai: {text}");
}

/// How a column of [`columns`] is padded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

/// The rows as lines: each column but the last padded to its widest cell as
/// `aligns` says, two spaces between columns, the last column as it is, and
/// each line trimmed at the end.
pub fn columns(rows: &[Vec<String>], aligns: &[Align]) -> Vec<String> {
    let widths: Vec<usize> = (0..aligns.len())
        .map(|i| {
            rows.iter()
                .filter_map(|r| r.get(i))
                .map(|c| c.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    rows.iter()
        .map(|row| {
            let mut line = String::new();
            for (i, cell) in row.iter().enumerate() {
                if i > 0 {
                    line.push_str("  ");
                }
                match (aligns.get(i), widths.get(i)) {
                    (Some(Align::Left), Some(&w)) => line.push_str(&format!("{cell:<w$}")),
                    (Some(Align::Right), Some(&w)) => line.push_str(&format!("{cell:>w$}")),
                    _ => line.push_str(cell),
                }
            }
            line.trim_end().to_owned()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(cells: &[&str]) -> Vec<String> {
        cells.iter().map(|c| (*c).to_owned()).collect()
    }

    #[test]
    fn columns_are_padded_and_the_last_is_free() {
        let rows = [row(&["a", "1", "x"]), row(&["long", "200", ""])];
        assert_eq!(
            columns(&rows, &[Align::Left, Align::Right]),
            ["a       1  x", "long  200"]
        );
    }

    #[test]
    fn widths_count_characters() {
        let rows = [row(&["wörld", "a"]), row(&["ab", "b"])];
        assert_eq!(columns(&rows, &[Align::Left]), ["wörld  a", "ab     b"]);
    }
}
