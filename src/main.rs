mod command;
mod domain;
mod import;
mod message;
mod model;
mod store;
mod tui;
mod update;
mod view;

use std::path::{Path, PathBuf};

use color_eyre::eyre::eyre;
use directories::ProjectDirs;
use jiff::civil::Date;

use crate::store::SqliteStore;

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let args = parse_args(std::env::args().skip(1))?;
    if args.help {
        print_help();
        return Ok(());
    }
    let db_path = match args.db {
        Some(path) => path,
        None => default_db_path()?,
    };
    let store =
        SqliteStore::open(&db_path).map_err(|err| eyre!("open {}: {err}", db_path.display()))?;
    if let Some(csv_path) = args.import {
        let count = import_csv(&store, &csv_path, today())?;
        println!("{}", imported_message(count));
        return Ok(());
    }
    let terminal = ratatui::init();
    let result = tui::run(terminal, store, db_path);
    ratatui::restore();
    result
}

struct Args {
    help: bool,
    db: Option<PathBuf>,
    import: Option<PathBuf>,
}

fn parse_args<I>(args: I) -> color_eyre::Result<Args>
where
    I: IntoIterator<Item = String>,
{
    let mut help = false;
    let mut db = None;
    let mut import = None;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "--help" || arg == "-h" {
            help = true;
        } else if let Some(path) = arg.strip_prefix("--db=") {
            if path.is_empty() {
                return Err(eyre!("--db needs a path"));
            }
            db = Some(PathBuf::from(path));
        } else if arg == "--db" {
            let Some(path) = iter.next() else {
                return Err(eyre!("--db needs a path"));
            };
            db = Some(PathBuf::from(path));
        } else if arg == "import" {
            let Some(path) = iter.next().filter(|path| !path.starts_with('-')) else {
                return Err(eyre!("import needs a CSV path"));
            };
            import = Some(PathBuf::from(path));
        } else {
            return Err(eyre!("unknown argument '{arg}'"));
        }
    }
    Ok(Args { help, db, import })
}

fn import_csv(store: &SqliteStore, path: &Path, today: Date) -> color_eyre::Result<usize> {
    let file = std::fs::File::open(path).map_err(|err| eyre!("open {}: {err}", path.display()))?;
    let applications = import::read_applications(file, today)?;
    let count = applications.len();
    store.insert_all(&applications)?;
    Ok(count)
}

fn imported_message(count: usize) -> String {
    if count == 1 {
        "imported 1 application".to_string()
    } else {
        format!("imported {count} applications")
    }
}

fn today() -> Date {
    jiff::ZonedDateTime::now().date()
}

fn default_db_path() -> color_eyre::Result<PathBuf> {
    let dirs = ProjectDirs::from("", "", "job-tracker")
        .ok_or_else(|| eyre!("could not find a local data directory; pass --db <path>"))?;
    Ok(dirs.data_local_dir().join("tracker.db"))
}

fn print_help() {
    println!(
        "\
tracker — local job application tracker

Usage:
  tracker [--db <path>]
  tracker import <file.csv> [--db <path>]

Options:
  --db <path>   SQLite database file. Default:
                %LOCALAPPDATA%\\job-tracker\\data\\tracker.db on Windows
  -h, --help    Show this help

import reads one application per row. Columns are company, role, stage,
and date. stage and date may be blank (applied and today). Other columns
are ignored.

Press ? in the app for keys and commands."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_database_path_is_the_local_data_dir() {
        let path = default_db_path().unwrap();
        let dirs = ProjectDirs::from("", "", "job-tracker").unwrap();
        assert_eq!(path, dirs.data_local_dir().join("tracker.db"));
        let text = path.to_string_lossy().replace('/', "\\");
        assert!(text.ends_with(r"\job-tracker\data\tracker.db"), "{path:?}");
    }

    #[test]
    fn args_accept_db_and_reject_unknown() {
        let args = parse_args(["--db".into(), "file.db".into()]).unwrap();
        assert_eq!(args.db.unwrap(), PathBuf::from("file.db"));
        let args = parse_args(["--db=file.db".into()]).unwrap();
        assert_eq!(args.db.unwrap(), PathBuf::from("file.db"));
        assert!(parse_args(["--db".into()]).is_err());
        assert!(parse_args(["--db=".into()]).is_err());
        assert!(parse_args(["--nope".into()]).is_err());
        assert!(parse_args(["--help".into()]).unwrap().help);
    }

    #[test]
    fn import_accepts_a_csv_path_around_db() {
        let args = parse_args(["import".into(), "apps.csv".into()]).unwrap();
        assert_eq!(args.import.unwrap(), PathBuf::from("apps.csv"));

        let args = parse_args([
            "--db".into(),
            "file.db".into(),
            "import".into(),
            "apps.csv".into(),
        ])
        .unwrap();
        assert_eq!(args.db.unwrap(), PathBuf::from("file.db"));
        assert_eq!(args.import.unwrap(), PathBuf::from("apps.csv"));

        let args = parse_args(["import".into(), "apps.csv".into(), "--db=file.db".into()]).unwrap();
        assert_eq!(args.db.unwrap(), PathBuf::from("file.db"));
        assert_eq!(args.import.unwrap(), PathBuf::from("apps.csv"));

        assert!(parse_args(["import".into()]).is_err());
    }
}
