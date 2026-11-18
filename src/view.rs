use std::path::Path;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Cell, Clear, HighlightSpacing, Paragraph, Row, Table, Wrap};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::domain::{Application, Stage};
use crate::model::{Mode, Model};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Field {
    Id,
    Company,
    Role,
    Stage,
    AppliedOn,
    Since,
}

impl Field {
    fn header(self) -> &'static str {
        match self {
            Field::Id => "id",
            Field::Company => "Company",
            Field::Role => "Role",
            Field::Stage => "Stage",
            Field::AppliedOn => "Applied On",
            Field::Since => "Since",
        }
    }

    fn cell(self, app: &Application) -> Cell<'static> {
        match self {
            Field::Id => Cell::from(app.id.to_string()),
            Field::Company => Cell::from(app.company.clone()),
            Field::Role => Cell::from(app.role.clone()),
            Field::Stage => match app.current() {
                Some(event) => {
                    Cell::from(event.stage.label()).style(Style::new().fg(stage_color(event.stage)))
                }
                None => Cell::from("-"),
            },
            Field::AppliedOn => Cell::from(short_date(app.applied_on)),
            Field::Since => Cell::from(
                app.current()
                    .map(|event| short_date(event.occurred_on))
                    .unwrap_or_default(),
            ),
        }
    }
}

struct Spec {
    field: Field,
    min: u16,
    max: u16,
    priority: u8,
    flex: bool,
}

fn field_specs() -> [Spec; 6] {
    [
        Spec {
            field: Field::Id,
            min: 4,
            max: 4,
            priority: 1,
            flex: false,
        },
        Spec {
            field: Field::Company,
            min: 8,
            max: u16::MAX,
            priority: 3,
            flex: true,
        },
        Spec {
            field: Field::Role,
            min: 8,
            max: u16::MAX,
            priority: 2,
            flex: true,
        },
        Spec {
            field: Field::Stage,
            min: 10,
            max: 18,
            priority: 5,
            flex: false,
        },
        Spec {
            field: Field::AppliedOn,
            min: 10,
            max: 10,
            priority: 4,
            flex: false,
        },
        Spec {
            field: Field::Since,
            min: 5,
            max: 6,
            priority: 4,
            flex: false,
        },
    ]
}

fn column_plan(area_width: u16) -> Vec<(Field, u16)> {
    let budget = area_width.saturating_sub(3);
    if budget == 0 {
        return vec![(Field::Stage, 1)];
    }

    let specs = field_specs();
    let mut ranked: Vec<&Spec> = specs.iter().collect();
    ranked.sort_by_key(|spec| std::cmp::Reverse(spec.priority));

    let mut chosen: Vec<&Spec> = Vec::new();
    let mut used = 0u16;
    for spec in ranked {
        let gap = u16::from(!chosen.is_empty());
        if used.saturating_add(gap).saturating_add(spec.min) <= budget {
            used = used.saturating_add(gap).saturating_add(spec.min);
            chosen.push(spec);
        }
    }
    if chosen.is_empty() {
        return vec![(Field::Stage, budget)];
    }

    let mut widths: Vec<u16> = chosen.iter().map(|spec| spec.min).collect();
    let mut leftover = budget.saturating_sub(used);
    for (width, spec) in widths.iter_mut().zip(&chosen) {
        if spec.flex {
            continue;
        }
        let add = spec.max.saturating_sub(*width).min(leftover);
        *width += add;
        leftover -= add;
    }

    let flex_count = chosen.iter().filter(|spec| spec.flex).count();
    if flex_count > 0 {
        let flex_count = u16::try_from(flex_count).unwrap_or(1);
        let share = leftover / flex_count;
        let mut remainder = leftover % flex_count;
        for (width, spec) in widths.iter_mut().zip(&chosen) {
            if !spec.flex {
                continue;
            }
            let mut add = share;
            if remainder > 0 {
                add += 1;
                remainder -= 1;
            }
            *width += add.min(spec.max.saturating_sub(*width));
        }
    }

    let mut plan: Vec<(Field, u16)> = chosen
        .into_iter()
        .zip(widths)
        .map(|(spec, width)| (spec.field, width))
        .collect();
    plan.sort_by_key(|(field, _)| *field);
    plan
}

