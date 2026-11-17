# Job tracker TUI

A local terminal program for tracking job applications. Each application is a company and a role, plus a history of stage changes. Every stage change has the calendar date it happened. The program stores that history, renders the current list, and draws each application across time.

This is a design for a new crate. It does not share the What's Next movie API, its Postgres database, or its auth. Checked against crates.io and the Ratatui docs on 17 Nov 2026.

## Recommendation

| Piece | Choice |
| --- | --- |
| UI | [Ratatui](https://ratatui.rs) 0.30 with its Crossterm backend |
| Shape of the program | The Elm Architecture: a `Model`, a `Message`, an `update` function, a `view` function. The event loop owns the database |
| Mutations | One command language. Key bindings fill that command; they do not write the database themselves |
| Store | SQLite through [rusqlite](https://crates.io/crates/rusqlite) 0.43, `bundled` feature, one file |
| Dates | [jiff](https://crates.io/crates/jiff) 1.0 civil dates (`YYYY-MM-DD`), stored as text |
| Errors | [color-eyre](https://crates.io/crates/color-eyre) 0.6 for fatal errors, a status line for command and database errors |
| Paths | [directories](https://crates.io/crates/directories) 6, `data_local_dir()` |
| Rust | Edition 2024. MSRV 1.89, set by rusqlite 0.43 |
| Crate | `apps/tracker`, binary name `tracker`, its own `Cargo.lock` |

## Decisions

These four choices are settled. The last section records what each alternative would have cost, so a reversal has a known blast radius.

1. The database is one local SQLite file. The movie API stays untouched.
2. A stage change is appended to a history. The current stage is the latest event. Past events can be corrected.
3. The time graph is one lane per application, colored by stage. A single line of counts summarizes the pipeline.
4. When this is built, the code lives in `apps/tracker`. This file is the design only.

## Why this stack

Ratatui is the maintained immediate-mode TUI library (about 6,900 direct dependents, active as of November 2026). The application owns the event loop, the state, and the widgets. That matches a program whose interesting work is a date-lane widget and a command parser, both of which should be testable without a terminal.

[Ratatui's own comparison](https://ratatui.rs/ecosystem/alternatives/) (November 2026) is the reason the other UI crates lose:

| Library | What it is good at | Why it loses here |
| --- | --- | --- |
| Ratatui | Custom widgets, lists, charts, full control of the loop | This is the one to use |
| Cursive | Forms and dialogs. Last release 2024 | A retained view tree fights a custom timeline, and the project is quiet |
| AppCUI | Desktop-style windows | Two crates on crates.io. No widget ecosystem to stand on |
| iocraft | Declarative CLI output | Aimed at command output, not a long-lived full-screen app |
| R3BL TUI | Async editor-style apps | A large framework for a list, a command line, and a chart |
| tui-realm | Elm-style components on top of Ratatui | A framework around the pattern we can write directly in a few modules |
| termion | Unix terminals | No Windows support. This machine is Windows |

Persistence follows the same rule: the smallest crate that matches the loop.

| Store | Fit |
| --- | --- |
| rusqlite | Sync API, same thread as `event::poll`. `bundled` compiles SQLite, so Windows does not need a system library |
| SQLx | The right client for the movie API's async Postgres. An async pool adds a runtime the TUI never awaits |
| Diesel | Compile-time queries for a schema of two tables |
| A JSON file | Hand-editable and diffable. A crash mid-write needs a temp-file rename dance, and date-range reads become application code. Reasonable if the file should live in git; see the last section |

Dates are civil dates. An application stage does not have a time of day or a time zone. `jiff::civil::Date` parses `YYYY-MM-DD` via `FromStr`. Store that string in SQLite. rusqlite 0.43's optional `jiff` feature tracks jiff 0.2, and jiff 1.0 shipped on 15 Nov 2026, so the conversion stays in our code until that feature catches up.

The movie API uses `chrono` and SQLx. This crate does not, so a jiff 1 upgrade here cannot break the API.

## Architecture

Three layers. Only the event loop may touch both the model and the database.

```
key or command
    -> Message
    -> update(&mut Model) -> Effect
    -> loop runs Effect against Store
    -> Message::Loaded(Snapshot) or Message::Failed(text)
    -> update again
    -> view(&mut Model, Frame)
```

`update` is a pure state transition. It returns a description of work and does not open SQLite. `view` reads the model and writes widgets. The official Ratatui TEA notes allow `view` to take `&mut Model` because `TableState` is a stateful widget; we do that, and `view` still does not perform I/O.

```rust
enum Effect {
    None,
    Apply(Op),
    Export(PathBuf),
    Quit,
}
```

The model holds the last successful snapshot plus UI state. A failed `apply` sends `Message::Failed` and leaves the snapshot as it was.

```rust
struct Model {
    apps: Vec<Application>,
    selection: usize,
    table_state: TableState,
    mode: Mode,
    command: CommandLine,
    filter: Filter,
    sort: Sort,
    timeline: Option<DateWindow>,
    status: String,
    running: bool,
}

enum Mode {
    Normal,
    Command,
    StagePick,
    Help,
}
```

`DateWindow` set to `None` means the timeline fits the visible applications. An explicit window is what pan and zoom edit.

Proposed modules, when the crate exists:

```
apps/tracker/
  Cargo.toml
  src/
    main.rs       open the db, install error hooks, run the loop
    tui.rs        ratatui::init / ratatui::restore
    model.rs
    message.rs
    update.rs     Message -> Effect
    view.rs       layout and widgets
    command.rs    text -> Op
    domain.rs     Stage, Application, Op, invariants
    timeline.rs   date window -> columns and colors
    store.rs      Store trait + SqliteStore
```

`domain`, `command`, `timeline`, and `store` have no Ratatui imports. That is what makes them unit-testable on Windows without a TTY.

### Cargo.toml

```toml
[package]
name = "tracker"
version = "0.1.0"
edition = "2024"
rust-version = "1.89"

[dependencies]
ratatui = "0.30"
rusqlite = { version = "0.43", features = ["bundled"] }
jiff = "1"
color-eyre = "0.6"
directories = "6"
thiserror = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
unicode-segmentation = "1"
```

Ratatui 0.30 re-exports Crossterm. A direct `crossterm` dependency is unnecessary. `serde_json` exists for `:export`. `unicode-segmentation` moves the command cursor by grapheme so accented company names edit correctly. `thiserror` is the store and parse error type.

There is no workspace `Cargo.toml` at the repo root today. `apps/tracker` keeps its own lockfile, the same way `apps/api` does.

## Domain

An application always has at least one stage event. The current stage is the event with the greatest `(occurred_on, id)`.

```rust
struct Application {
    id: i64,
    company: String,
    role: String,
    events: Vec<StageEvent>, // sorted by (occurred_on, id)
}

struct StageEvent {
    id: i64,
    stage: Stage,
    occurred_on: jiff::civil::Date,
}

enum Stage {
    Applied,
    FirstMeeting,
    SecondMeeting,
    ThirdMeeting,
    TechnicalMeeting,
    Refused,
    OfferMade,
    OfferAccepted,
}
```

Stored tokens, in pipeline order:

| Token | On screen | Counts as |
| --- | --- | --- |
| `applied` | Applied | Active |
| `first_meeting` | First meeting | Active |
| `second_meeting` | Second meeting | Active |
| `third_meeting` | Third meeting | Active |
| `technical_meeting` | Technical meeting | Active |
| `offer_made` | Offer made | Active. The decision is still open |
| `offer_accepted` | Offer accepted | Closed |
| `refused` | Refused | Closed |

Pipeline order is display order for the summary line. It is not a state machine. `applied` can jump to `technical_meeting`, and `offer_made` can move to `refused`. Any stage can follow any stage, because people record corrections and unusual processes.

Invariants the store enforces inside the write transaction:

- Company and role are trimmed, non-empty, and at most 200 characters.
- The first write for an application inserts the row and its first event together.
- `delete-event` of the only event fails. Deleting the application is the way to drop it.
- Dates may be in the future (a meeting already scheduled).
- The same company and role may exist on more than one row. A re-apply is a new id.
- Ids come from `INTEGER PRIMARY KEY AUTOINCREMENT`, so a deleted id is not reused in commands the user already typed.

`Op` is the only write the store understands:

```rust
enum Op {
    Add { company: String, role: String, stage: Stage, on: Date },
    SetStage { application_id: i64, stage: Stage, on: Date },
    Rename { application_id: i64, company: String, role: String },
    Delete { application_id: i64 },
    AmendEvent { event_id: i64, stage: Stage, on: Date },
    DeleteEvent { event_id: i64 },
}
```

`Rename` does not append a stage event. `SetStage` always appends. `AmendEvent` rewrites one past row.

Active means the current stage is neither `refused` nor `offer_accepted`. The default filter is active.

## Storage

One SQLite file. On startup the program creates the directory, opens the file, sets the pragmas, runs migrations, and loads every application. A personal pipeline stays in the hundreds of rows, and a low number of thousands is still a trivial load. Filtering and sorting happen on the snapshot in memory. The SQL is the durability layer.

Default path, from `ProjectDirs::from("", "", "job-tracker").data_local_dir()`:

| OS | File |
| --- | --- |
| Windows | `%LOCALAPPDATA%\job-tracker\data\tracker.db` |
| Linux | `~/.local/share/job-tracker/tracker.db` |
| macOS | `~/Library/Application Support/job-tracker/tracker.db` |

`data_local_dir` keeps the file in `%LOCALAPPDATA%` on Windows, so a roaming profile is not asked to sync a live database. `--db <path>` replaces this. If `ProjectDirs` returns `None`, the program requires `--db`.

Connection setup for a file database:

```sql
PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;
PRAGMA busy_timeout = 3000;
```

WAL means a crash during a commit leaves the previous commit intact. Quit the program before copying `tracker.db` as a backup, so the `-wal` and `-shm` sidecars are checkpointed. `:export` writes a JSON copy that is safe to take while the program is open. A second process that finds the file busy gets the error on the status line.

Schema, migration version 1, applied when `PRAGMA user_version` is 0. Bump `user_version` in the same transaction as the DDL. Two tables do not need a migration crate.

```sql
CREATE TABLE applications (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    company    TEXT NOT NULL CHECK (length(company) BETWEEN 1 AND 200),
    role       TEXT NOT NULL CHECK (length(role) BETWEEN 1 AND 200)
) STRICT;

CREATE TABLE stage_events (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    application_id  INTEGER NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    stage           TEXT NOT NULL CHECK (stage IN (
                        'applied',
                        'first_meeting',
                        'second_meeting',
                        'third_meeting',
                        'technical_meeting',
                        'refused',
                        'offer_made',
                        'offer_accepted'
                    )),
    occurred_on     TEXT NOT NULL CHECK (occurred_on GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]')
) STRICT;

CREATE INDEX stage_events_by_app
    ON stage_events (application_id, occurred_on, id);
```

Adding a stage later is a new migration that rebuilds the `CHECK`. That is deliberate: the token list in SQLite and the `Stage` enum move together.

`Store` has three methods.

```rust
trait Store {
    fn load(&self) -> Result<Snapshot, StoreError>;
    fn apply(&mut self, op: Op) -> Result<Snapshot, StoreError>;
    fn export_json(&self) -> Result<String, StoreError>;
}
```

`apply` runs one transaction, then `load`. The model swaps its snapshot only after that load returns. Tests call `SqliteStore` on `Connection::open_in_memory()` (skip WAL there; it is a file-database pragma). There is no second, hand-rolled in-memory database to drift out of sync.

Export shape, pretty-printed:

```json
[
  {
    "id": 1,
    "company": "Acme",
    "role": "Backend engineer",
    "events": [
      { "id": 1, "stage": "applied", "date": "2026-11-01" },
      { "id": 4, "stage": "first_meeting", "date": "2026-11-14" }
    ]
  }
]
```

## Commands

Every mutation is a command that parses to an `Op`. The command line is how the user types it. Keys that change data only pre-fill that line.

Grammar, after the leading colon:

```
add <company> | <role> [| <stage>] [| <date>]
stage <id> <stage> [<date>]
rename <id> | <company> | <role>
delete <id>
event <event_id> <stage> <date>
delete-event <event_id>
filter [<active|closed|all|offer> | <substring>]
export <path>
```

`<stage>` is the token (`first_meeting`) or a unique prefix (`first`, `tech`, `made`, `accepted`). An ambiguous prefix is a parse error and does not write. An omitted date is today, from `jiff::Zoned::now().date()` in the local time zone. An omitted stage on `add` is `applied`.

```
:add Acme | Backend engineer
:add Acme | Backend engineer | technical | 2026-11-01
:stage 3 first
:stage 3 technical_meeting 2026-11-17
:rename 3 | Acme Corp | Backend engineer
:delete 3
:event 4 first_meeting 2026-11-12
:delete-event 4
:filter active
:filter all
:filter acme
:export C:\Users\me\Desktop\applications.json
```

`:filter` with no argument returns to `active`. `closed` is `refused` and `offer_accepted`. `offer` is `offer_made` and `offer_accepted`. Any other word is a case-insensitive substring of company or role. The table and the timeline use the same filtered slice.

A parse error or a store error replaces the status line. The database is unchanged.

### Keys

Handle `KeyEventKind::Press` only. Crossterm on Windows also delivers key-release events, and acting on those double-fires every shortcut. The [user-input example](https://ratatui.rs/examples/apps/user_input/) filters the same way.

`Ctrl-C` quits from every mode. Raw mode swallows the terminal's normal `Ctrl-C`.

Normal mode:

| Key | Action |
| --- | --- |
| `j` / `k`, or Up / Down | Move the selection. The timeline highlights the same row |
| `Enter` | Open the detail pane when the terminal is narrower than 100 columns |
| `n` | Command line pre-filled with `add ` |
| `s` | Stage picker for the selected application, dated today |
| `d` | Command line pre-filled with `delete <id> `. Enter confirms it |
| `/` | Command line pre-filled with `filter ` |
| `o` | Cycle sort: recent stage date, company, pipeline stage |
| `h` / `l` | Pan the timeline by a quarter of its window |
| `-` / `=` | Zoom the timeline |
| `Home` | Fit the timeline to the visible applications |
| `:` | Empty command line |
| `?` | Help overlay |
| `q` | Quit |
| `Esc` | Close detail, help, or the picker |

The stage picker is one line of letters (`a` applied, `f` first, `s` second, `t` third, `c` technical, `m` offer made, `o` offer accepted, `r` refused). Choosing a letter submits `SetStage` for the selected id at today's date. A different date is `:stage`.

Command mode uses a one-line buffer: printable characters insert, Backspace deletes, Left and Right move by grapheme, Enter submits, Esc cancels. That is the behavior in Ratatui's user-input example, with grapheme cursor movement added. A multi-line editor (`ratatui-textarea`) earns its dependency when a notes field exists. Company, role, stage, and date fit on one line.

Leave mouse capture off so the terminal can still select text to copy.

Sort default is the current stage date, newest first, then id.

## Screen

The layout that has to work is 80×24. A third chart does not fit, so the funnel is a colored counts line rather than a `BarChart`. Ratatui's `Chart` widget plots numeric series; the lane graph is a date axis painted by stage, which is a custom `Widget`.

```
┌ Applications ────────────────────────────────────────────┐
│ id  Company        Role            Stage         Since   │
│  3  Acme           Backend         First meeting 11-14   │
│ >4  Northwind      iOS             Technical     11-17   │
├ 4 applied · 2 first · 1 technical · 1 offer ─────────────┤
│ Nov 01          Nov 08          Nov 15          Nov 22   │
│ Acme      ███████████░░░░░░░░                              │
│ Northwind       ░░░░░██████████████                        │
├ :stage 4 technical ───────────────────────────────────────┤
│ wrote application 4                                       │
└───────────────────────────────────────────────────────────┘
```

Blocks in a lane are `█`, styled with the stage color for the date that column represents. The selected row uses the table's highlight style and a marker on its lane.

At width 100 or more, a detail column sits on the right: id, company, role, current stage, and every event from oldest to newest with its event id. Below that width, Enter swaps the body for that detail and Esc returns. Column priority when the table is tight: stage, since, company, role, id.

Stage colors are the named ANSI colors, so a 16-color Windows Terminal still distinguishes them:

| Stage | Color |
| --- | --- |
| Applied | Blue |
| First meeting | Cyan |
| Second meeting | Green |
| Third meeting | Yellow |
| Technical meeting | Magenta |
| Offer made | Light green |
| Offer accepted | White |
| Refused | Red |

### Timeline geometry

`timeline.rs` takes the filtered applications, a date window, and a width in columns. It returns, for each application, one color per column. `view` only paints that.

Window when the user has not panned: the earliest visible event through the later of today and the latest visible event.

A column owns a date:

```
span_days = max(1, days from window start to window end)
column    = day_offset * (width - 1) / span_days
```

Walking a column back to a date uses the inverse, so tests can round-trip a known date to a column and back within a day.

Color of a column for one application:

- Sort its events by `(occurred_on, id)`.
- An event covers from its date inclusive up to the next event's date exclusive.
- Two events on the same day: the later id wins the column. The detail pane still lists both.
- The last event of an active application covers through today, and through its own date when that date is in the future.
- The last event of a closed application (`refused`, `offer_accepted`) covers only its own date. The lane stops when the process stopped.
- Columns before the first event stay empty.

The timeline renders the same row slice as the table, including scroll position. One `selection` drives both.

## Terminal lifecycle

Match the [Ratatui 0.30 hello-world template](https://crates.io/crates/ratatui):

```rust
fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let terminal = ratatui::init();
    let result = run(terminal);
    ratatui::restore();
    result
}
```

`color_eyre::install()` runs before `ratatui::init()`, which is the order in that template. Fatal errors print after the terminal is restored. Command mistakes and constraint failures stay on the status line; they are not eyre reports.

Do not send `tracing` to stderr. The alternate screen would fight the logs. The status line is the error channel. The help screen prints the database path so a backup has an obvious target.

## Tests

Run with `cargo test`. No terminal required.

| Test | Where |
| --- | --- |
| Parse each command, including bad prefixes, empty company, and default date | `command.rs` |
| Each `Op` against `open_in_memory()`: add, append stage, rename, amend, delete event, refuse to delete the last event, cascade delete | `store.rs` |
| Current stage is the latest `(occurred_on, id)`, including two events on one day | `domain.rs` |
| A closed lane stops on its last date; an active lane continues through today | `timeline.rs` |
| A known date lands on the expected column for a fixed window and width | `timeline.rs` |
| `update` changes mode and emits `Effect::Apply` without a `Store` | `update.rs` |
| Draw the fixture snapshot into `ratatui::backend::TestBackend` and assert the company name is on the buffer | `view.rs` |

One `TestBackend` render is enough to lock the layout to the model. Snapshot-testing the whole frame is brittle across width changes and can wait.

## Build order

1. `domain`, schema, and `SqliteStore`, with the in-memory tests.
2. Command parser, with the parse tests.
3. Event loop, table, selection, filter, sort.
4. Command line wired to `Effect::Apply`.
5. Timeline widget and the counts line.
6. Detail pane, stage picker, `:export`, help screen with the database path.

Each step is usable. The list is real as soon as step 4 lands; the graph is step 5 and does not change the store.

## Alternatives set aside

Each of these was a real option. They lost.

**JSON as the source of truth.** Replace `store.rs` and the schema with one pretty-printed file, written by serializing to a temporary file and renaming over the destination. `Op`, the command language, the model, and the timeline stay. That drops the `CHECK` constraints and transactional multi-row writes, and gains a file that git can diff. Worth revisiting only if the tracker should live in a dotfiles repo.

**One current stage per application, overwrite on change.** Drop `stage_events` down to columns on `applications` (`stage`, `occurred_on`). `AmendEvent` and `DeleteEvent` go away. The lane collapses to a marker on the current date, because the dates of earlier stages are gone. The list and the commands `add`, `stage`, `rename`, and `delete` still hold.

**Counts over time instead of lanes.** Replace `timeline.rs` with a `BarChart` or `Chart` of "applications that changed stage in this week" (or "applications active at the end of this week"). The snapshot already has the dates. The table layout stays.

**Sharing the movie API's Postgres.** The TUI becomes an HTTP client of a new authenticated route, with Tokio and the API's JWT. That is a different program: offline use, account scope, and where the file lives all move to the server. The domain types can still be copied, but `rusqlite` and `directories` drop out.

## Sources

- [The Elm Architecture](https://ratatui.rs/concepts/application-patterns/the-elm-architecture/) and the [0.30 counter-app template](https://ratatui.rs/tutorials/counter-app/basic-app/), including `ratatui::init` / `ratatui::restore`.
- [Library comparison](https://ratatui.rs/ecosystem/alternatives/), status column dated November 2026. [Chart example](https://ratatui.rs/examples/apps/chart/) for numeric series. [User input](https://ratatui.rs/examples/apps/user_input/) for the one-line editor and `KeyEventKind::Press`.
- [Blessed.rs](https://blessed.rs/crates) on rusqlite for sync SQLite, and on jiff as the date crate with the preferred API when the rest of the ecosystem does not force chrono.
- Crate versions read from crates.io on 17 Nov 2026: ratatui 0.30.2, rusqlite 0.43.0, jiff 1.0.0, color-eyre 0.6.5, directories 6.0.0, thiserror 2.0.21. `directories` 6 `data_local_dir` is `%LOCALAPPDATA%\<app>\data` on Windows.
