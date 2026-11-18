//! Terminal lifecycle and the event loop. This is the only place that touches
//! both the model and the database.

use std::path::{Path, PathBuf};

use jiff::civil::Date;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::domain::{Op, Snapshot};
use crate::message::Message;
use crate::model::{Mode, Model};
use crate::store::{SqliteStore, Store};
use crate::update::{Effect, update};

pub(crate) fn run(
    mut terminal: ratatui::DefaultTerminal,
    mut store: SqliteStore,
    db_path: PathBuf,
) -> color_eyre::Result<()> {
    let snapshot = store.load()?;
    let mut model = Model::new(snapshot, db_path);
    while model.running {
        terminal.draw(|frame| crate::view::draw(frame, &mut model))?;
        let event = event::read()?;
        let (width, _) = ratatui::crossterm::terminal::size()?;
        let today = today();
        if let Some(message) = map_event(event, model.mode, today, width) {
            let effect = update(&mut model, message);
            dispatch(&mut model, &mut store, effect)?;
        }
    }
    Ok(())
}

fn today() -> Date {
    jiff::ZonedDateTime::now().date()
}

fn map_event(event: Event, mode: Mode, today: Date, width: u16) -> Option<Message> {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => map_key(key, mode, today, width),
        Event::Paste(text) if mode == Mode::Command => Some(Message::CommandInsert(text)),
        _ => None,
    }
}

fn map_key(key: KeyEvent, mode: Mode, today: Date, width: u16) -> Option<Message> {
    if is_ctrl_c(&key) {
        return Some(Message::Quit);
    }
    if key.modifiers.contains(KeyModifiers::CONTROL)
        || key.modifiers.contains(KeyModifiers::ALT)
        || key.modifiers.contains(KeyModifiers::SUPER)
    {
        return None;
    }
    match mode {
        Mode::Normal => map_normal(key.code, width),
        Mode::Command => map_command(key.code, today),
        Mode::StagePick => map_pick(key.code, today),
        Mode::Help => map_help(key.code),
    }
}

fn is_ctrl_c(key: &KeyEvent) -> bool {
    let control_c = matches!(key.code, KeyCode::Char('c' | 'C'))
        && key.modifiers.contains(KeyModifiers::CONTROL);
    control_c || matches!(key.code, KeyCode::Char('\u{3}'))
}

fn map_normal(code: KeyCode, width: u16) -> Option<Message> {
    Some(match code {
        KeyCode::Char('q') => Message::Quit,
        KeyCode::Char('j') | KeyCode::Down => Message::Move(1),
        KeyCode::Char('k') | KeyCode::Up => Message::Move(-1),
        KeyCode::Enter => Message::Enter { width },
        KeyCode::Esc => Message::Escape,
        KeyCode::Char('n') => Message::BeginCommand("add ".into()),
        KeyCode::Char('s') => Message::OpenStagePick,
        KeyCode::Char('d') => Message::PromptDelete,
        KeyCode::Char('/') => Message::BeginCommand("filter ".into()),
        KeyCode::Char('o') => Message::CycleSort,
        KeyCode::Char(':') => Message::BeginCommand(String::new()),
        KeyCode::Char('?') => Message::ShowHelp,
        _ => return None,
    })
}

fn map_command(code: KeyCode, today: Date) -> Option<Message> {
    Some(match code {
        KeyCode::Enter => Message::SubmitCommand { today },
        KeyCode::Esc => Message::Escape,
        KeyCode::Backspace => Message::CommandBackspace,
        KeyCode::Delete => Message::CommandDelete,
        KeyCode::Left => Message::CommandLeft,
        KeyCode::Right => Message::CommandRight,
        KeyCode::Home => Message::CommandHome,
        KeyCode::End => Message::CommandEnd,
        KeyCode::Char(ch) => Message::CommandInsert(ch.to_string()),
        _ => return None,
    })
}

