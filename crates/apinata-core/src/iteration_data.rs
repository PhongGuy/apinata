//! Data files for iterated runs: a CSV with a header row, or a JSON array of objects.
//! Each row becomes one iteration's variables, every value bound as a string.

use serde_json::Value;

/// The rows of a data file, each a list of (column, value) in column order.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct IterationData {
    rows: Vec<Vec<(String, String)>>,
}

/// Why a data file could not be read as iteration rows.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DataFileError {
    /// The file holds no data rows (a CSV header alone counts as none).
    #[error("the data file has no rows")]
    Empty,
    /// The text is not valid CSV or JSON; `line` is one-based.
    #[error("line {line}: {message}")]
    Malformed {
        /// The physical line where the problem record starts.
        line: usize,
        /// What is wrong with it.
        message: String,
    },
    /// The JSON parsed but is not an array whose every element is an object.
    #[error("the JSON data file must be an array of objects")]
    NotAnArrayOfObjects,
}

/// The iteration total a run's sends see as `iterationCount`: the data file's rows capped by
/// `count`, whichever one is given, or 1 for a single pass without either.
pub fn iteration_total(data: Option<&IterationData>, count: Option<usize>) -> usize {
    match (data, count) {
        (Some(data), Some(n)) => data.len().min(n),
        (Some(data), None) => data.len(),
        (None, Some(n)) => n,
        (None, None) => 1,
    }
}

impl IterationData {
    /// Parses RFC 4180 CSV whose first non-blank record names the columns.
    /// A leading BOM is ignored and entirely blank lines are skipped.
    pub fn from_csv(text: &str) -> Result<IterationData, DataFileError> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut records = csv_records(text)?.into_iter();
        let Some((_, header)) = records.next() else {
            return Err(DataFileError::Empty);
        };
        let rows = records
            .map(|(line, fields)| {
                if fields.len() != header.len() {
                    return Err(DataFileError::Malformed {
                        line,
                        message: format!(
                            "expected {} fields, found {}",
                            header.len(),
                            fields.len()
                        ),
                    });
                }
                Ok(header.iter().cloned().zip(fields).collect())
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::non_empty(rows)
    }

    /// Parses a JSON array of objects. Values bind as JavaScript would print them:
    /// numbers in JS form, `null` as empty, arrays and objects as compact JSON.
    pub fn from_json(text: &str) -> Result<IterationData, DataFileError> {
        let value: Value = serde_json::from_str(text).map_err(|e| {
            // serde_json appends the location to its message; the variant carries the line.
            let message = e.to_string();
            let message = message
                .rfind(" at line ")
                .map_or(message.as_str(), |at| &message[..at])
                .to_owned();
            DataFileError::Malformed {
                line: e.line(),
                message,
            }
        })?;
        let Value::Array(items) = value else {
            return Err(DataFileError::NotAnArrayOfObjects);
        };
        let rows = items
            .into_iter()
            .map(|item| match item {
                Value::Object(map) => Ok(map
                    .into_iter()
                    .map(|(key, value)| (key, js_string(value)))
                    .collect()),
                _ => Err(DataFileError::NotAnArrayOfObjects),
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::non_empty(rows)
    }

    /// The number of rows, one per iteration.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Always false for a parsed value, since an empty file is an error.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The row at `index` as (column, value) pairs, or `None` past the end.
    pub fn row(&self, index: usize) -> Option<&[(String, String)]> {
        self.rows.get(index).map(Vec::as_slice)
    }

    fn non_empty(rows: Vec<Vec<(String, String)>>) -> Result<IterationData, DataFileError> {
        if rows.is_empty() {
            return Err(DataFileError::Empty);
        }
        Ok(IterationData { rows })
    }
}

/// Renders a JSON value the way JavaScript's `String(value)` would for a data cell.
fn js_string(value: Value) -> String {
    match value {
        Value::String(s) => s,
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n
            .as_f64()
            .map_or_else(|| n.to_string(), crate::js::number_to_string),
        nested @ (Value::Array(_) | Value::Object(_)) => nested.to_string(),
    }
}

/// Splits CSV text into records, each with the physical line it starts on.
/// A quote opens a quoted field only at a field's start; inside one, `""` is a literal
/// quote and newlines are data. A record that is one empty unquoted field is a blank line.
fn csv_records(text: &str) -> Result<Vec<(usize, Vec<String>)>, DataFileError> {
    let mut records = Vec::new();
    let mut fields: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut in_quotes = false;
    let mut line = 1;
    let mut start_line = 1;
    let mut chars = text.chars().peekable();

    let mut finish_record =
        |fields: &mut Vec<String>, field: &mut String, quoted: bool, at: usize| {
            fields.push(std::mem::take(field));
            let record = std::mem::take(fields);
            let blank = !quoted && record.len() == 1 && record[0].is_empty();
            if !blank {
                records.push((at, record));
            }
        };

    while let Some(c) = chars.next() {
        if in_quotes {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => in_quotes = false,
                _ => {
                    if c == '\n' {
                        line += 1;
                    }
                    field.push(c);
                }
            }
            continue;
        }
        match c {
            '"' if field.is_empty() && !quoted => {
                in_quotes = true;
                quoted = true;
            }
            ',' => {
                fields.push(std::mem::take(&mut field));
                quoted = false;
            }
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                finish_record(&mut fields, &mut field, quoted, start_line);
                quoted = false;
                line += 1;
                start_line = line;
            }
            _ => field.push(c),
        }
    }
    if in_quotes {
        return Err(DataFileError::Malformed {
            line: start_line,
            message: "unterminated quoted field".to_owned(),
        });
    }
    if !field.is_empty() || !fields.is_empty() || quoted {
        finish_record(&mut fields, &mut field, quoted, start_line);
    }
    Ok(records)
}
