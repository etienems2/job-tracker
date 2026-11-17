//! Command text, after the leading colon, parsed into a store write or a view change.

use std::path::PathBuf;

use jiff::civil::Date;
use thiserror::Error;

use crate::domain::{Filter, NameIssue, Op, Stage, StageParseError, normalize_name};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Parsed {
    Apply(Op),
    Filter(Filter),
    Export(PathBuf),
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum ParseError {
    #[error("enter a command")]
    Empty,
    #[error("unknown command '{0}'")]
    UnknownCommand(String),
    #[error("company is empty")]
    EmptyCompany,
    #[error("role is empty")]
    EmptyRole,
    #[error("company is longer than 200 characters")]
    CompanyTooLong,
    #[error("role is longer than 200 characters")]
    RoleTooLong,
    #[error("unknown stage '{0}'")]
    UnknownStage(String),
    #[error("ambiguous stage '{0}'")]
    AmbiguousStage(String),
    #[error("invalid date '{0}'")]
    InvalidDate(String),
    #[error("invalid id '{0}'")]
    InvalidId(String),
    #[error("{0}")]
    Usage(String),
}

pub(crate) fn parse(input: &str, today: Date) -> Result<Parsed, ParseError> {
    let Some((verb, rest)) = split_verb(input) else {
        return Err(ParseError::Empty);
    };
    match verb.to_ascii_lowercase().as_str() {
        "add" => parse_add(rest, today).map(Parsed::Apply),
        "stage" => parse_stage(rest, today).map(Parsed::Apply),
        "rename" => parse_rename(rest).map(Parsed::Apply),
        "delete" => parse_delete(rest).map(Parsed::Apply),
        "event" => parse_event(rest).map(Parsed::Apply),
        "delete-event" => parse_delete_event(rest).map(Parsed::Apply),
        "filter" => Ok(Parsed::Filter(parse_filter(rest))),
        "export" => parse_export(rest).map(Parsed::Export),
        _ => Err(ParseError::UnknownCommand(verb.to_string())),
    }
}

fn split_verb(input: &str) -> Option<(&str, &str)> {
    let input = input.trim().strip_prefix(':').unwrap_or(input).trim();
    if input.is_empty() {
        return None;
    }
    match input.split_once(char::is_whitespace) {
        Some((verb, rest)) => Some((verb, rest.trim())),
        None => Some((input, "")),
    }
}

fn parse_add(rest: &str, today: Date) -> Result<Op, ParseError> {
    let parts: Vec<&str> = rest.split('|').map(str::trim).collect();
    let (company, role, stage, on) = match parts.as_slice() {
        [company, role] => (*company, *role, Stage::Applied, today),
        [company, role, third] => {
            let (stage, on) = stage_or_date(third, today)?;
            (*company, *role, stage, on)
        }
        [company, role, stage, date] => {
            (*company, *role, require_stage(stage)?, require_date(date)?)
        }
        _ => {
            return Err(ParseError::Usage(
                "usage: add <company> | <role> [| <stage>] [| <date>]".into(),
            ));
        }
    };
    Ok(Op::Add {
        company: company_name(company)?,
        role: role_name(role)?,
        stage,
        on,
    })
}

fn stage_or_date(token: &str, today: Date) -> Result<(Stage, Date), ParseError> {
    if token.is_empty() {
        return Err(ParseError::Usage(
            "usage: add <company> | <role> [| <stage>] [| <date>]".into(),
        ));
    }
    match Stage::parse_prefix(token) {
        Ok(stage) => Ok((stage, today)),
        Err(StageParseError::Ambiguous) => Err(ParseError::AmbiguousStage(token.to_string())),
        Err(StageParseError::Unknown) => {
            if token.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
                Ok((Stage::Applied, require_date(token)?))
            } else {
                Err(ParseError::UnknownStage(token.to_string()))
            }
        }
    }
}

fn parse_stage(rest: &str, today: Date) -> Result<Op, ParseError> {
    let tokens: Vec<&str> = rest.split_whitespace().collect();
    let (id, stage, on) = match tokens.as_slice() {
        [id, stage] => (*id, *stage, today),
        [id, stage, date] => (*id, *stage, require_date(date)?),
        _ => {
            return Err(ParseError::Usage(
                "usage: stage <id> <stage> [<date>]".into(),
            ));
        }
    };
    Ok(Op::SetStage {
        application_id: parse_id(id)?,
        stage: require_stage(stage)?,
        on,
    })
}

fn parse_rename(rest: &str) -> Result<Op, ParseError> {
    let parts: Vec<&str> = rest.split('|').map(str::trim).collect();
    let [id, company, role] = parts.as_slice() else {
        return Err(ParseError::Usage(
            "usage: rename <id> | <company> | <role>".into(),
        ));
    };
    Ok(Op::Rename {
        application_id: parse_id(id)?,
        company: company_name(company)?,
        role: role_name(role)?,
    })
}

