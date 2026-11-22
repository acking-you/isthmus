//! The `isthmus` command: Isthmus without the app.
//!
//! ```text
//!                        +-> devices   keyboards, mice, trackpads (read-only)
//!                        +-> status    data dir, settings, lock, journal, Accessibility
//! process args -> clap --+-> preset    a one-click preset, written to settings.json
//!                        +-> settings  the settings file
//!                        +-> run       the service in this process until Ctrl-C
//!                        +-> restore   put back what a crashed run left changed
//!                        +-> inspect   raw input events until Ctrl-C
//! ```
//!
//! Each command is a module. `devices`, `status` and `settings` only read
//! (status probes the instance lock for an instant); `preset` writes the
//! settings file and nothing else, so a running Isthmus sees it only after
//! a restart; `run`, `restore` and `inspect` touch the system. Exit codes: 0
//! done, 1 failed (`isthmus: <step>: <cause>…` on stderr), 2 usage.

mod devices;
mod error;
mod inspect;
mod output;
mod restore;
mod run;
mod settings_file;
mod signals;
mod status;
#[cfg(test)]
mod test_support;

use std::io::IsTerminal;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use isthmus_core::paths;
use isthmus_input::{Preset, PresetTarget};
use snafu::ResultExt;
use tracing_subscriber::EnvFilter;

use crate::error::{CliError, DataDirSnafu, chain};
use crate::output::print_json;

#[derive(Debug, Parser)]
#[command(
    name = "isthmus",
    author = "L_B__",
    version,
    about = "Windows keyboard and mouse habits on a Mac, from the command line",
    after_help = "ISTHMUS_DATA_DIR moves the data directory (settings, journal and \
                  instance lock), for development. RUST_LOG sets the log level.",
    subcommand_required = true,
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// List keyboards, mice and trackpads as Isthmus sees them (read-only).
    Devices {
        /// Print JSON instead of a table.
        #[arg(long)]
        json: bool,
    },
    /// Show the data directory, the settings, whether an Isthmus is
    /// running, the journal and the Accessibility grant.
    Status {
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Write a one-click preset into the settings file. A running Isthmus
    /// picks it up when it restarts; the app applies presets at once.
    Preset {
        #[arg(value_enum)]
        preset: PresetArg,
        /// What the preset applies to.
        #[arg(long, value_enum, default_value_t = TargetArg::All)]
        target: TargetArg,
    },
    /// Print the settings file.
    Settings {
        /// Print the settings as JSON, normalized as Isthmus applies them.
        #[arg(long)]
        json: bool,
    },
    /// Run Isthmus in this process, without the app, until Ctrl-C, which
    /// puts the keyboard and mouse back first.
    Run {
        /// Apply this preset to everything after starting; it is saved to
        /// the settings file, as the app's one-click switch saves it.
        #[arg(long, value_enum)]
        preset: Option<PresetArg>,
    },
    /// Put back what a previous run left changed (after a crash or a forced
    /// quit), without starting anything.
    Restore,
    /// Print raw key, modifier, button and wheel events until Ctrl-C.
    Inspect,
}

impl Command {
    /// The log level when `RUST_LOG` says nothing: the long-running commands
    /// narrate, the quick ones stay quiet unless something goes wrong.
    fn default_log_level(&self) -> &'static str {
        match self {
            Command::Run { .. } | Command::Restore | Command::Inspect => "info",
            Command::Devices { .. }
            | Command::Status { .. }
            | Command::Preset { .. }
            | Command::Settings { .. } => "warn",
        }
    }
}

/// A one-click preset, as typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum PresetArg {
    /// Windows habits: Ctrl as Command, the Windows shortcuts, the Windows
    /// wheel and pointer. Also switches Isthmus on.
    Windows,
    /// macOS as it was, with Isthmus left on but changing nothing.
    Mac,
}

impl From<PresetArg> for Preset {
    fn from(preset: PresetArg) -> Self {
        match preset {
            PresetArg::Windows => Preset::Windows,
            PresetArg::Mac => Preset::Mac,
        }
    }
}

/// What a preset applies to, as typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum TargetArg {
    All,
    Keyboard,
    Mouse,
}

impl From<TargetArg> for PresetTarget {
    fn from(target: TargetArg) -> Self {
        match target {
            TargetArg::All => PresetTarget::All,
            TargetArg::Keyboard => PresetTarget::Keyboard,
            TargetArg::Mouse => PresetTarget::Mouse,
        }
    }
}

fn main() -> ExitCode {
    // A usage error exits with 2 here, `--help` and `--version` with 0.
    let cli = Cli::parse();
    init_logging(cli.command.default_log_level());
    match execute(cli.command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("isthmus: {}", chain(&error));
            ExitCode::FAILURE
        }
    }
}

fn execute(command: Command) -> Result<(), CliError> {
    match command {
        Command::Devices { json } => devices::run(json),
        Command::Status { json } => status::run(json),
        Command::Preset { preset, target } => apply_preset(preset, target),
        Command::Settings { json } => show_settings(json),
        Command::Run { preset } => run::run(preset.map(Preset::from)),
        Command::Restore => restore::run(),
        Command::Inspect => inspect::run(),
    }
}

/// `isthmus preset`.
fn apply_preset(preset: PresetArg, target: TargetArg) -> Result<(), CliError> {
    let dir = paths::data_dir().context(DataDirSnafu)?;
    let file = settings_file::apply_preset(&dir, preset.into(), target.into())?;
    let preset = match preset {
        PresetArg::Windows => "Windows",
        PresetArg::Mac => "Mac",
    };
    let target = match target {
        TargetArg::All => "the keyboard and mouse",
        TargetArg::Keyboard => "the keyboard",
        TargetArg::Mouse => "the mouse",
    };
    println!(
        "Applied the {preset} preset to {target} in {}.",
        file.path.display()
    );
    println!(
        "A running Isthmus keeps the settings it started with: restart it, or choose \
         the preset in the app."
    );
    Ok(())
}

/// `isthmus settings`.
fn show_settings(json: bool) -> Result<(), CliError> {
    let dir = paths::data_dir().context(DataDirSnafu)?;
    let file = settings_file::load(&dir)?;
    if json {
        print_json(&file.settings)
    } else {
        print!("{}", settings_file::describe(&file));
        Ok(())
    }
}

/// Logs go to stderr, so `--json` output on stdout stays parseable.
fn init_logging(default_level: &str) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    /// The clap definition is consistent (no clashing names or bad
    /// defaults), which clap only checks when asked.
    #[test]
    fn the_command_line_is_well_formed() {
        Cli::command().debug_assert();
    }

    /// The documented forms parse; a preset outside the two is a usage
    /// error (exit 2), not a silent default.
    #[test]
    fn presets_and_targets_parse_as_documented() {
        let cli =
            Cli::try_parse_from(["isthmus", "preset", "windows", "--target", "mouse"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Preset {
                preset: PresetArg::Windows,
                target: TargetArg::Mouse
            }
        ));
        let cli = Cli::try_parse_from(["isthmus", "preset", "mac"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Preset {
                target: TargetArg::All,
                ..
            }
        ));
        let cli = Cli::try_parse_from(["isthmus", "run", "--preset", "windows"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Run {
                preset: Some(PresetArg::Windows)
            }
        ));
        let error = Cli::try_parse_from(["isthmus", "preset", "linux"]).unwrap_err();
        assert_eq!(error.exit_code(), 2);
    }
}
