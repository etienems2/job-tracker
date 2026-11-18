# tracker

Local terminal tracker for job applications.

## Start

```
cargo run
cargo run -- --db path\to\tracker.db
```

With no `--db`, the database is `%LOCALAPPDATA%\job-tracker\data\tracker.db`. `q` quits. `Ctrl-C` quits from every screen. `?` shows the database path.

## Import a CSV

```
cargo run -- import applications.csv
cargo run -- import applications.csv --db path\to\tracker.db
```

One row is one new application. `company` and `role` are required. `stage` and `date` may be blank, which means `applied` and today. Other columns are ignored. A company that contains a comma belongs in quotes. The file is one transaction: a bad row is named and nothing from that file is saved.

Dates are `YYYY-MM-DD`, such as `2026-11-02`.

```csv
company,role,stage,date
Acme,Backend engineer,,
Northwind,iOS,technical,2026-11-01
```

## Commands

Type these after `:`. A blank stage on `add` is `applied`. A blank date is today. A stage is a token or a unique prefix (`first`, `tech`, `made`, `accepted`).

```
add <company> | <role> [| <stage>] [| <date>]
stage <id> <stage> [<date>]
rename <id> | <company> | <role>
delete <id>
event <event id> <stage> <date>
delete-event <event id>
filter [active|closed|all|offer|<text>]
export <path>
```

Stages: `applied`, `first_meeting`, `second_meeting`, `third_meeting`, `technical_meeting`, `offer_made`, `offer_accepted`, `refused`.

`n` starts `add`, `s` picks a stage for today, `d` starts `delete`, `/` starts `filter`, and `o` cycles the sort. `filter` with no argument returns to `active`.