fn parse_delete(rest: &str) -> Result<Op, ParseError> {
    let tokens: Vec<&str> = rest.split_whitespace().collect();
    let [id] = tokens.as_slice() else {
        return Err(ParseError::Usage("usage: delete <id>".into()));
    };
    Ok(Op::Delete {
        application_id: parse_id(id)?,
    })
}

fn parse_event(rest: &str) -> Result<Op, ParseError> {
    let tokens: Vec<&str> = rest.split_whitespace().collect();
    let [id, stage, date] = tokens.as_slice() else {
        return Err(ParseError::Usage(
            "usage: event <event_id> <stage> <date>".into(),
        ));
    };
    Ok(Op::AmendEvent {
        event_id: parse_id(id)?,
        stage: require_stage(stage)?,
        on: require_date(date)?,
    })
}

fn parse_delete_event(rest: &str) -> Result<Op, ParseError> {
    let tokens: Vec<&str> = rest.split_whitespace().collect();
    let [id] = tokens.as_slice() else {
        return Err(ParseError::Usage("usage: delete-event <event_id>".into()));
    };
    Ok(Op::DeleteEvent {
        event_id: parse_id(id)?,
    })
}

fn parse_filter(rest: &str) -> Filter {
    let rest = rest.trim();
    if rest.is_empty() || rest.eq_ignore_ascii_case("active") {
        Filter::Active
    } else if rest.eq_ignore_ascii_case("closed") {
        Filter::Closed
    } else if rest.eq_ignore_ascii_case("all") {
        Filter::All
    } else if rest.eq_ignore_ascii_case("offer") {
        Filter::Offer
    } else {
        Filter::Text(rest.to_string())
    }
}

fn parse_export(rest: &str) -> Result<PathBuf, ParseError> {
    let path = rest.trim();
    if path.is_empty() {
        return Err(ParseError::Usage("usage: export <path>".into()));
    }
    Ok(PathBuf::from(path))
}

fn parse_id(text: &str) -> Result<i64, ParseError> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ParseError::InvalidId(text.to_string()));
    }
    let id: i64 = text
        .parse()
        .map_err(|_| ParseError::InvalidId(text.to_string()))?;
    if id < 1 {
        return Err(ParseError::InvalidId(text.to_string()));
    }
    Ok(id)
}

fn require_stage(token: &str) -> Result<Stage, ParseError> {
    Stage::parse_prefix(token).map_err(|error| match error {
        StageParseError::Unknown => ParseError::UnknownStage(token.to_string()),
        StageParseError::Ambiguous => ParseError::AmbiguousStage(token.to_string()),
    })
}

fn require_date(token: &str) -> Result<Date, ParseError> {
    token
        .parse()
        .map_err(|_| ParseError::InvalidDate(token.to_string()))
}

fn company_name(value: &str) -> Result<String, ParseError> {
    named("company", value)
}

fn role_name(value: &str) -> Result<String, ParseError> {
    named("role", value)
}