pub(crate) fn draw(frame: &mut Frame, model: &mut Model) {
    let [body, command, status] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    draw_body(frame, model, body);
    if let Some((x, y)) = draw_command(frame, model, command) {
        frame.set_cursor_position((x, y));
    }
    draw_status(frame, model, status);
}

fn draw_body(frame: &mut Frame, model: &mut Model, area: Rect) {
    if model.mode == Mode::Help {
        draw_help(frame, model, area);
        return;
    }

    if area.width >= 100 {
        let [list, detail] =
            Layout::horizontal([Constraint::Fill(1), Constraint::Length(36)]).areas(area);
        draw_table(frame, model, list);
        draw_detail(frame, model, detail);
    } else if model.show_detail {
        draw_detail(frame, model, area);
    } else {
        draw_table(frame, model, area);
    }

    if model.mode == Mode::StagePick {
        draw_stage_picker(frame, model, area);
    }
}

fn draw_table(frame: &mut Frame, model: &mut Model, area: Rect) {
    model.sync_table();
    let indices = model.visible_indices();
    let plan = column_plan(area.width);
    let title = format!(
        "Applications · {} · {} · {}",
        indices.len(),
        model.filter.label(),
        model.sort.label()
    );
    let block = Block::bordered().title(title);

    if indices.is_empty() {
        let message = if model.apps.is_empty() {
            "No applications yet. Press n to add one."
        } else {
            "No applications match this filter."
        };
        frame.render_widget(Paragraph::new(message).block(block), area);
        return;
    }

    let header = Row::new(plan.iter().map(|(field, _)| field.header()))
        .style(Style::new().add_modifier(Modifier::BOLD));
    let widths: Vec<Constraint> = plan
        .iter()
        .map(|(_, width)| Constraint::Length(*width))
        .collect();
    let rows: Vec<Row> = indices
        .iter()
        .map(|&index| {
            let app = &model.apps[index];
            Row::new(plan.iter().map(|(field, _)| field.cell(app)))
        })
        .collect();
    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .column_spacing(1)
        .row_highlight_style(Style::new().add_modifier(Modifier::REVERSED))
        .highlight_symbol(">")
        .highlight_spacing(HighlightSpacing::Always);
    frame.render_stateful_widget(table, area, &mut model.table_state);
}

fn draw_detail(frame: &mut Frame, model: &Model, area: Rect) {
    let text = match model.selected_app() {
        Some(app) => detail_text(app),
        None => Text::from("No application selected."),
    };
    let paragraph = Paragraph::new(text)
        .block(Block::bordered().title("Detail"))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn detail_text(app: &Application) -> Text<'static> {
    let mut lines = vec![
        Line::from(format!("{}  {}", app.id, app.company))
            .style(Style::new().add_modifier(Modifier::BOLD)),
        Line::from(app.role.clone()),
    ];
    lines.push(Line::from(format!("applied on {}", app.applied_on)));
    if let Some(current) = app.current() {
        lines.push(Line::from(Span::styled(
            current.stage.label(),
            Style::new().fg(stage_color(current.stage)),
        )));
        lines.push(Line::from(format!("since {}", current.occurred_on)));
    }
    lines.push(Line::from(""));
    lines.push(Line::from("Events").style(Style::new().add_modifier(Modifier::BOLD)));
    for event in &app.events {
        lines.push(Line::from(vec![
            Span::raw(format!("{:>4}  ", event.id)),
            Span::styled(
                format!("{:<18}", event.stage.label()),
                Style::new().fg(stage_color(event.stage)),
            ),
            Span::raw(event.occurred_on.to_string()),
        ]));
    }
    Text::from(lines)
}

fn draw_help(frame: &mut Frame, model: &Model, area: Rect) {
    let paragraph = Paragraph::new(help_text(&model.db_path))
        .block(Block::bordered().title("Help"))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn help_text(path: &Path) -> Text<'static> {
    Text::from(vec![
        Line::from(format!("Database  {}", path.display())),
        Line::from(""),
        Line::from("j/k, up/down  move      n add           s set stage"),
        Line::from("d delete       / filter       o cycle sort"),
        Line::from("Enter detail   : command      ? this help"),
        Line::from("q quit         Esc close      Ctrl-C quit"),
        Line::from(""),
        Line::from("add <company> | <role> [| stage] [| date]"),
        Line::from("stage <id> <stage> [date]"),
        Line::from("rename <id> | <company> | <role>"),
        Line::from("delete <id>"),
        Line::from("event <event id> <stage> <date>"),
        Line::from("delete-event <event id>"),
        Line::from("filter [active|closed|all|offer|text]"),
        Line::from("export <path>"),
        Line::from("Stages: applied, first_meeting, second_meeting,"),
        Line::from("third_meeting, technical_meeting, offer_made,"),
        Line::from("offer_accepted, refused. A unique prefix works."),
        Line::from("Picker: a f s t c m o r, dated today. Empty filter is active."),
        Line::from("The time graph is not drawn in this build."),
    ])
}

