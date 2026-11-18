use std::cmp::Ordering;

use jiff::civil::Date;

pub(crate) const MAX_NAME_CHARS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NameIssue {
    Empty,
    TooLong,
}

pub(crate) fn normalize_name(value: &str) -> Result<String, NameIssue> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(NameIssue::Empty);
    }
    if trimmed.chars().count() > MAX_NAME_CHARS {
        return Err(NameIssue::TooLong);
    }
    Ok(trimmed.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
    Applied,
    FirstMeeting,
    SecondMeeting,
    ThirdMeeting,
    TechnicalMeeting,
    Refused,
    OfferMade,
    OfferAccepted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StageParseError {
    Unknown,
    Ambiguous,
}

impl Stage {
    pub(crate) const ALL: [Stage; 8] = [
        Stage::Applied,
        Stage::FirstMeeting,
        Stage::SecondMeeting,
        Stage::ThirdMeeting,
        Stage::TechnicalMeeting,
        Stage::Refused,
        Stage::OfferMade,
        Stage::OfferAccepted,
    ];

    pub(crate) fn token(self) -> &'static str {
        match self {
            Stage::Applied => "applied",
            Stage::FirstMeeting => "first_meeting",
            Stage::SecondMeeting => "second_meeting",
            Stage::ThirdMeeting => "third_meeting",
            Stage::TechnicalMeeting => "technical_meeting",
            Stage::Refused => "refused",
            Stage::OfferMade => "offer_made",
            Stage::OfferAccepted => "offer_accepted",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Stage::Applied => "Applied",
            Stage::FirstMeeting => "First meeting",
            Stage::SecondMeeting => "Second meeting",
            Stage::ThirdMeeting => "Third meeting",
            Stage::TechnicalMeeting => "Technical meeting",
            Stage::Refused => "Refused",
            Stage::OfferMade => "Offer made",
            Stage::OfferAccepted => "Offer accepted",
        }
    }

    pub(crate) fn pipeline_rank(self) -> u8 {
        match self {
            Stage::Applied => 0,
            Stage::FirstMeeting => 1,
            Stage::SecondMeeting => 2,
            Stage::ThirdMeeting => 3,
            Stage::TechnicalMeeting => 4,
            Stage::OfferMade => 5,
            Stage::OfferAccepted => 6,
            Stage::Refused => 7,
        }
    }

    pub(crate) fn is_closed(self) -> bool {
        matches!(self, Stage::Refused | Stage::OfferAccepted)
    }

    pub(crate) fn is_offer(self) -> bool {
        matches!(self, Stage::OfferMade | Stage::OfferAccepted)
    }

    pub(crate) fn from_token(token: &str) -> Option<Stage> {
        Stage::ALL.into_iter().find(|stage| stage.token() == token)
    }

    pub(crate) fn parse_prefix(input: &str) -> Result<Stage, StageParseError> {
        let input = input.trim().to_ascii_lowercase();
        if input.is_empty() {
            return Err(StageParseError::Unknown);
        }
        let mut matches = Stage::ALL
            .into_iter()
            .filter(|stage| stage_matches(*stage, &input));
        match (matches.next(), matches.next()) {
            (Some(stage), None) => Ok(stage),
            (None, _) => Err(StageParseError::Unknown),
            (Some(_), Some(_)) => Err(StageParseError::Ambiguous),
        }
    }
}