fn named(kind: &str, value: &str) -> Result<String, ParseError> {
    normalize_name(value).map_err(|issue| match (kind, issue) {
        ("company", NameIssue::Empty) => ParseError::EmptyCompany,
        ("role", NameIssue::Empty) => ParseError::EmptyRole,
        ("company", NameIssue::TooLong) => ParseError::CompanyTooLong,
        _ => ParseError::RoleTooLong,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Stage;

    fn today() -> Date {
        Date::new(2026, 11, 17).unwrap()
    }

    fn parse_ok(input: &str) -> Parsed {
        parse(input, today()).unwrap_or_else(|error| panic!("{input}: {error}"))
    }

    #[test]
    fn parses_the_documented_commands() {
        assert_eq!(
            parse_ok("add Acme | Backend engineer"),
            Parsed::Apply(Op::Add {
                company: "Acme".into(),
                role: "Backend engineer".into(),
                stage: Stage::Applied,
                on: today(),
            })
        );
        assert_eq!(
            parse_ok(":add Acme | Backend engineer | technical | 2026-11-01"),
            Parsed::Apply(Op::Add {
                company: "Acme".into(),
                role: "Backend engineer".into(),
                stage: Stage::TechnicalMeeting,
                on: Date::new(2026, 11, 1).unwrap(),
            })
        );
        assert_eq!(
            parse_ok("stage 3 first"),
            Parsed::Apply(Op::SetStage {
                application_id: 3,
                stage: Stage::FirstMeeting,
                on: today(),
            })
        );
        assert_eq!(
            parse_ok("stage 3 technical_meeting 2026-11-17"),
            Parsed::Apply(Op::SetStage {
                application_id: 3,
                stage: Stage::TechnicalMeeting,
                on: today(),
            })
        );
        assert_eq!(
            parse_ok("rename 3 | Acme Corp | Backend engineer"),
            Parsed::Apply(Op::Rename {
                application_id: 3,
                company: "Acme Corp".into(),
                role: "Backend engineer".into(),
            })
        );
        assert_eq!(
            parse_ok("delete 3"),
            Parsed::Apply(Op::Delete { application_id: 3 })
        );
        assert_eq!(
            parse_ok("event 4 first_meeting 2026-11-12"),
            Parsed::Apply(Op::AmendEvent {
                event_id: 4,
                stage: Stage::FirstMeeting,
                on: Date::new(2026, 11, 12).unwrap(),
            })
        );
        assert_eq!(
            parse_ok("delete-event 4"),
            Parsed::Apply(Op::DeleteEvent { event_id: 4 })
        );
        assert_eq!(parse_ok("filter active"), Parsed::Filter(Filter::Active));
        assert_eq!(parse_ok("filter"), Parsed::Filter(Filter::Active));
        assert_eq!(parse_ok("FILTER All"), Parsed::Filter(Filter::All));
        assert_eq!(
            parse_ok("filter acme"),
            Parsed::Filter(Filter::Text("acme".into()))
        );
        assert_eq!(
            parse_ok(r"export C:\Users\me\Desktop\applications.json"),
            Parsed::Export(PathBuf::from(r"C:\Users\me\Desktop\applications.json"))
        );
    }

    #[test]
    fn omitted_stage_and_date_use_applied_and_today() {
        match parse_ok("add   Acme Corp   |   Backend   ") {
            Parsed::Apply(Op::Add {
                company,
                role,
                stage,
                on,
            }) => {
                assert_eq!(company, "Acme Corp");
                assert_eq!(role, "Backend");
                assert_eq!(stage, Stage::Applied);
                assert_eq!(on, today());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_date_without_a_stage_on_add_stays_applied() {
        assert_eq!(
            parse_ok("add Acme | Backend | 2026-11-01"),
            Parsed::Apply(Op::Add {
                company: "Acme".into(),
                role: "Backend".into(),
                stage: Stage::Applied,
                on: Date::new(2026, 11, 1).unwrap(),
            })
        );
    }

    #[test]
    fn bad_prefixes_empty_names_and_bad_dates_do_not_parse() {
        assert_eq!(
            parse("stage 3 t", today()),
            Err(ParseError::AmbiguousStage("t".into()))
        );
        assert_eq!(
            parse("stage 3 nope", today()),
            Err(ParseError::UnknownStage("nope".into()))
        );
        assert_eq!(
            parse("add  | Backend", today()),
            Err(ParseError::EmptyCompany)
        );
        assert_eq!(parse("add Acme |   ", today()), Err(ParseError::EmptyRole));
        assert_eq!(
            parse("stage 3 applied 2026-02-31", today()),
            Err(ParseError::InvalidDate("2026-02-31".into()))
        );
        assert_eq!(
            parse("delete 0", today()),
            Err(ParseError::InvalidId("0".into()))
        );
        assert_eq!(
            parse("delete abc", today()),
            Err(ParseError::InvalidId("abc".into()))
        );
        assert_eq!(parse("   ", today()), Err(ParseError::Empty));
        assert_eq!(
            parse("nope", today()),
            Err(ParseError::UnknownCommand("nope".into()))
        );
        assert!(matches!(
            parse("add Acme", today()),
            Err(ParseError::Usage(_))
        ));
        assert!(matches!(
            parse("event 4 applied", today()),
            Err(ParseError::Usage(_))
        ));
        assert!(matches!(
            parse("export", today()),
            Err(ParseError::Usage(_))
        ));
        assert_eq!(
            parse(&format!("add {} | Role", "a".repeat(201)), today()),
            Err(ParseError::CompanyTooLong)
        );
    }

    #[test]
    fn filter_offer_is_the_keyword_and_other_words_are_text() {
        assert_eq!(parse_ok("filter offer"), Parsed::Filter(Filter::Offer));
        assert_eq!(parse_ok("filter closed"), Parsed::Filter(Filter::Closed));
        assert_eq!(
            parse_ok("filter offering"),
            Parsed::Filter(Filter::Text("offering".into()))
        );
        assert_eq!(
            parse_ok("filter north wind"),
            Parsed::Filter(Filter::Text("north wind".into()))
        );
    }

    #[test]
    fn future_dates_parse() {
        assert_eq!(
            parse_ok("stage 3 made 2027-01-15"),
            Parsed::Apply(Op::SetStage {
                application_id: 3,
                stage: Stage::OfferMade,
                on: Date::new(2027, 1, 15).unwrap(),
            })
        );
    }
}