fn draw_stage_picker(frame: &mut Frame, model: &Model, area: Rect) {
    let popup = centered(area, 62, 6);
    let paragraph =
        Paragraph::new(picker_text(model.selected_app())).block(Block::bordered().title("Stage"));
    frame.render_widget(Clear, popup);
    frame.render_widget(paragraph, popup);
}

fn picker_text(app: Option<&Application>) -> Text<'static> {
    let title = match app {
        Some(app) => format!("{} - {}", app.company, app.role),
        None => "No application selected".to_string(),
    };
    Text::from(vec![
        Line::from(title).style(Style::new().add_modifier(Modifier::BOLD)),
        Line::from("a applied     f first       s second    t third"),
        Line::from("c technical   m offer made  o accepted  r refused"),
        Line::from("Records today. Esc cancels. :stage picks a date."),
    ])
}

fn draw_command(frame: &mut Frame, model: &Model, area: Rect) -> Option<(u16, u16)> {
    if area.width == 0 {
        return None;
    }
    let budget = usize::from(area.width.saturating_sub(2));
    let (shown, column) = if model.mode == Mode::Command {
        command_window(&model.command.buffer, model.command.cursor, budget)
    } else {
        (String::new(), 0)
    };
    let prompt_style = if model.mode == Mode::Command {
        Style::new().fg(Color::Yellow)
    } else {
        Style::new().fg(Color::DarkGray)
    };
    let line = Line::from(vec![Span::styled(":", prompt_style), Span::raw(shown)]);
    frame.render_widget(Paragraph::new(line), area);

    if model.mode != Mode::Command || area.width < 2 {
        return None;
    }
    let x = area
        .x
        .saturating_add(1)
        .saturating_add(u16::try_from(column).unwrap_or(u16::MAX));
    Some((x.min(area.right().saturating_sub(1)), area.y))
}

fn draw_status(frame: &mut Frame, model: &Model, area: Rect) {
    let style = if model.status_error {
        Style::new().fg(Color::Red)
    } else {
        Style::new()
    };
    frame.render_widget(Paragraph::new(model.status.as_str()).style(style), area);
}

fn short_date(date: jiff::civil::Date) -> String {
    date.strftime("%m-%d").to_string()
}

fn stage_color(stage: Stage) -> Color {
    match stage {
        Stage::Applied => Color::Blue,
        Stage::FirstMeeting => Color::Cyan,
        Stage::SecondMeeting => Color::Green,
        Stage::ThirdMeeting => Color::Yellow,
        Stage::TechnicalMeeting => Color::Magenta,
        Stage::OfferMade => Color::LightGreen,
        Stage::OfferAccepted => Color::White,
        Stage::Refused => Color::Red,
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width, height)
}

struct Grapheme<'a> {
    text: &'a str,
    width: usize,
}

fn command_window(buffer: &str, cursor: usize, width: usize) -> (String, usize) {
    if width == 0 {
        return (String::new(), 0);
    }
    let graphemes = buffer
        .graphemes(true)
        .map(|text| Grapheme {
            width: UnicodeWidthStr::width(text),
            text,
        })
        .collect::<Vec<_>>();
    let cursor_at = cursor_grapheme(&graphemes, cursor);

    let mut start = cursor_at;
    let mut used = 0;
    while start > 0 {
        let grapheme_width = graphemes[start - 1].width;
        if used + grapheme_width > width {
            break;
        }
        used += grapheme_width;
        start -= 1;
    }
    let mut end = cursor_at;
    while end < graphemes.len() {
        let grapheme_width = graphemes[end].width;
        if used + grapheme_width > width {
            break;
        }
        used += grapheme_width;
        end += 1;
    }

    let mut shown = String::new();
    let mut column = 0;
    for (index, grapheme) in graphemes.iter().enumerate().take(end).skip(start) {
        if index < cursor_at {
            column += grapheme.width;
        }
        shown.push_str(grapheme.text);
    }
    (shown, column)
}