fn map_pick(code: KeyCode, today: Date) -> Option<Message> {
    match code {
        KeyCode::Esc => Some(Message::Escape),
        KeyCode::Char(ch) => match ch.to_ascii_lowercase() {
            'a' => Some(Message::PickStage {
                stage: crate::domain::Stage::Applied,
                today,
            }),
            'f' => Some(Message::PickStage {
                stage: crate::domain::Stage::FirstMeeting,
                today,
            }),
            's' => Some(Message::PickStage {
                stage: crate::domain::Stage::SecondMeeting,
                today,
            }),
            't' => Some(Message::PickStage {
                stage: crate::domain::Stage::ThirdMeeting,
                today,
            }),
            'c' => Some(Message::PickStage {
                stage: crate::domain::Stage::TechnicalMeeting,
                today,
            }),
            'm' => Some(Message::PickStage {
                stage: crate::domain::Stage::OfferMade,
                today,
            }),
            'o' => Some(Message::PickStage {
                stage: crate::domain::Stage::OfferAccepted,
                today,
            }),
            'r' => Some(Message::PickStage {
                stage: crate::domain::Stage::Refused,
                today,
            }),
            _ => Some(Message::Failed(format!("unknown stage key '{ch}'"))),
        },
        _ => None,
    }
}

fn map_help(code: KeyCode) -> Option<Message> {
    match code {
        KeyCode::Esc => Some(Message::Escape),
        KeyCode::Char('q') => Some(Message::Quit),
        _ => None,
    }
}

fn dispatch(
    model: &mut Model,
    store: &mut SqliteStore,
    effect: Effect,
) -> Result<(), crate::store::StoreError> {
    match effect {
        Effect::None | Effect::Quit => Ok(()),
        Effect::Apply(op) => {
            let previous = match &op {
                Op::DeleteEvent { event_id } => parent_application(model, *event_id),
                _ => None,
            };
            match store.apply(op.clone()) {
                Ok(snapshot) => {
                    let (status, focus) = outcome(&op, &snapshot, &model.filter, previous);
                    update(
                        model,
                        Message::Loaded {
                            snapshot,
                            status,
                            focus,
                        },
                    );
                    Ok(())
                }
                Err(error) => {
                    update(model, Message::Failed(error.to_string()));
                    Ok(())
                }
            }
        }
        Effect::Export(path) => {
            match write_export(store, &path) {
                Ok(message) => {
                    update(model, Message::Status(message));
                }
                Err(message) => {
                    update(model, Message::Failed(message));
                }
            }
            Ok(())
        }
    }
}

fn parent_application(model: &Model, event_id: i64) -> Option<i64> {
    model
        .apps
        .iter()
        .find(|app| app.events.iter().any(|event| event.id == event_id))
        .map(|app| app.id)
}

fn outcome(
    op: &Op,
    snapshot: &Snapshot,
    filter: &crate::domain::Filter,
    previous_app: Option<i64>,
) -> (String, Option<i64>) {
    let focus = match op {
        Op::Add { .. } => snapshot.applications.iter().map(|app| app.id).max(),
        Op::SetStage { application_id, .. } | Op::Rename { application_id, .. } => {
            Some(*application_id)
        }
        Op::Delete { .. } => None,
        Op::AmendEvent { event_id, .. } => application_id_for_event(snapshot, *event_id),
        Op::DeleteEvent { .. } => previous_app,
    };
    let mut status = match op {
        Op::Add { .. } => format!("wrote application {}", focus.unwrap_or(0)),
        Op::SetStage { application_id, .. } => format!("wrote application {application_id}"),
        Op::Rename { application_id, .. } => format!("renamed application {application_id}"),
        Op::Delete { application_id } => format!("deleted application {application_id}"),
        Op::AmendEvent { event_id, .. } => format!("amended event {event_id}"),
        Op::DeleteEvent { event_id } => format!("deleted event {event_id}"),
    };
    if let Some(id) = focus {
        let visible = snapshot
            .applications
            .iter()
            .any(|app| app.id == id && app.matches(filter));
        if !visible {
            status.push_str(" (hidden by the current filter)");
        }
    }
    (status, focus)
}

fn application_id_for_event(snapshot: &Snapshot, event_id: i64) -> Option<i64> {
    snapshot
        .applications
        .iter()
        .find(|app| app.events.iter().any(|event| event.id == event_id))
        .map(|app| app.id)
}

