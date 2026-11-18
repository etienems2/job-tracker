//! CSV rows become new applications. One row is one application.

use std::io::Read;

use jiff::civil::Date;
use thiserror::Error;

use crate::domain::{MAX_NAME_CHARS, NameIssue, Op, Stage, StageParseError, normalize_name};

#[derive(Debug, Error)]
pub(crate) enum ImportError {
    #[error("row {row}: {message}")]
    Row { row: usize, message: String },
    #[error("missing required column '{0}'")]
    MissingColumn(&'static str),
    #[error(transparent)]
    Csv(#[from] csv::Error),
}

/// Read applications from a CSV document. The first data row is row 1.
///
/// `company` and `role` are required. `stage` and `date` may be absent or
/// blank (`applied` and `today`). Any other column is ignored.
pub(crate) fn read_applications(reader: impl Read, today: Date) -> Result<Vec<Op>, ImportError> {
    let mut csv_reader = csv::ReaderBuilder::new().flexible(true).from_reader(reader);
    let headers = csv_reader.headers()?.clone();
    let company_col = column(&headers, "company")?;
    let role_col = column(&headers, "role")?;
    let stage_col = column_optional(&headers, "stage");
    let date_col = column_optional(&headers, "date");

    let mut applications = Vec::new();
    for (index, record) in csv_reader.records().enumerate() {
        let row = index + 1;
        let record = record?;
        applications.push(parse_row(
            row,
            &field(&record, company_col),
            &field(&record, role_col),
            &stage_col
                .map(|column| field(&record, column))
                .unwrap_or_default(),
            &date_col
                .map(|column| field(&record, column))
                .unwrap_or_default(),
            today,
        )?);
    }
    Ok(applications)
}

fn column(headers: &csv::StringRecord, name: &'static str) -> Result<usize, ImportError> {
    column_optional(headers, name).ok_or(ImportError::MissingColumn(name))
}

fn column_optional(headers: &csv::StringRecord, name: &str) -> Option<usize> {
    headers
        .iter()
        .position(|header| header.trim().eq_ignore_ascii_case(name))
}

fn field(record: &csv::StringRecord, index: usize) -> String {
    record.get(index).unwrap_or("").trim().to_string()
}

fn parse_row(
    row: usize,
    company: &str,
    role: &str,
    stage: &str,
    date: &str,
    today: Date,
) -> Result<Op, ImportError> {
    Ok(Op::Add {
        company: name(row, "company", company)?,
        role: name(row, "role", role)?,
        stage: parse_stage(row, stage)?,
        on: parse_date(row, date, today)?,
    })
}

fn name(row: usize, kind: &str, value: &str) -> Result<String, ImportError> {
    normalize_name(value).map_err(|issue| ImportError::Row {
        row,
        message: match (kind, issue) {
            ("company", NameIssue::Empty) => "company is empty".to_string(),
            ("role", NameIssue::Empty) => "role is empty".to_string(),
            ("company", NameIssue::TooLong) => {
                format!("company is longer than {MAX_NAME_CHARS} characters")
            }
            _ => format!("role is longer than {MAX_NAME_CHARS} characters"),
        },
    })
}

fn parse_stage(row: usize, token: &str) -> Result<Stage, ImportError> {
    if token.is_empty() {
        return Ok(Stage::Applied);
    }
    Stage::parse_prefix(token).map_err(|error| ImportError::Row {
        row,
        message: match error {
            StageParseError::Unknown => format!("unknown stage '{token}'"),
            StageParseError::Ambiguous => format!("ambiguous stage '{token}'"),
        },
    })
}

fn parse_date(row: usize, token: &str, today: Date) -> Result<Date, ImportError> {
    if token.is_empty() {
        return Ok(today);
    }
    token.parse().map_err(|_| ImportError::Row {
        row,
        message: format!("invalid date '{token}'"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> Date {
        Date::new(2026, 11, 17).unwrap()
    }

    fn parse(csv: &str) -> Result<Vec<Op>, ImportError> {
        read_applications(csv.as_bytes(), today())
    }

    #[test]
    fn defaults_prefix_and_extra_columns() {
        let ops = parse(
            "\
company,role,stage,date,notes
Acme,Backend engineer,,,keep
Northwind,iOS,technical,2026-11-01,ignore
",
        )
        .unwrap();
        assert_eq!(
            ops,
            vec![
                Op::Add {
                    company: "Acme".into(),
                    role: "Backend engineer".into(),
                    stage: Stage::Applied,
                    on: today(),
                },
                Op::Add {
                    company: "Northwind".into(),
                    role: "iOS".into(),
                    stage: Stage::TechnicalMeeting,
                    on: Date::new(2026, 11, 1).unwrap(),
                },
            ]
        );
    }

    #[test]
    fn a_quoted_comma_stays_in_the_company() {
        let ops = parse(
            "\
Company,Role
\"Acme, Inc\",Backend engineer
",
        )
        .unwrap();
        assert_eq!(
            ops,
            vec![Op::Add {
                company: "Acme, Inc".into(),
                role: "Backend engineer".into(),
                stage: Stage::Applied,
                on: today(),
            }]
        );
    }

    #[test]
    fn row_errors_name_the_data_row() {
        let empty = parse("company,role\n,Backend\n").unwrap_err();
        assert_eq!(empty.to_string(), "row 1: company is empty");

        let ambiguous = parse("company,role,stage\nAcme,Backend,t\n").unwrap_err();
        assert_eq!(ambiguous.to_string(), "row 1: ambiguous stage 't'");

        let date = parse("company,role,date\nAcme,Backend,2026-02-31\n").unwrap_err();
        assert_eq!(date.to_string(), "row 1: invalid date '2026-02-31'");

        let header = parse("role,stage\nBackend,applied\n").unwrap_err();
        assert_eq!(header.to_string(), "missing required column 'company'");
    }
}