fn stage_matches(stage: Stage, input: &str) -> bool {
    let token = stage.token();
    token.starts_with(input) || token.split('_').any(|part| part.starts_with(input))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StageEvent {
    pub(crate) id: i64,
    pub(crate) stage: Stage,
    pub(crate) occurred_on: Date,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Application {
    pub(crate) id: i64,
    pub(crate) company: String,
    pub(crate) role: String,
    pub(crate) events: Vec<StageEvent>,
}

impl Application {
    pub(crate) fn new(id: i64, company: String, role: String, events: Vec<StageEvent>) -> Self {
        let mut app = Self {
            id,
            company,
            role,
            events,
        };
        app.sort_events();
        app
    }

    pub(crate) fn sort_events(&mut self) {
        self.events.sort_by(|left, right| {
            left.occurred_on
                .cmp(&right.occurred_on)
                .then(left.id.cmp(&right.id))
        });
    }

    pub(crate) fn current(&self) -> Option<&StageEvent> {
        self.events.last()
    }

    pub(crate) fn is_active(&self) -> bool {
        self.current().is_some_and(|event| !event.stage.is_closed())
    }

    pub(crate) fn matches(&self, filter: &Filter) -> bool {
        match filter {
            Filter::All => true,
            Filter::Active => self.is_active(),
            Filter::Closed => self.current().is_some_and(|event| event.stage.is_closed()),
            Filter::Offer => self.current().is_some_and(|event| event.stage.is_offer()),
            Filter::Text(needle) => {
                let needle = needle.to_lowercase();
                self.company.to_lowercase().contains(&needle)
                    || self.role.to_lowercase().contains(&needle)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub(crate) applications: Vec<Application>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Op {
    Add {
        company: String,
        role: String,
        stage: Stage,
        on: Date,
    },
    SetStage {
        application_id: i64,
        stage: Stage,
        on: Date,
    },
    Rename {
        application_id: i64,
        company: String,
        role: String,
    },
    Delete {
        application_id: i64,
    },
    AmendEvent {
        event_id: i64,
        stage: Stage,
        on: Date,
    },
    DeleteEvent {
        event_id: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Filter {
    Active,
    Closed,
    All,
    Offer,
    Text(String),
}

impl Filter {
    pub(crate) fn label(&self) -> String {
        match self {
            Filter::Active => "active".to_string(),
            Filter::Closed => "closed".to_string(),
            Filter::All => "all".to_string(),
            Filter::Offer => "offer".to_string(),
            Filter::Text(text) => text.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sort {
    Recent,
    Company,
    Pipeline,
}

impl Sort {
    pub(crate) fn cycle(self) -> Self {
        match self {
            Sort::Recent => Sort::Company,
            Sort::Company => Sort::Pipeline,
            Sort::Pipeline => Sort::Recent,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Sort::Recent => "recent",
            Sort::Company => "company",
            Sort::Pipeline => "pipeline",
        }
    }
}

pub(crate) fn cmp_applications(left: &Application, right: &Application, sort: Sort) -> Ordering {
    match sort {
        Sort::Recent => recent_key(right).cmp(&recent_key(left)),
        Sort::Company => company_key(left).cmp(&company_key(right)),
        Sort::Pipeline => pipeline_key(left).cmp(&pipeline_key(right)),
    }
}

fn recent_key(app: &Application) -> (Date, i64, i64) {
    match app.current() {
        Some(event) => (event.occurred_on, event.id, app.id),
        None => (Date::MIN, 0, app.id),
    }
}

fn company_key(app: &Application) -> (String, String, i64) {
    (app.company.to_lowercase(), app.role.to_lowercase(), app.id)
}

fn pipeline_key(app: &Application) -> (u8, String, i64) {
    let rank = app
        .current()
        .map(|event| event.stage.pipeline_rank())
        .unwrap_or(u8::MAX);
    (rank, app.company.to_lowercase(), app.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(year: i16, month: i8, day: i8) -> Date {
        Date::new(year, month, day).unwrap()
    }

    fn event(id: i64, stage: Stage, on: Date) -> StageEvent {
        StageEvent {
            id,
            stage,
            occurred_on: on,
        }
    }

    fn app(events: Vec<StageEvent>) -> Application {
        Application::new(1, "Acme".into(), "Backend".into(), events)
    }

    #[test]
    fn current_event_is_latest_date_then_id() {
        let same_day = date(2026, 11, 14);
        let application = app(vec![
            event(2, Stage::FirstMeeting, same_day),
            event(1, Stage::Applied, same_day),
        ]);
        assert_eq!(application.events[0].id, 1);
        assert_eq!(application.current().unwrap().id, 2);
        assert_eq!(application.current().unwrap().stage, Stage::FirstMeeting);

        let later_date_lower_id = app(vec![
            event(9, Stage::Applied, date(2026, 11, 2)),
            event(1, Stage::Refused, date(2026, 11, 3)),
        ]);
        assert_eq!(later_date_lower_id.current().unwrap().id, 1);
        assert_eq!(later_date_lower_id.current().unwrap().stage, Stage::Refused);
    }

    #[test]
    fn closed_and_offer_follow_the_current_stage() {
        let active = app(vec![event(1, Stage::OfferMade, date(2026, 11, 1))]);
        assert!(active.is_active());
        assert!(active.matches(&Filter::Offer));
        assert!(active.matches(&Filter::Active));
        assert!(!active.matches(&Filter::Closed));

        let accepted = app(vec![event(1, Stage::OfferAccepted, date(2026, 11, 2))]);
        assert!(!accepted.is_active());
        assert!(accepted.matches(&Filter::Closed));
        assert!(accepted.matches(&Filter::Offer));

        let refused = app(vec![event(1, Stage::Refused, date(2026, 11, 2))]);
        assert!(!refused.is_active());
        assert!(refused.matches(&Filter::Closed));
        assert!(!refused.matches(&Filter::Offer));
    }

    #[test]
    fn text_filter_matches_company_or_role_ignoring_case() {
        let application = app(vec![event(1, Stage::Applied, date(2026, 11, 1))]);
        assert!(application.matches(&Filter::Text("acme".into())));
        assert!(application.matches(&Filter::Text("BACK".into())));
        assert!(!application.matches(&Filter::Text("northwind".into())));
    }

    #[test]
    fn stage_prefixes_match_the_documented_examples() {
        assert_eq!(Stage::parse_prefix("applied").unwrap(), Stage::Applied);
        assert_eq!(Stage::parse_prefix("FIRST").unwrap(), Stage::FirstMeeting);
        assert_eq!(
            Stage::parse_prefix("tech").unwrap(),
            Stage::TechnicalMeeting
        );
        assert_eq!(Stage::parse_prefix("made").unwrap(), Stage::OfferMade);
        assert_eq!(
            Stage::parse_prefix("accepted").unwrap(),
            Stage::OfferAccepted
        );
        assert_eq!(Stage::parse_prefix("f").unwrap(), Stage::FirstMeeting);
        assert_eq!(Stage::parse_prefix("s").unwrap(), Stage::SecondMeeting);
        assert_eq!(Stage::parse_prefix("r").unwrap(), Stage::Refused);
        assert_eq!(Stage::parse_prefix("offer_m").unwrap(), Stage::OfferMade);

        for stage in Stage::ALL {
            assert_eq!(Stage::from_token(stage.token()), Some(stage));
            assert_eq!(Stage::parse_prefix(stage.token()).unwrap(), stage);
        }

        assert_eq!(Stage::parse_prefix("t"), Err(StageParseError::Ambiguous));
        assert_eq!(Stage::parse_prefix("a"), Err(StageParseError::Ambiguous));
        assert_eq!(Stage::parse_prefix("o"), Err(StageParseError::Ambiguous));
        assert_eq!(Stage::parse_prefix("m"), Err(StageParseError::Ambiguous));
        assert_eq!(
            Stage::parse_prefix("offer"),
            Err(StageParseError::Ambiguous)
        );
        assert_eq!(
            Stage::parse_prefix("meeting"),
            Err(StageParseError::Ambiguous)
        );
        assert_eq!(Stage::parse_prefix("c"), Err(StageParseError::Unknown));
        assert_eq!(Stage::parse_prefix("nope"), Err(StageParseError::Unknown));
        assert_eq!(Stage::parse_prefix("  "), Err(StageParseError::Unknown));
    }

    #[test]
    fn names_are_trimmed_and_limited_to_200_characters() {
        assert_eq!(normalize_name("  Acme  ").unwrap(), "Acme");
        assert_eq!(normalize_name("   "), Err(NameIssue::Empty));
        assert_eq!(normalize_name(""), Err(NameIssue::Empty));
        let max = "a".repeat(MAX_NAME_CHARS);
        assert_eq!(normalize_name(&max).unwrap(), max);
        assert_eq!(
            normalize_name(&"a".repeat(MAX_NAME_CHARS + 1)),
            Err(NameIssue::TooLong)
        );
    }

    #[test]
    fn recent_sort_is_newest_stage_date_then_id() {
        let older = Application::new(
            2,
            "Acme".into(),
            "Role".into(),
            vec![event(1, Stage::Applied, date(2026, 11, 1))],
        );
        let newer = Application::new(
            1,
            "Northwind".into(),
            "Role".into(),
            vec![event(2, Stage::Applied, date(2026, 11, 20))],
        );
        assert_eq!(
            cmp_applications(&older, &newer, Sort::Recent),
            Ordering::Greater
        );

        let same_day_low = Application::new(
            3,
            "A".into(),
            "Role".into(),
            vec![event(1, Stage::Applied, date(2026, 11, 20))],
        );
        let same_day_high = Application::new(
            4,
            "B".into(),
            "Role".into(),
            vec![event(8, Stage::FirstMeeting, date(2026, 11, 20))],
        );
        assert_eq!(
            cmp_applications(&same_day_low, &same_day_high, Sort::Recent),
            Ordering::Greater
        );
    }
}