fn write_export(store: &SqliteStore, path: &Path) -> Result<String, String> {
    let mut json = store.export_json().map_err(|error| error.to_string())?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|error| format!("export: {error}"))?;
    }
    if !json.ends_with('\n') {
        json.push('\n');
    }
    std::fs::write(path, json).map_err(|error| format!("export: {error}"))?;
    Ok(format!("exported to {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Stage;
    use crate::update::Effect;
    use jiff::civil::Date;

    fn day(day: i8) -> Date {
        Date::new(2026, 11, day).unwrap()
    }

    #[test]
    fn apply_add_loads_the_application_into_the_model() {
        let mut store = SqliteStore::open_in_memory().unwrap();
        let mut model = Model::new(store.load().unwrap(), PathBuf::from("mem"));
        let effect = Effect::Apply(Op::Add {
            company: "Acme".into(),
            role: "Backend".into(),
            stage: Stage::Applied,
            on: day(1),
        });
        dispatch(&mut model, &mut store, effect).unwrap();
        assert_eq!(model.apps.len(), 1);
        assert_eq!(model.apps[0].company, "Acme");
        assert_eq!(model.selected_id(), Some(model.apps[0].id));
        assert!(model.status.starts_with("wrote application"));
        assert!(!model.status_error);
    }

    #[test]
    fn a_failed_apply_keeps_the_snapshot() {
        let mut store = SqliteStore::open_in_memory().unwrap();
        let mut model = Model::new(store.load().unwrap(), PathBuf::from("mem"));
        dispatch(
            &mut model,
            &mut store,
            Effect::Apply(Op::Delete { application_id: 99 }),
        )
        .unwrap();
        assert!(model.apps.is_empty());
        assert!(model.status_error);
        assert!(model.status.contains("no application 99"));
    }

    #[test]
    fn adding_a_closed_application_says_when_the_filter_hides_it() {
        let mut store = SqliteStore::open_in_memory().unwrap();
        let mut model = Model::new(store.load().unwrap(), PathBuf::from("mem"));
        dispatch(
            &mut model,
            &mut store,
            Effect::Apply(Op::Add {
                company: "Acme".into(),
                role: "Backend".into(),
                stage: Stage::Refused,
                on: day(1),
            }),
        )
        .unwrap();
        assert_eq!(model.apps.len(), 1);
        assert!(model.selected_id().is_none());
        assert!(model.status.contains("hidden by the current filter"));
        assert!(!model.status_error);
    }

    #[test]
    fn export_writes_a_json_file() {
        let mut store = SqliteStore::open_in_memory().unwrap();
        let mut model = Model::new(store.load().unwrap(), PathBuf::from("mem"));
        dispatch(
            &mut model,
            &mut store,
            Effect::Apply(Op::Add {
                company: "Acme".into(),
                role: "Backend".into(),
                stage: Stage::Applied,
                on: day(1),
            }),
        )
        .unwrap();
        let path = std::env::temp_dir().join(format!(
            "job-tracker-export-{}-{}.json",
            std::process::id(),
            model.apps[0].id
        ));
        dispatch(&mut model, &mut store, Effect::Export(path.clone())).unwrap();
        let json = std::fs::read_to_string(&path).unwrap();
        assert!(json.contains("Acme"), "{json}");
        assert!(model.status.contains("exported"));
        assert!(!model.status_error);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn only_key_presses_become_messages() {
        let press = Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert_eq!(
            map_event(press, Mode::Normal, day(1), 80),
            Some(Message::Quit)
        );
        let mut release = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        assert_eq!(
            map_event(Event::Key(release), Mode::Normal, day(1), 80),
            None
        );

        let typed = Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert_eq!(
            map_event(typed, Mode::Command, day(1), 80),
            Some(Message::CommandInsert("q".into()))
        );
        let ctrl_c = Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(
            map_event(ctrl_c, Mode::Command, day(1), 80),
            Some(Message::Quit)
        );
    }

    fn press(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn normal_keys_prefill_commands_and_the_picker_uses_letters() {
        let today = day(17);
        assert_eq!(
            map_event(press(KeyCode::Char('n')), Mode::Normal, today, 80),
            Some(Message::BeginCommand("add ".into()))
        );
        assert_eq!(
            map_event(press(KeyCode::Char('d')), Mode::Normal, today, 80),
            Some(Message::PromptDelete)
        );
        assert_eq!(
            map_event(press(KeyCode::Char('/')), Mode::Normal, today, 80),
            Some(Message::BeginCommand("filter ".into()))
        );
        assert_eq!(
            map_event(press(KeyCode::Char('s')), Mode::StagePick, today, 80),
            Some(Message::PickStage {
                stage: Stage::SecondMeeting,
                today,
            })
        );
        assert_eq!(
            map_event(press(KeyCode::Char('c')), Mode::StagePick, today, 80),
            Some(Message::PickStage {
                stage: Stage::TechnicalMeeting,
                today,
            })
        );
    }

    #[test]
    fn today_reads_the_local_civil_date() {
        let today = today();
        assert!((2020..=2100).contains(&today.year()));
    }
}
