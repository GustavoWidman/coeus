use colored::Colorize;

use super::text::{pad, width};

/// columns aligned on their widest cell, with free-form notes under a row
pub struct Table {
    header: Vec<&'static str>,
    lines: Vec<Line>,
}

enum Line {
    Cells(Vec<String>),
    Note(String),
}

impl Table {
    const GAP: &'static str = "  ";
    const NOTE: &'static str = "  ↳ ";

    pub fn new(header: &[&'static str]) -> Self {
        Self {
            header: header.to_vec(),
            lines: Vec::new(),
        }
    }

    pub fn row(&mut self, cells: Vec<String>) {
        self.lines.push(Line::Cells(cells));
    }

    /// an indented line under the last row
    pub fn note(&mut self, text: impl Into<String>) {
        self.lines.push(Line::Note(text.into()));
    }

    pub fn lines(&self) -> Vec<String> {
        let widths = self.widths();
        let header: Vec<String> = self
            .header
            .iter()
            .map(|title| title.bold().dimmed().to_string())
            .collect();

        std::iter::once(Self::join(&header, &widths))
            .chain(self.lines.iter().map(|line| match line {
                Line::Cells(cells) => Self::join(cells, &widths),
                Line::Note(text) => format!("{}{text}", Self::NOTE.dimmed()),
            }))
            .collect()
    }

    fn widths(&self) -> Vec<usize> {
        let mut widths: Vec<usize> = self.header.iter().map(|title| title.len()).collect();
        for line in &self.lines {
            if let Line::Cells(cells) = line {
                for (at, cell) in cells.iter().enumerate() {
                    match widths.get_mut(at) {
                        Some(widest) => *widest = (*widest).max(width(cell)),
                        None => widths.push(width(cell)),
                    }
                }
            }
        }
        widths
    }

    fn join(cells: &[String], widths: &[usize]) -> String {
        let last = cells.len().saturating_sub(1);
        cells
            .iter()
            .zip(widths)
            .enumerate()
            .map(|(at, (cell, columns))| match at == last {
                true => cell.clone(),
                false => pad(cell, *columns),
            })
            .collect::<Vec<_>>()
            .join(Self::GAP)
            .trim_end()
            .to_owned()
    }
}
