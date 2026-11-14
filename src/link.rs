//! Links in a terminal's text: those its programs mark as links, and addresses
//! written out in full. Only the kinds a browser or a mail program takes are
//! followed. A terminal may show another machine's output, so never a `file:`.
use std::{
    io,
    process::{Command, Stdio},
};

use crate::terminal::Frame;

const SCHEMES: [&str; 4] = ["https://", "http://", "ftp://", "mailto:"];
/// What may end a sentence an address is written in, without being part of it.
const TRAILING: &str = ".,;:!?'\"";
const BRACKETS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];

pub struct Link {
    pub target: String,
    /// The cells that show it: a row, and its columns from one up to another.
    pub spans: Vec<(usize, usize, usize)>,
}

/// The link shown at a column of a row, if it is one that may be followed.
pub fn at(frame: &Frame, x: usize, y: usize) -> Option<Link> {
    let marked = |row: &crate::terminal::Row| {
        let covers = |(start, end, _): &&(usize, usize, _)| (*start..*end).contains(&x);
        row.links.iter().find(covers).map(|link| link.2.clone())
    };
    // What a program marks as a link leads where it says, whatever the text reads.
    let Some(target) = marked(frame.lines.get(y)?) else {
        return written(frame, x, y);
    };
    let rows = frame.lines.iter().enumerate();
    let spans = rows.flat_map(|(y, row)| {
        let same = row.links.iter().filter(|link| link.2 == target);
        same.map(move |(start, end, _)| (y, *start, *end))
    });
    let spans = spans.collect();
    let target = target.to_string();
    followed(&target).then_some(Link { target, spans })
}

/// An address written out in the text, which may run over rows that wrap.
fn written(frame: &Frame, x: usize, y: usize) -> Option<Link> {
    let wraps = |row: usize| frame.lines.get(row).is_some_and(|row| row.wrapped);
    let first = (0..y)
        .rev()
        .take_while(|row| wraps(*row))
        .last()
        .unwrap_or(y);
    let last = (y..).find(|row| !wraps(*row))?;
    // Every character of the line, with the row and the column it is shown at.
    let mut line = Vec::new();
    for (row, cells) in frame.lines.iter().enumerate().take(last + 1).skip(first) {
        for (column, cell) in cells.cells.iter().enumerate() {
            if cell.spacer_tail || cell.spacer_head {
                continue;
            }
            let text = if cell.text.is_empty() {
                " "
            } else {
                &cell.text
            };
            line.extend(text.chars().map(|c| (c, row, column)));
        }
    }
    let here = line
        .iter()
        .position(|&(_, row, column)| (row, column) == (y, x))?;
    // Blanks end an address, and so do the lines a program draws around its text.
    let part = |&(c, ..): &(char, usize, usize)| {
        !c.is_whitespace() && !c.is_control() && !"<>\"`".contains(c) && !('─'..='▟').contains(&c)
    };
    let start = here - line[..here].iter().rev().take_while(|c| part(c)).count();
    let end = here + line[here..].iter().take_while(|c| part(c)).count();
    let word: String = line[start..end].iter().map(|c| c.0).collect();
    // It begins where its kind is named, which may be after a bracket or a quote.
    let begin = SCHEMES
        .iter()
        .filter_map(|scheme| word.find(scheme))
        .min()?;
    let mut target = &word[begin..];
    loop {
        let unopened = |(open, close): &(char, char)| {
            target.ends_with(*close)
                && target.matches(*close).count() > target.matches(*open).count()
        };
        let extra = target.ends_with(|c| TRAILING.contains(c)) || BRACKETS.iter().any(unopened);
        if !extra {
            break;
        }
        target = &target[..target.len() - 1];
    }
    let begin = start + word[..begin].chars().count();
    let shown = &line[begin..begin + target.chars().count()];
    if !(begin..begin + shown.len()).contains(&here) || !followed(target) {
        return None;
    }
    let mut spans: Vec<(usize, usize, usize)> = Vec::new();
    for &(_, row, column) in shown {
        match spans.last_mut() {
            Some(span) if span.0 == row => span.2 = column + 1,
            _ => spans.push((row, column, column + 1)),
        }
    }
    let target = target.to_owned();
    Some(Link { target, spans })
}

/// Whether a link is of a kind to follow, and is nothing but an address.
fn followed(target: &str) -> bool {
    let kind = SCHEMES.iter().find(|scheme| {
        let start = target.get(..scheme.len());
        start.is_some_and(|start| start.eq_ignore_ascii_case(scheme))
    });
    let plain = |c: char| !c.is_whitespace() && !c.is_control();
    kind.is_some_and(|kind| target.len() > kind.len()) && target.chars().all(plain)
}

/// Hand a link to whatever the desktop opens such links with.
pub fn open(target: &str) -> io::Result<()> {
    let mut opener = Command::new("xdg-open")
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    // It is waited for only so that it is not left behind when it ends.
    std::thread::spawn(move || opener.wait());
    Ok(())
}