fn cursor_grapheme(graphemes: &[Grapheme<'_>], cursor: usize) -> usize {
    let mut end = 0;
    for (index, grapheme) in graphemes.iter().enumerate() {
        if cursor <= end {
            return index;
        }
        end += grapheme.text.len();
    }
    graphemes.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Snapshot, Stage, StageEvent};
    use crate::model::Model;
    use jiff::civil::Date;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::path::PathBuf;

    fn date(year: i16, month: i8, day: i8) -> Date {
        Date::new(year, month, day).unwrap()
    }

    fn fixture() -> Model {
        let acme = Application::new(
            3,
            "Acme".into(),
            "Backend".into(),
            vec![StageEvent {
                id: 2,
                stage: Stage::FirstMeeting,
                occurred_on: date(2026, 11, 14),
            }],
        );
        let northwind = Application::new(
            4,
            "Northwind".into(),
            "iOS".into(),
            vec![StageEvent {
                id: 9,
                stage: Stage::TechnicalMeeting,
                occurred_on: date(2026, 11, 17),
            }],
        );
        Model::new(
            Snapshot {
                applications: vec![acme, northwind],
            },
            PathBuf::from(r"C:\data\job-tracker\data\tracker.db"),
        )
    }

    fn render(model: &mut Model, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, model)).unwrap();
        buffer_text(terminal.backend().buffer())
    }

    fn buffer_text(buffer: &ratatui::buffer::Buffer) -> String {
        let mut text = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                text.push_str(buffer[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }

    #[test]
    fn fixture_company_is_on_the_buffer() {
        let mut model = fixture();
        let text = render(&mut model, 80, 24);
        assert!(text.contains("Acme"), "{text}");
        assert!(text.contains("Northwind"), "{text}");
        assert!(text.contains("First meeting"), "{text}");
        assert!(text.contains("11-14"), "{text}");
        assert!(text.contains("Stage"), "{text}");
        assert!(text.contains("Applied On"), "{text}");
        assert!(text.contains("Since"), "{text}");
        assert!(!text.contains('█'), "{text}");
    }

    #[test]
    fn narrow_detail_lists_the_selected_history() {
        let mut model = fixture();
        model.show_detail = true;
        let text = render(&mut model, 80, 24);
        assert!(text.contains("Northwind"), "{text}");
        assert!(text.contains("2026-11-17"), "{text}");
        assert!(text.contains("Technical meeting"), "{text}");
    }

    #[test]
    fn help_shows_the_database_path() {
        let mut model = fixture();
        model.mode = Mode::Help;
        model.status.clear();
        let text = render(&mut model, 80, 24);
        assert!(
            text.contains(r"C:\data\job-tracker\data\tracker.db"),
            "{text}"
        );
        assert!(
            text.contains("The time graph is not drawn in this build."),
            "{text}"
        );
    }

    #[test]
    fn wide_layout_shows_the_list_and_the_detail() {
        let mut model = fixture();
        let text = render(&mut model, 120, 24);
        assert!(text.contains("Acme"), "{text}");
        assert!(text.contains("Detail"), "{text}");
        assert!(text.contains("2026-11-17"), "{text}");
        assert!(text.contains("Northwind"), "{text}");
    }

    #[test]
    fn wide_table_keeps_every_column_and_tight_table_keeps_stage() {
        let wide: Vec<_> = column_plan(80)
            .into_iter()
            .map(|(field, _)| field)
            .collect();
        assert_eq!(
            wide,
            vec![
                Field::Id,
                Field::Company,
                Field::Role,
                Field::Stage,
                Field::AppliedOn,
                Field::Since
            ]
        );
        let tight: Vec<_> = column_plan(32)
            .into_iter()
            .map(|(field, _)| field)
            .collect();
        assert!(tight.contains(&Field::Stage));
        assert!(tight.contains(&Field::AppliedOn));
        assert!(tight.contains(&Field::Since));
        assert!(!tight.contains(&Field::Id));
    }

    #[test]
    fn command_window_keeps_the_cursor_on_screen() {
        assert_eq!(command_window("abcdef", 3, 10), ("abcdef".into(), 3));
        assert_eq!(command_window("abcdefghij", 10, 4), ("ghij".into(), 4));
        assert_eq!(command_window("é", "é".len(), 10), ("é".into(), 1));
    }
}
