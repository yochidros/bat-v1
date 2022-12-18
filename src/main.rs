use std::{
    collections::HashMap,
    env,
    io::{self, BufRead, StdoutLock, Write},
    path::Path,
    process,
};

#[macro_use]
extern crate clap;
use ansi_term::{
    Colour::{Fixed, Green, Red, White, Yellow},
    Style,
};
use atty::Stream;
use clap::{Arg, ArgAction, ArgMatches, ColorChoice, Command};
use console::Term;
use git2::{DiffOptions, IntoCString, Repository};
use syntect::util::as_24_bit_terminal_escaped;
use syntect::{
    easy::HighlightFile,
    highlighting::{Theme, ThemeSet},
    parsing::SyntaxSet,
};

#[derive(Copy, Clone, Debug)]
enum LineChange {
    Added,
    RemovedAbove,
    RemovedBelow,
    Modified,
}

type LineChanges = HashMap<u32, LineChange>;

const PANEL_WIDTH: usize = 7;
const GRID_COLOR: u8 = 238;

fn print_horizontal_line(
    handle: &mut StdoutLock,
    grid_char: char,
    term_width: usize,
) -> io::Result<()> {
    let bar = "-".repeat(term_width - (PANEL_WIDTH + 1));
    let line = format!("{}{}{}", "-".repeat(PANEL_WIDTH), grid_char, bar);

    write!(handle, "{}\n", Fixed(GRID_COLOR).paint(line))?;
    Ok(())
}

fn print_file<P: AsRef<Path>>(
    theme: &Theme,
    syntax_set: &SyntaxSet,
    filename: P,
    line_changes: Option<LineChanges>,
) -> io::Result<()> {
    let mut hightlighter = HighlightFile::new(filename.as_ref(), syntax_set, theme)?;

    let stdout = io::stdout();
    let mut handle = stdout.lock();

    let term = Term::stdout();

    let (_, term_width) = term.size();

    let term_width = term_width as usize;

    print_horizontal_line(&mut handle, '┬', term_width)?;

    write!(
        handle,
        "{}{} {}\n",
        " ".repeat(PANEL_WIDTH),
        Fixed(GRID_COLOR).paint("|"),
        White.bold().paint(filename.as_ref().to_string_lossy())
    )?;

    print_horizontal_line(&mut handle, '┼', term_width)?;

    for (idx, maybe_line) in hightlighter.reader.lines().enumerate() {
        let line_nr = idx + 1;
        let line = maybe_line.unwrap_or("<INVALID UTF-8>".into());
        let regions = hightlighter
            .highlight_lines
            .highlight_line(&line, syntax_set)
            .ok()
            .unwrap();

        let line_change = if let Some(ref changes) = line_changes {
            match changes.get(&(line_nr as u32)) {
                Some(&LineChange::Added) => Green.paint("+"),
                Some(&LineChange::RemovedAbove) => Red.paint("‾"),
                Some(&LineChange::RemovedBelow) => Red.paint("_"),
                Some(&LineChange::Modified) => Yellow.paint("~"),
                _ => Style::default().paint(" "),
            }
        } else {
            Style::default().paint(" ")
        };
        write!(
            handle,
            "{} {} {} {}\n",
            Fixed(244).paint(format!("{:4}", line_nr)),
            line_change,
            Fixed(GRID_COLOR).paint("|"),
            as_24_bit_terminal_escaped(&regions, false)
        )?;
    }
    print_horizontal_line(&mut handle, '┴', term_width)?;

    Ok(())
}

fn run(matches: &ArgMatches) -> io::Result<()> {
    let home_dir = env::home_dir().ok_or(io::Error::new(
        io::ErrorKind::Other,
        "Could not get home directory",
    ))?;

    let theme_dir = home_dir.join(".config").join("bat").join("themes");

    let theme_set = ThemeSet::load_from_folder(theme_dir)
        .map_err(|_| io::Error::new(io::ErrorKind::Other, "Could not load themes"))?;
    let theme = &theme_set.themes["Monokai"];

    let syntax_set = SyntaxSet::load_defaults_nonewlines();

    if let Some(files) = matches.get_many::<String>("FILE") {
        for file in files {
            println!("{}", file);
            let line_changes = get_changes(file.clone());
            print_file(theme, &syntax_set, file, line_changes)?;
        }
    }
    Ok(())
}

fn get_changes(filename: String) -> Option<LineChanges> {
    let repo = Repository::open_from_env().ok()?;
    let mut diff_options = DiffOptions::new();
    diff_options.pathspec(filename.into_c_string().ok()?);
    diff_options.context_lines(0);

    let diff = repo
        .diff_index_to_workdir(None, Some(&mut diff_options))
        .ok()?;
    let mut line_changes: LineChanges = HashMap::new();

    let mark_section =
        |line_changes: &mut LineChanges, start: u32, end: i32, change: LineChange| {
            for line in start..(end + 1) as u32 {
                line_changes.insert(line, change);
            }
        };
    let _ = diff.foreach(
        &mut |_, _| true,
        None,
        Some(&mut |_, hunk| {
            let old_lines = hunk.old_lines();
            let new_start = hunk.new_start();
            let new_lines = hunk.new_lines();
            let new_end = (new_start + new_lines) as i32 - 1;

            if old_lines == 0 && new_lines > 0 {
                mark_section(&mut line_changes, new_start, new_end, LineChange::Added);
            } else if new_lines == 0 && old_lines > 0 {
                if new_start <= 0 {
                    mark_section(&mut line_changes, 1, 1, LineChange::RemovedAbove);
                } else {
                    mark_section(
                        &mut line_changes,
                        new_start,
                        new_start as i32,
                        LineChange::RemovedBelow,
                    );
                }
            } else {
                mark_section(&mut line_changes, new_start, new_end, LineChange::Modified);
            }

            true
        }),
        None,
    );
    println!("{:?}", line_changes);

    Some(line_changes)
}

fn main() {
    let clap_color_setting = if atty::is(Stream::Stdout) {
        ColorChoice::Auto
    } else {
        ColorChoice::Never
    };

    let matches = Command::new(crate_name!())
        .version(crate_version!())
        .color(clap_color_setting)
        .about(crate_description!())
        .max_term_width(90)
        .arg(
            Arg::new("FILE")
                .action(ArgAction::Set)
                .num_args(1..)
                .help("File(s) to print"),
        )
        .get_matches();

    let result = run(&matches);

    if let Err(e) = result {
        if e.kind() != io::ErrorKind::BrokenPipe {
            eprintln!("{}: {}", ansi_term::Colour::Red.paint("[bat error]"), e);
            process::exit(1);
        }
    }
}
