//! `mto completions`: the completion scripts for bash, zsh, fish and PowerShell, and where
//! each shell loads them from.
//!
//! The scripts are written from the command line's tables in [`super::spec`] and from the
//! settings table, so a flag, value, command or setting cannot be added without every
//! shell completing it. A script holds no time, path or machine detail: the same version
//! of mto writes the same bytes. Its first line names the version and the shell (zsh's
//! second, after the `#compdef` line zsh needs first), so `status` can tell a current
//! script from a stale one and `uninstall` removes only what mto wrote.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use super::spec::{COMMANDS, CommandSpec, Complete, FLAGS, Flag, FlagSpec, Group, SHELLS, spec_of};
use crate::config::Environment;
use crate::options::{self, File, Kind};
use crate::version;

/// What the PowerShell completer does with its tables.
const POWERSHELL_BODY: &str = r#"    }

    $result = {
        param($text, $tip)
        if ($text.StartsWith($wordToComplete, [System.StringComparison]::OrdinalIgnoreCase)) {
            $shown = $text
            if ($text -match '\s') { $shown = "'" + $text.Replace("'", "''") + "'" }
            if (-not $tip) { $tip = $text }
            [System.Management.Automation.CompletionResult]::new($shown, $text, 'ParameterValue', $tip)
        }
    }
    # Folders, and Rust files, or every file when no Rust file matches; only folders
    # when $kind is dirs.
    $pathsOf = {
        param($before, $typed, $kind)
        $parent = ''
        $cut = $typed.LastIndexOfAny([char[]]@('/', '\'))
        if ($cut -ge 0) { $parent = $typed.Substring(0, $cut + 1) }
        $items = @(Get-ChildItem -Path ($typed + '*') -Force -ErrorAction SilentlyContinue)
        $files = @()
        if ($kind -eq 'files') {
            $all = @($items | Where-Object { -not $_.PSIsContainer })
            $files = @($all | Where-Object { $_.Name -like '*.rs' })
            if ($files.Count -eq 0) { $files = $all }
        }
        foreach ($item in @($items | Where-Object { $_.PSIsContainer })) {
            & $result ($before + $parent + $item.Name + [System.IO.Path]::DirectorySeparatorChar) $item.Name
        }
        foreach ($item in $files) {
            & $result ($before + $parent + $item.Name) $item.Name
        }
    }
    # --config's pairs: the settings, then after `=` the setting's values.
    $settingsOf = {
        param($before, $typed)
        $head = ''
        $last = $typed
        $comma = $typed.LastIndexOf(',')
        if ($comma -ge 0) {
            $head = $typed.Substring(0, $comma + 1)
            $last = $typed.Substring($comma + 1)
        }
        $equals = $last.IndexOf('=')
        if ($equals -ge 0) {
            $key = $last.Substring(0, $equals)
            if ($settings.Contains($key)) {
                foreach ($value in $settings[$key]) {
                    & $result ($before + $head + $key + '=' + $value) $value
                }
            }
        } else {
            foreach ($key in $settings.Keys) { & $result ($before + $head + $key + '=') $key }
        }
    }
    $valueOf = {
        param($flag, $before, $typed)
        if ($values.Contains($flag)) {
            foreach ($value in $values[$flag]) { & $result ($before + $value) $value }
        } elseif ($paths.Contains($flag)) {
            & $pathsOf $before $typed $paths[$flag]
        } elseif ($flag -eq '--config') {
            & $settingsOf $before $typed
        }
    }

    # The command and subcommand given so far, and how many arguments followed them.
    $words = @($commandAst.CommandElements |
        Where-Object { $_.Extent.EndOffset -lt $cursorPosition } |
        ForEach-Object { $_.Extent.Text })
    $command = ''
    $sub = ''
    $operands = 0
    $expect = $null
    for ($i = 1; $i -lt $words.Count; $i++) {
        $word = $words[$i]
        if ($expect) { $expect = $null; continue }
        if ($word -eq '--') { return }
        if ($word -like '--*=*') { continue }
        if ($word.Length -gt 1 -and $word.StartsWith('-')) {
            if ($takesValue -ccontains $word) { $expect = $word }
            continue
        }
        if ($operands -eq 0 -and $command -eq '' -and $commands -ccontains $word) {
            $command = $word
        } elseif ($operands -eq 0 -and $command -ne '' -and $sub -eq '' -and
            $subcommands.Contains($command) -and $subcommands[$command] -ccontains $word) {
            $sub = $word
        } else {
            $operands++
        }
    }

    if ($expect) {
        & $valueOf $expect '' $wordToComplete
        return
    }
    if ($wordToComplete -like '--*=*') {
        $flag = $wordToComplete.Substring(0, $wordToComplete.IndexOf('='))
        & $valueOf $flag ($flag + '=') $wordToComplete.Substring($flag.Length + 1)
        return
    }
    $place = "$command $sub".Trim()
    if ($wordToComplete.StartsWith('-')) {
        foreach ($flag in $flags[$place]) { & $result $flag $help[$flag] }
        return
    }
    if ($command -eq '') {
        if ($operands -eq 0) {
            foreach ($name in $commands) { & $result $name $help[$name] }
        }
        & $pathsOf '' $wordToComplete 'files'
        return
    }
    if ($operands -eq 0 -and $sub -eq '' -and $subcommands.Contains($command)) {
        foreach ($name in $subcommands[$command]) { & $result $name $help["$command $name"] }
    }
    if ($operandWords.Contains($place)) {
        foreach ($value in $operandWords[$place]) { & $result $value $value }
    } elseif ($operandPaths.Contains($place)) {
        & $pathsOf '' $wordToComplete $operandPaths[$place]
    }
}
"#;

/// How the shell was chosen, for the note that names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Found {
  Named,
  Parent,
  ShellVariable,
  PsModulePath,
}

impl Found {
  pub fn how(self) -> &'static str {
    match self {
      Found::Named => "as named",
      Found::Parent => "from the parent process",
      Found::ShellVariable => "from $SHELL",
      Found::PsModulePath => "from $PSModulePath",
    }
  }
}

/// What `mto completions install` did, or would do.
#[derive(Debug, PartialEq, Eq)]
pub enum Installed {
  Wrote,
  Current,
  WouldWrite,
}

/// The shells there are scripts for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shell {
  Bash,
  Zsh,
  Fish,
  PowerShell,
}

impl Shell {
  /// In the order of [`SHELLS`].
  pub const ALL: [Shell; 4] = [Shell::Bash, Shell::Zsh, Shell::Fish, Shell::PowerShell];

  /// The shell `name` names, as [`SHELLS`] spells it.
  pub fn named(name: &str) -> Option<Shell> {
    Shell::ALL.into_iter().find(|shell| shell.name() == name)
  }

  pub fn name(self) -> &'static str {
    match self {
      Shell::Bash => SHELLS[0],
      Shell::Zsh => SHELLS[1],
      Shell::Fish => SHELLS[2],
      Shell::PowerShell => SHELLS[3],
    }
  }

  /// The script's file name in the folder the shell loads completions from.
  pub fn file_name(self) -> &'static str {
    match self {
      Shell::Bash => "mto",
      Shell::Zsh => "_mto",
      Shell::Fish => "mto.fish",
      Shell::PowerShell => "mto.ps1",
    }
  }
}

/// What a script file holds, by its header.
#[derive(Debug, PartialEq, Eq)]
pub enum State {
  Missing,
  /// mto's, for this version.
  Current,
  /// mto's, from another version.
  Stale(String),
  /// Not written by mto.
  Foreign,
}

/// The folders the scripts are installed under, from the environment, so tests can supply
/// their own.
#[derive(Clone, Debug, Default)]
pub struct Folders {
  pub home: Option<PathBuf>,
  /// `$XDG_CONFIG_HOME`, when set to an absolute path.
  pub xdg_config_home: Option<PathBuf>,
  /// `$XDG_DATA_HOME`, when set to an absolute path.
  pub xdg_data_home: Option<PathBuf>,
  /// `$BASH_COMPLETION_USER_DIR`, where bash-completion looks first.
  pub bash_completion_user_dir: Option<PathBuf>,
  /// `%LOCALAPPDATA%` (Windows).
  pub local_app_data: Option<PathBuf>,
  pub windows: bool,
}

impl Folders {
  pub fn from_process() -> Folders {
    let env = Environment::from_process();
    let absolute = |name: &str| {
      std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    };
    Folders {
      home: env.home,
      xdg_config_home: env.xdg_config_home,
      xdg_data_home: absolute("XDG_DATA_HOME"),
      bash_completion_user_dir: absolute("BASH_COMPLETION_USER_DIR"),
      local_app_data: env.local_app_data,
      windows: cfg!(windows),
    }
  }

  /// `$XDG_DATA_HOME`, else `~/.local/share`.
  fn data_home(&self) -> Option<PathBuf> {
    self.xdg_data_home.clone().or_else(|| {
      self
        .home
        .as_ref()
        .map(|home| home.join(".local").join("share"))
    })
  }

  /// The folder `shell` loads completions from, XDG's first on every system, as mto's own
  /// files are kept.
  pub fn folder(&self, shell: Shell) -> Option<PathBuf> {
    match shell {
      Shell::Fish => self
        .xdg_config_home
        .clone()
        .or_else(|| self.home.as_ref().map(|home| home.join(".config")))
        .map(|base| base.join("fish").join("completions")),
      Shell::Bash => match &self.bash_completion_user_dir {
        Some(dir) => Some(dir.join("completions")),
        None => self
          .data_home()
          .map(|base| base.join("bash-completion").join("completions")),
      },
      Shell::Zsh => self
        .data_home()
        .map(|base| base.join("zsh").join("site-functions")),
      Shell::PowerShell => match &self.xdg_data_home {
        Some(base) => Some(base.join("powershell")),
        None if self.windows => self
          .local_app_data
          .as_ref()
          .map(|base| base.join("mtindo").join("completions")),
        None => self.data_home().map(|base| base.join("powershell")),
      },
    }
  }

  /// `path` with the home folder written `~`, as a startup file line can say it.
  fn tilde(&self, path: &Path) -> String {
    match self
      .home
      .as_ref()
      .and_then(|home| path.strip_prefix(home).ok())
    {
      Some(rest) if !self.windows => format!("~/{}", rest.display()),
      _ => path.display().to_string(),
    }
  }
}

/// One place on the command line and the flags that may follow it: before any command
/// (`mto --check`), after a command (`mto history --all`), or after a subcommand
/// (`mto history prune --keep`).
struct Place {
  command: &'static str,
  sub: &'static str,
  flags: Vec<&'static FlagSpec>,
}

/// The shell mto runs in: its parent process when that is a shell, else `$SHELL`, else
/// PowerShell on Windows when `$PSModulePath` is set.
pub fn detect(
  parent: Option<&str>,
  shell_variable: Option<&str>,
  ps_module_path: bool,
  windows: bool,
) -> Option<(Shell, Found)> {
  if let Some(shell) = parent.and_then(shell_of) {
    return Some((shell, Found::Parent));
  }
  if let Some(shell) = shell_variable.and_then(shell_of) {
    return Some((shell, Found::ShellVariable));
  }
  (windows && ps_module_path).then_some((Shell::PowerShell, Found::PsModulePath))
}

/// The first line of every script (zsh's second): the version and build that wrote it,
/// and the shell. A script is current only for that build: a release's matches every
/// install of the release, a development build's only itself.
pub fn header(shell: Shell) -> String {
  format!(
    "# mto {} completions for {}, generated by `mto completions {}`; do not edit.\n",
    version::short(),
    shell.name(),
    shell.name()
  )
}

/// Writes `shell`'s script into `dir`, or the shell's own folder. `Err` holds the message
/// and the exit code.
pub fn install(
  shell: Shell,
  dir: Option<&Path>,
  dry_run: bool,
  folders: &Folders,
) -> Result<(PathBuf, Installed), (String, i32)> {
  let path = target(shell, dir, folders)?;
  let script = script(shell);
  match state_of(&path) {
    State::Foreign => {
      return Err((
        format!(
          "{} exists and mto did not write it; remove it, or pass --dir",
          path.display()
        ),
        3,
      ));
    }
    State::Current if fs::read_to_string(&path).is_ok_and(|text| text == script) => {
      return Ok((path, Installed::Current));
    }
    _ => {}
  }
  if dry_run {
    return Ok((path, Installed::WouldWrite));
  }
  let fail = |e: std::io::Error| (format!("cannot write {}: {}", path.display(), e), 3);
  if let Some(parent) = path.parent() {
    fs::create_dir_all(parent).map_err(fail)?;
  }
  crate::write_atomic(&path, &script).map_err(fail)?;
  Ok((path, Installed::Wrote))
}

/// What to add to the shell's startup file for it to load the script at `path`, if
/// anything: the lines to print under the one naming the file.
pub fn next_step(shell: Shell, path: &Path, dir_given: bool, folders: &Folders) -> String {
  let folder = path.parent().unwrap_or(path);
  let line = |intro: &str, line: String| format!("\n{}\n\n    {}\n", intro, line);
  match shell {
    Shell::Zsh => line(
      "  zsh loads it once that folder is in $fpath. If it is not there yet, add this\n  \
       to ~/.zshrc before `compinit`, then open a new shell:",
      format!("fpath+=({})", folders.tilde(folder)),
    ),
    Shell::Bash => line(
      "  bash loads it by itself when the bash-completion package (version 2) is set up\n  \
       and the file is in its folder. Without it, add this to ~/.bashrc, then open a\n  \
       new shell:",
      format!("source {}", folders.tilde(path)),
    ),
    Shell::Fish if !dir_given => "\n  fish loads it by itself in a new shell.\n".to_string(),
    Shell::Fish => line(
      "  fish loads completions from its own folders only. Add this to\n  \
       ~/.config/fish/config.fish, then open a new shell:",
      format!("source {}", folders.tilde(path)),
    ),
    Shell::PowerShell => line(
      "  PowerShell runs it from your profile. Add this to $PROFILE, then open a new\n  \
       shell:",
      format!(". {}", folders.tilde(path)),
    ),
  }
}

/// The name of the program that started mto, as the system lists it, or `None` when it
/// cannot be known (Windows offers no way to the standard library).
pub fn parent_process_name() -> Option<String> {
  #[cfg(unix)]
  {
    let parent = std::os::unix::process::parent_id().to_string();
    let output = std::process::Command::new("ps")
      .args(["-o", "comm=", "-p", &parent])
      .stdin(std::process::Stdio::null())
      .stderr(std::process::Stdio::null())
      .output()
      .ok()?;
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (output.status.success() && !name.is_empty()).then_some(name)
  }
  #[cfg(not(unix))]
  None
}

/// `shell`'s completion script.
pub fn script(shell: Shell) -> String {
  match shell {
    Shell::Bash => bash(),
    Shell::Zsh => zsh(),
    Shell::Fish => fish(),
    Shell::PowerShell => powershell(),
  }
}

/// What the file at `path` holds, by its header.
pub fn state_of(path: &Path) -> State {
  let Ok(text) = fs::read_to_string(path) else {
    return if path.exists() {
      State::Foreign
    } else {
      State::Missing
    };
  };
  let mut lines = text.lines();
  let mut first = lines.next().unwrap_or("");
  if first == "#compdef mto" {
    first = lines.next().unwrap_or("");
  }
  let version = first
    .strip_prefix("# mto ")
    .and_then(|rest| rest.split_once(" completions for "))
    .filter(|(_, rest)| rest.contains(", generated by `mto completions "))
    .map(|(version, _)| version);
  match version {
    Some(written) if written == version::short() => State::Current,
    Some(version) => State::Stale(version.to_string()),
    None => State::Foreign,
  }
}

/// Where `shell`'s script is, or would be installed: in `dir` when given.
pub fn target(
  shell: Shell,
  dir: Option<&Path>,
  folders: &Folders,
) -> Result<PathBuf, (String, i32)> {
  let folder = match dir {
    Some(dir) => dir.to_path_buf(),
    None => folders.folder(shell).ok_or_else(|| {
      (
        format!(
          "no home folder to install the {} completions under; pass --dir",
          shell.name()
        ),
        2,
      )
    })?,
  };
  Ok(folder.join(shell.file_name()))
}

/// Removes `shell`'s script from `dir`, or the shell's own folder, if mto wrote it.
/// `Ok` holds what it held.
pub fn uninstall(
  shell: Shell,
  dir: Option<&Path>,
  dry_run: bool,
  folders: &Folders,
) -> Result<(PathBuf, State), (String, i32)> {
  let path = target(shell, dir, folders)?;
  let state = state_of(&path);
  if matches!(state, State::Current | State::Stale(_)) && !dry_run {
    fs::remove_file(&path).map_err(|e| (format!("cannot remove {}: {}", path.display(), e), 3))?;
  }
  Ok((path, state))
}

fn bash() -> String {
  let mut out = header(Shell::Bash);
  out.push_str("# Works with bash 3.2 and later, with or without the bash-completion package.\n");
  let takes: Vec<&str> = value_flags().flat_map(names).collect();
  let _ = write!(
    out,
    "
# Whether flag $1 is followed by its value.
_mto_takes_value() {{
  case \"$1\" in
    {}) return 0 ;;
  esac
  return 1
}}

# Adds each word after $1 that, after $1, starts the word being completed.
_mto_words() {{
  local before=\"$1\" word
  shift
  for word in \"$@\"; do
    case \"$before$word\" in
      \"$_mto_cur\"*) COMPREPLY+=(\"$before$word \") ;;
    esac
  done
}}

# Adds the paths starting with $2, after $1: folders, and Rust files, or every file
# when no Rust file matches; only folders when $3 is dirs.
_mto_paths() {{
  local before=\"$1\" path IFS='
'
  local dirs=() files=()
  dirs=($(compgen -d -- \"$2\"))
  if [ \"$3\" != dirs ]; then
    files=($(compgen -f -X '!*.rs' -- \"$2\"))
    if [ ${{#files[@]}} -eq 0 ]; then
      files=($(compgen -f -- \"$2\"))
    fi
  fi
  if [ ${{#dirs[@]}} -gt 0 ]; then
    for path in \"${{dirs[@]}}\"; do
      COMPREPLY+=(\"$before$path/\")
    done
  fi
  if [ ${{#files[@]}} -gt 0 ]; then
    for path in \"${{files[@]}}\"; do
      [ -d \"$path\" ] || COMPREPLY+=(\"$before$path \")
    done
  fi
}}
",
    takes.join("|")
  );
  // --config's pairs.
  let settings = settings();
  let _ = write!(
    out,
    "
# --config's pairs, after $1: the settings, then after `=` the setting's values.
_mto_settings() {{
  local head=\"\" last=\"$2\" key
  case \"$2\" in
    *,*) head=\"${{2%,*}},\"; last=\"${{2##*,}}\" ;;
  esac
  case \"$last\" in
    *=*)
      key=\"${{last%%=*}}\"
      _mto_words \"$1$head$key=\" $(_mto_setting_values \"$key\")
      ;;
    *)
      for key in {}; do
        case \"$1$head$key=\" in
          \"$_mto_cur\"*) COMPREPLY+=(\"$1$head$key=\") ;;
        esac
      done
      ;;
  esac
}}

# The values setting $1 takes, when they are a fixed set.
_mto_setting_values() {{
  case \"$1\" in
",
    wrapped(settings.iter().map(|(key, _)| *key), "        ", " \\")
  );
  for (keys, values) in grouped(&settings) {
    let _ = writeln!(out, "    {}) echo {} ;;", keys.join("|"), values.join(" "));
  }
  out.push_str("  esac\n}\n");
  // Flag values.
  out.push_str(
    "\n# Completes the value of flag $1, after $2; $3 is the value so far.\n_mto_value() {\n  \
     case \"$1\" in\n",
  );
  for spec in value_flags() {
    let action = match spec.completes() {
      Complete::Nothing => continue,
      Complete::Files => "_mto_paths \"$2\" \"$3\"".to_string(),
      Complete::Dirs => "_mto_paths \"$2\" \"$3\" dirs".to_string(),
      Complete::Settings => "_mto_settings \"$2\" \"$3\"".to_string(),
      complete => format!("_mto_words \"$2\" {}", words(complete).join(" ")),
    };
    let _ = writeln!(out, "    {}) {} ;;", names(spec).join("|"), action);
  }
  out.push_str("  esac\n}\n");
  // Flags by place.
  out.push_str(
    "\n# The flags that may follow command $1 and subcommand $2 (either may be empty).\n\
     _mto_flags() {\n  case \"$1 $2\" in\n",
  );
  for place in places() {
    let flags: Vec<&str> = place.flags.iter().copied().flat_map(names).collect();
    let _ = writeln!(
      out,
      "    \"{} {}\") _mto_words \"\" {} ;;",
      place.command,
      place.sub,
      flags.join(" ")
    );
  }
  out.push_str("  esac\n}\n");
  // Subcommands.
  out.push_str("\n# The subcommands of command $1.\n_mto_subcommands() {\n  case \"$1\" in\n");
  for command in COMMANDS.iter().filter(|c| !c.subcommands.is_empty()) {
    let subs: Vec<&str> = command.subcommands.iter().map(|s| s.word).collect();
    let _ = writeln!(
      out,
      "    {}) _mto_words \"\" {} ;;",
      command.name,
      subs.join(" ")
    );
  }
  out.push_str("  esac\n}\n");
  // Operands.
  out.push_str(
    "\n# Completes an argument of command $1 and subcommand $2; $3 is the argument so far.\n\
     _mto_operand() {\n  case \"$1 $2\" in\n",
  );
  let mut operands: Vec<(Complete, Vec<String>)> = Vec::new();
  for (command, sub, complete) in operands_of() {
    let key = format!("\"{} {}\"", command.name, sub);
    match operands.iter_mut().find(|(c, _)| *c == complete) {
      Some((_, keys)) => keys.push(key),
      None => operands.push((complete, vec![key])),
    }
  }
  for (complete, keys) in operands {
    let action = match complete {
      Complete::Nothing => continue,
      Complete::Files => "_mto_paths \"\" \"$3\"".to_string(),
      Complete::Dirs => "_mto_paths \"\" \"$3\" dirs".to_string(),
      Complete::Settings => "_mto_settings \"\" \"$3\"".to_string(),
      complete => format!("_mto_words \"\" {}", words(complete).join(" ")),
    };
    let _ = writeln!(out, "    {}) {} ;;", keys.join("|"), action);
  }
  out.push_str("  esac\n}\n");
  let commands: Vec<&str> = COMMANDS.iter().map(|c| c.name).collect();
  let _ = write!(
    out,
    "
_mto() {{
  local _mto_cur words=() w i n=0 keep=\"\" command=\"\" sub=\"\" operands=0 expect=\"\"
  COMPREPLY=()
  # bash splits `--emit=json` and `max_width=80` at `=`: put them back together.
  for ((i = 1; i <= COMP_CWORD; i++)); do
    w=\"${{COMP_WORDS[i]}}\"
    if [ $n -gt 0 ] && {{ [ \"$w\" = \"=\" ] || [ \"${{words[n-1]%=}}\" != \"${{words[n-1]}}\" ]; }}; then
      words[n-1]=\"${{words[n-1]}}$w\"
    else
      words[n]=\"$w\"
      n=$((n + 1))
    fi
  done
  _mto_cur=\"${{words[n-1]}}\"
  # Readline replaces only what follows the word's last `=`.
  case \"$COMP_WORDBREAKS\" in
    *=*) case \"$_mto_cur\" in *=*) keep=\"${{_mto_cur%=*}}=\" ;; esac ;;
  esac
  # The command and subcommand given so far, and how many arguments followed.
  for ((i = 0; i < n - 1; i++)); do
    w=\"${{words[i]}}\"
    if [ -n \"$expect\" ]; then
      expect=\"\"
      continue
    fi
    case \"$w\" in
      --) return 0 ;;
      --*=*) ;;
      -?*) _mto_takes_value \"$w\" && expect=\"$w\" ;;
      *)
        if [ $operands -eq 0 ] && [ -z \"$command\" ] && _mto_is_command \"$w\"; then
          command=\"$w\"
        elif [ $operands -eq 0 ] && [ -n \"$command\" ] && [ -z \"$sub\" ] \\
          && _mto_is_subcommand \"$command\" \"$w\"; then
          sub=\"$w\"
        else
          operands=$((operands + 1))
        fi
        ;;
    esac
  done
  if [ -n \"$expect\" ]; then
    _mto_value \"$expect\" \"\" \"$_mto_cur\"
  else
    case \"$_mto_cur\" in
      --*=*) _mto_value \"${{_mto_cur%%=*}}\" \"${{_mto_cur%%=*}}=\" \"${{_mto_cur#*=}}\" ;;
      -*) _mto_flags \"$command\" \"$sub\" ;;
      *)
        if [ -z \"$command\" ]; then
          [ $operands -eq 0 ] && _mto_words \"\" {}
          _mto_paths \"\" \"$_mto_cur\"
        else
          if [ $operands -eq 0 ] && [ -z \"$sub\" ]; then
            _mto_subcommands \"$command\"
          fi
          _mto_operand \"$command\" \"$sub\" \"$_mto_cur\"
        fi
        ;;
    esac
  fi
  if [ -n \"$keep\" ]; then
    for ((i = 0; i < ${{#COMPREPLY[@]}}; i++)); do
      COMPREPLY[i]=\"${{COMPREPLY[i]#\"$keep\"}}\"
    done
  fi
  return 0
}}

# Whether $1 is one of mto's commands.
_mto_is_command() {{
  case \"$1\" in
    {}) return 0 ;;
  esac
  return 1
}}

# Whether $2 is a subcommand of command $1.
_mto_is_subcommand() {{
  case \"$1 $2\" in
    {}) return 0 ;;
  esac
  return 1
}}

complete -o nospace -F _mto mto
",
    commands.join(" "),
    commands.join("|"),
    COMMANDS
      .iter()
      .flat_map(|c| c.subcommands.iter().map(move |s| format!("\"{} {}\"", c.name, s.word)))
      .collect::<Vec<_>>()
      .join("|")
  );
  out
}

/// The command named `name`, which the tables hold.
fn command_of(name: &str) -> &'static CommandSpec {
  super::spec::command_named(name).expect("a command of the spec")
}

fn fish() -> String {
  let mut out = header(Shell::Fish);
  out.push_str("# fish loads it from its completions folder.\n");
  let takes: Vec<&str> = value_flags().flat_map(names).collect();
  let commands: Vec<&str> = COMMANDS.iter().map(|c| c.name).collect();
  let subs: Vec<String> = COMMANDS
    .iter()
    .flat_map(|c| {
      c.subcommands
        .iter()
        .map(move |s| format!("'{} {}'", c.name, s.word))
    })
    .collect();
  let _ = write!(
    out,
    "
# Prints the command and the subcommand given so far (- for none), then how many
# arguments followed them (-1 after `--`).
function __mto_state
    set -l takes_value {}
    set -l commands {}
    set -l subcommands {}
    set -l words (commandline -opc)
    set -e words[1]
    set -l command -
    set -l sub -
    set -l operands 0
    set -l expect 0
    for word in $words
        if test $expect = 1
            set expect 0
            continue
        end
        switch $word
            case '--'
                set operands -1
                break
            case '--*=*'
            case '-?*'
                if contains -- $word $takes_value
                    set expect 1
                end
            case '*'
                if test $operands = 0; and test $command = -; and contains -- $word $commands
                    set command $word
                else if test $operands = 0; and test $command != -; and test $sub = -; and contains -- \"$command $word\" $subcommands
                    set sub $word
                else
                    set operands (math $operands + 1)
                end
        end
    end
    printf '%s\\n' $command $sub $operands
end

# Whether the command line is at command $1 and subcommand $2, and with a third
# argument, whether nothing followed them yet.
function __mto_at
    set -l state (__mto_state)
    test $state[1] = $argv[1]; and test $state[2] = $argv[2]; or return 1
    if set -q argv[3]
        test $state[3] = 0
    end
end

# --config's pairs: the settings, then after `=` the setting's values.
function __mto_settings
    set -l typed (string replace -r -- '^--config=' '' (commandline -ct))
    set -l head (string match -r -- '^.*,' $typed)
    set -l last (string replace -r -- '^.*,' '' $typed)
    if string match -q -- '*=*' $last
        set -l key (string replace -r -- '=.*' '' $last)
        for value in (__mto_setting_values $key)
            printf '%s%s=%s\\n' \"$head\" $key $value
        end
    else
        for key in {}
            printf '%s%s=\\n' \"$head\" $key
        end
    end
end

# The values setting $1 takes, when they are a fixed set.
function __mto_setting_values
    switch $argv[1]
",
    wrapped(takes.iter().copied(), "        ", " \\"),
    wrapped(commands.iter().copied(), "        ", " \\"),
    wrapped(subs.iter().map(String::as_str), "        ", " \\"),
    wrapped(settings().iter().map(|(key, _)| *key), "            ", " \\")
  );
  for (keys, values) in grouped(&settings()) {
    let _ = writeln!(
      out,
      "        case {}\n            printf '%s\\n' {}",
      keys.join(" "),
      values.join(" ")
    );
  }
  out.push_str("    end\nend\n\ncomplete -c mto -f\n");
  for place in places() {
    let at = format!("__mto_at {} {}", or_dash(place.command), or_dash(place.sub));
    let _ = writeln!(
      out,
      "\n# {}",
      match (place.command, place.sub) {
        ("", _) => "Before any command: the flags, the commands and the files to format.".into(),
        (command, "") => format!("mto {}", command),
        (command, sub) => format!("mto {} {}", command, sub),
      }
    );
    for spec in &place.flags {
      let mut line = format!("complete -c mto -n '{}'", at);
      if let Some(short) = spec.short {
        let _ = write!(line, " -s {}", &short[1..]);
      }
      let _ = write!(line, " -l {}", &spec.long[2..]);
      if spec.takes_value() {
        line.push_str(&match spec.completes() {
          Complete::Files => " -r -F".to_string(),
          Complete::Dirs => " -x -a '(__fish_complete_directories (commandline -ct))'".to_string(),
          Complete::Settings => " -x -a '(__mto_settings)'".to_string(),
          Complete::Nothing => " -x".to_string(),
          complete => format!(" -x -a {}", fish_quoted(&words(complete).join(" "))),
        });
      }
      let _ = writeln!(line, " -d {}", fish_quoted(spec.help));
      out.push_str(&line);
    }
    let first = format!("{} 0", at);
    match place.command {
      "" => {
        for command in COMMANDS {
          let _ = writeln!(
            out,
            "complete -c mto -n '{}' -a {} -d {}",
            first,
            command.name,
            fish_quoted(command.summary)
          );
        }
        let _ = writeln!(
          out,
          "complete -c mto -n '{}' -a '(__fish_complete_suffix .rs)'",
          at
        );
      }
      name => {
        let command = command_of(name);
        if place.sub.is_empty() {
          for sub in command.subcommands {
            let _ = writeln!(
              out,
              "complete -c mto -n '{}' -a {} -d {}",
              first,
              sub.word,
              fish_quoted(sub.summary)
            );
          }
        }
        let operand = match place.sub {
          "" => command.operand,
          sub => {
            command
              .subcommands
              .iter()
              .find(|s| s.word == sub)
              .expect("a subcommand of the command")
              .operand
          }
        };
        let action = match operand {
          Complete::Nothing => continue,
          Complete::Files => "(__fish_complete_suffix .rs)".to_string(),
          Complete::Dirs => "(__fish_complete_directories (commandline -ct))".to_string(),
          Complete::Settings => "(__mto_settings)".to_string(),
          complete => words(complete).join(" "),
        };
        let _ = writeln!(out, "complete -c mto -n '{}' -a '{}'", at, action);
      }
    }
  }
  out
}

/// `text` in fish's single quotes.
fn fish_quoted(text: &str) -> String {
  format!("'{}'", text.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// Settings that share their values, in the order they first appear: (keys, values).
fn grouped(
  settings: &[(&'static str, Vec<&'static str>)],
) -> Vec<(Vec<&'static str>, Vec<&'static str>)> {
  let mut groups: Vec<(Vec<&str>, Vec<&str>)> = Vec::new();
  for (key, values) in settings.iter().filter(|(_, values)| !values.is_empty()) {
    match groups.iter_mut().find(|(_, v)| v == values) {
      Some((keys, _)) => keys.push(key),
      None => groups.push((vec![key], values.clone())),
    }
  }
  groups
}

/// A flag's spellings: `-q` and `--quiet`, or `--check`.
fn names(spec: &FlagSpec) -> Vec<&'static str> {
  spec.short.into_iter().chain([spec.long]).collect()
}

/// Every command and subcommand, with what its arguments complete to: (command,
/// subcommand or "", completion).
fn operands_of() -> Vec<(&'static CommandSpec, &'static str, Complete)> {
  let mut out = Vec::new();
  for command in COMMANDS {
    out.push((command, "", command.operand));
    for sub in command.subcommands {
      out.push((command, sub.word, sub.operand));
    }
  }
  out
}

fn or_dash(word: &str) -> &str {
  if word.is_empty() { "-" } else { word }
}

fn places() -> Vec<Place> {
  let mut out = vec![Place {
    command: "",
    sub: "",
    flags: FLAGS.iter().filter(|s| s.group != Group::Command).collect(),
  }];
  for command in COMMANDS {
    // Every command and subcommand takes -h.
    let flags = |sub: Option<&str>| -> Vec<&'static FlagSpec> {
      std::iter::once(spec_of(Flag::Help))
        .chain(command.flags_after(sub).into_iter().map(spec_of))
        .collect()
    };
    out.push(Place {
      command: command.name,
      sub: "",
      flags: flags(None),
    });
    for sub in command.subcommands {
      out.push(Place {
        command: command.name,
        sub: sub.word,
        flags: flags(Some(sub.word)),
      });
    }
  }
  out
}

fn powershell() -> String {
  let mut out = header(Shell::PowerShell);
  out.push_str("# Run it from your profile ($PROFILE) to complete mto's arguments.\n");
  let quoted = |text: &str| format!("'{}'", text.replace('\'', "''"));
  let list = |items: Vec<String>| {
    if items.is_empty() {
      "@()".to_string()
    } else {
      format!("@({})", items.join(", "))
    }
  };
  // Hash literals match keys without regard to case, and `-v` is not `-V`: each table
  // is built key by key, matching keys exactly.
  let table = |out: &mut String, comment: &str, name: &str, entries: Vec<(String, String)>| {
    if !comment.is_empty() {
      let _ = writeln!(out, "    # {}", comment);
    }
    let _ = writeln!(
      out,
      "    ${} = [System.Collections.Hashtable]::new([System.StringComparer]::Ordinal)",
      name
    );
    for (key, value) in entries {
      let _ = writeln!(out, "    ${}[{}] = {}", name, quoted(&key), value);
    }
  };
  out.push_str(
    "\nRegister-ArgumentCompleter -Native -CommandName 'mto', 'mto.exe' -ScriptBlock {\n    \
     param($wordToComplete, $commandAst, $cursorPosition)\n\n",
  );
  let flags = places()
    .into_iter()
    .map(|place| {
      let key = format!("{} {}", place.command, place.sub);
      let names = place
        .flags
        .iter()
        .copied()
        .flat_map(names)
        .map(quoted)
        .collect();
      (key.trim().to_string(), list(names))
    })
    .collect();
  table(
    &mut out,
    "The flags that may come before any command (''), after a command, and after a\n    \
     # subcommand.",
    "flags",
    flags,
  );
  let mut help = Vec::new();
  for spec in FLAGS {
    for name in names(spec) {
      help.push((name.to_string(), quoted(spec.help)));
    }
  }
  for command in COMMANDS {
    help.push((command.name.to_string(), quoted(command.summary)));
    for sub in command.subcommands {
      help.push((
        format!("{} {}", command.name, sub.word),
        quoted(sub.summary),
      ));
    }
  }
  table(
    &mut out,
    "What each flag, command and subcommand does.",
    "help",
    help,
  );
  let takes = value_flags().flat_map(names).map(quoted).collect();
  let _ = writeln!(
    out,
    "    # The flags followed by a value, and what the value completes to.\n    \
     $takesValue = {}",
    list(takes)
  );
  let mut values = Vec::new();
  let mut paths = Vec::new();
  for spec in value_flags() {
    for name in names(spec) {
      match spec.completes() {
        complete @ (Complete::Words(_) | Complete::Setting(_)) => {
          let words = words(complete).into_iter().map(quoted).collect();
          values.push((name.to_string(), list(words)));
        }
        Complete::Files => paths.push((name.to_string(), quoted("files"))),
        Complete::Dirs => paths.push((name.to_string(), quoted("dirs"))),
        _ => {}
      }
    }
  }
  table(
    &mut out,
    "A flag's values, when they are a fixed set.",
    "values",
    values,
  );
  table(&mut out, "The flags that take a path.", "paths", paths);
  let commands = COMMANDS.iter().map(|c| quoted(c.name)).collect();
  let _ = writeln!(out, "    $commands = {}", list(commands));
  let subcommands = COMMANDS
    .iter()
    .filter(|c| !c.subcommands.is_empty())
    .map(|command| {
      let subs = command.subcommands.iter().map(|s| quoted(s.word)).collect();
      (command.name.to_string(), list(subs))
    })
    .collect();
  table(
    &mut out,
    "Each command's subcommands.",
    "subcommands",
    subcommands,
  );
  let mut operand_paths = Vec::new();
  let mut operand_words = Vec::new();
  for (command, sub, complete) in operands_of() {
    let key = format!("{} {}", command.name, sub).trim().to_string();
    match complete {
      Complete::Files => operand_paths.push((key, quoted("files"))),
      Complete::Dirs => operand_paths.push((key, quoted("dirs"))),
      Complete::Words(_) | Complete::Setting(_) => {
        let words = words(complete).into_iter().map(quoted).collect();
        operand_words.push((key, list(words)));
      }
      _ => {}
    }
  }
  table(
    &mut out,
    "What a command's arguments complete to, after any subcommand.",
    "operandPaths",
    operand_paths,
  );
  table(&mut out, "", "operandWords", operand_words);
  out.push_str(
    "    # The settings --config takes, and their values when they are a fixed set.\n    \
     $settings = [ordered]@{\n",
  );
  for (key, values) in settings() {
    let values = values.into_iter().map(quoted).collect();
    let _ = writeln!(out, "        {} = {}", quoted(key), list(values));
  }
  out.push_str(POWERSHELL_BODY);
  out
}

/// The settings `--config` takes, with their values when they are a fixed set. The
/// per-user settings belong in the user config, and hidden ones are left out of listings.
fn settings() -> Vec<(&'static str, Vec<&'static str>)> {
  options::all()
    .filter(|s| s.file != File::User && !s.hidden && s.kind != Kind::Internal)
    .map(|s| {
      let values = match s.kind {
        Kind::Bool => vec!["true", "false"],
        Kind::Choice(choices) => choices.iter().map(|c| c.name).collect(),
        _ => Vec::new(),
      };
      (s.name, values)
    })
    .collect()
}

/// The shell a process name or program path names: `zsh`, `-zsh` (a login shell),
/// `/bin/bash`, `pwsh.exe`.
fn shell_of(name: &str) -> Option<Shell> {
  let base = name.trim().rsplit(['/', '\\']).next()?;
  let base = base.trim_start_matches('-');
  let base = base
    .strip_suffix(".exe")
    .or_else(|| base.strip_suffix(".EXE"))
    .unwrap_or(base);
  match base {
    "bash" => Some(Shell::Bash),
    "zsh" => Some(Shell::Zsh),
    "fish" => Some(Shell::Fish),
    "pwsh" | "powershell" => Some(Shell::PowerShell),
    _ => None,
  }
}

/// The flags followed by a value.
fn value_flags() -> impl Iterator<Item = &'static FlagSpec> {
  FLAGS.iter().filter(|spec| spec.takes_value())
}

/// The words a completion offers.
fn words(complete: Complete) -> Vec<&'static str> {
  match complete {
    Complete::Words(words) => words.to_vec(),
    Complete::Setting(name) => options::find(name)
      .map(|setting| setting.choices().iter().map(|c| c.name).collect())
      .unwrap_or_default(),
    _ => Vec::new(),
  }
}

/// `words` joined by spaces, a line at most about 80 columns, each line after `indent`
/// and each but the last ended by `continuation`.
fn wrapped<'a>(words: impl Iterator<Item = &'a str>, indent: &str, continuation: &str) -> String {
  let mut lines: Vec<String> = Vec::new();
  let mut line = String::new();
  for word in words {
    if !line.is_empty() && indent.len() + line.len() + 1 + word.len() > 78 {
      lines.push(std::mem::take(&mut line));
    }
    if !line.is_empty() {
      line.push(' ');
    }
    line.push_str(word);
  }
  lines.push(line);
  let first = lines.remove(0);
  let mut out = first;
  for line in lines {
    let _ = write!(out, "{}\n{}{}", continuation, indent, line);
  }
  out
}

fn zsh() -> String {
  let mut out = format!("#compdef mto\n{}", header(Shell::Zsh));
  out.push_str("# zsh loads it from a folder in $fpath, by compinit.\n");
  let settings = settings();
  let _ = write!(
    out,
    "
# --config's pairs: the settings, then after `=` the setting's values.
_mto_settings() {{
  local -a keys
  keys=(
    {}
  )
  compset -P '*,'
  if compset -P '*='; then
    local key=${{IPREFIX%=}}
    _mto_setting_values ${{key##*[,=]}}
  else
    compadd -S '=' -a keys
  fi
}}

# The values setting $1 takes, when they are a fixed set.
_mto_setting_values() {{
  case $1 in
",
    wrapped(settings.iter().map(|(key, _)| *key), "    ", "")
  );
  for (keys, values) in grouped(&settings) {
    let _ = writeln!(
      out,
      "    ({}) compadd -- {} ;;",
      keys.join("|"),
      values.join(" ")
    );
  }
  out.push_str("  esac\n}\n\n_mto_commands() {\n  local -a commands\n  commands=(\n");
  for command in COMMANDS {
    let _ = writeln!(
      out,
      "    '{}:{}'",
      command.name,
      command.summary.replace('\'', "'\\''")
    );
  }
  out.push_str("  )\n  _describe -t commands command commands\n}\n");
  let rust_files = "_files -g \"*.rs(-.)\"";
  let action = |complete: Complete| match complete {
    Complete::Nothing => " ".to_string(),
    Complete::Files => rust_files.to_string(),
    Complete::Dirs => "_files -/".to_string(),
    Complete::Settings => "_mto_settings".to_string(),
    complete => format!("({})", words(complete).join(" ")),
  };
  for command in COMMANDS {
    let place_flags = |sub: Option<&str>| -> Vec<&'static FlagSpec> {
      std::iter::once(spec_of(Flag::Help))
        .chain(command.flags_after(sub).into_iter().map(spec_of))
        .collect()
    };
    let name = command.name;
    if command.subcommands.is_empty() {
      let _ = write!(
        out,
        "\n_mto_{}() {{\n  _arguments -s : \\\n{}    '*: :{}'\n}}\n",
        name,
        zsh_flags(&place_flags(None), "    "),
        action(command.operand)
      );
      continue;
    }
    out.push_str(&format!(
      "\n_mto_{}_subcommands() {{\n  local -a subcommands\n  subcommands=(\n",
      name
    ));
    for sub in command.subcommands {
      let _ = writeln!(
        out,
        "    '{}:{}'",
        sub.word,
        sub.summary.replace('\'', "'\\''")
      );
    }
    out.push_str("  )\n  _describe -t subcommands subcommand subcommands\n}\n");
    let first = match command.operand {
      Complete::Nothing => format!("_mto_{}_subcommands", name),
      operand => format!(
        "_alternative \\\n        'subcommands:subcommand:_mto_{}_subcommands' \\\n        \
         'arguments:argument:{}'",
        name,
        action(operand)
      ),
    };
    let _ = write!(
      out,
      "
_mto_{}() {{
  local curcontext=$curcontext state state_descr line ret=1
  _arguments -C -s : \\
{}    '1: :->first' \\
    '*:: :->rest' && ret=0
  case $state in
    (first)
      {} && ret=0
      ;;
    (rest)
      case $words[1] in
",
      name,
      zsh_flags(&place_flags(None), "    "),
      first
    );
    for sub in command.subcommands {
      let _ = write!(
        out,
        "        ({})\n          _arguments -s : \\\n{}            '*: :{}' && ret=0\n          ;;\n",
        sub.word,
        zsh_flags(&place_flags(Some(sub.word)), "            "),
        action(sub.operand)
      );
    }
    out.push_str("      esac\n      ;;\n  esac\n  return ret\n}\n");
  }
  let top: Vec<&FlagSpec> = FLAGS.iter().filter(|s| s.group != Group::Command).collect();
  let _ = write!(
    out,
    "
_mto() {{
  local curcontext=$curcontext state state_descr line ret=1
  local -a top
  top=(
{}  )
  _arguments -C -s : $top \\
    '1: :->first' \\
    '*:: :->rest' && ret=0
  case $state in
    (first)
      _alternative \\
        'commands:command:_mto_commands' \\
        'files:file:{}' && ret=0
      ;;
    (rest)
      case $words[1] in
",
    zsh_flags(&top, "    ").replace(" \\\n", "\n"),
    rust_files
  );
  for command in COMMANDS {
    let _ = writeln!(
      out,
      "        ({}) _mto_{} && ret=0 ;;",
      command.name, command.name
    );
  }
  let _ = write!(
    out,
    "        (*) _arguments -s : $top '*:file:{}' && ret=0 ;;
      esac
      ;;
  esac
  return ret
}}

if [ \"$funcstack[1]\" = \"_mto\" ]; then
  _mto \"$@\"
else
  compdef _mto mto
fi
",
    rust_files
  );
  out
}

/// `text` in zsh's single quotes, inside `_arguments`' `[description]`.
fn zsh_described(text: &str) -> String {
  text
    .replace('\\', "\\\\")
    .replace('[', "\\[")
    .replace(']', "\\]")
    .replace('\'', "'\\''")
}

/// The `_arguments` specs of a place's flags, one a line, each indented by `indent`.
fn zsh_flags(flags: &[&FlagSpec], indent: &str) -> String {
  let mut out = String::new();
  for spec in flags {
    let names: Vec<&str> = names(spec);
    let value = if spec.takes_value() {
      let action = match spec.completes() {
        Complete::Nothing => " ".to_string(),
        Complete::Files => "_files".to_string(),
        Complete::Dirs => "_files -/".to_string(),
        Complete::Settings => "_mto_settings".to_string(),
        complete => format!("({})", words(complete).join(" ")),
      };
      let message = spec.value.trim_matches(['<', '>', '[', ']']).to_lowercase();
      let message = if message.is_empty() || !spec.values.is_empty() {
        "value".to_string()
      } else {
        message.replace(':', "\\:")
      };
      format!(":{}:{}", message, action)
    } else {
      String::new()
    };
    let help = zsh_described(spec.help);
    // `--flag=` takes its value after `=` or as the next word; `-f+` right after it or
    // as the next word.
    let spelled: Vec<String> = names
      .iter()
      .map(|name| match (spec.takes_value(), name.starts_with("--")) {
        (false, _) => name.to_string(),
        (true, true) => format!("{}=", name),
        (true, false) => format!("{}+", name),
      })
      .collect();
    let line = if spelled.len() == 1 {
      format!("'{}[{}]{}'", spelled[0], help, value)
    } else {
      // Help ends every other completion; another spelling of a flag is not offered
      // once it is given.
      let exclusive = if spec.flag == Flag::Help {
        "- *".to_string()
      } else {
        names.join(" ")
      };
      format!(
        "'({})'{{{}}}'[{}]{}'",
        exclusive,
        spelled.join(","),
        help,
        value
      )
    };
    let _ = writeln!(out, "{}{} \\", indent, line);
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_bash_words_need_no_quoting() {
    // The bash script puts names and values in `case` patterns and word lists unquoted.
    let plain = |word: &str| {
      !word.is_empty()
        && word
          .chars()
          .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    };
    for (key, values) in settings() {
      assert!(plain(key), "{}", key);
      for value in values {
        assert!(plain(value), "{} = {}", key, value);
      }
    }
    for spec in FLAGS {
      for word in names(spec).into_iter().chain(words(spec.completes())) {
        assert!(plain(word), "{}", word);
      }
    }
  }

  #[test]
  fn test_every_shell_is_in_the_spec_and_back() {
    for (shell, name) in Shell::ALL.into_iter().zip(SHELLS) {
      assert_eq!(shell.name(), *name);
      assert_eq!(Shell::named(name), Some(shell));
    }
  }

  #[test]
  fn test_the_folders_are_xdg_first() {
    let home = PathBuf::from("/home/u");
    let folders = Folders {
      home: Some(home.clone()),
      ..Folders::default()
    };
    let at = |shell| folders.folder(shell).unwrap();
    assert_eq!(at(Shell::Fish), home.join(".config/fish/completions"));
    assert_eq!(
      at(Shell::Bash),
      home.join(".local/share/bash-completion/completions")
    );
    assert_eq!(at(Shell::Zsh), home.join(".local/share/zsh/site-functions"));
    assert_eq!(at(Shell::PowerShell), home.join(".local/share/powershell"));
    let xdg = Folders {
      xdg_config_home: Some(PathBuf::from("/x/config")),
      xdg_data_home: Some(PathBuf::from("/x/data")),
      bash_completion_user_dir: Some(PathBuf::from("/b")),
      local_app_data: Some(PathBuf::from("/w")),
      windows: true,
      ..folders.clone()
    };
    assert_eq!(
      xdg.folder(Shell::Fish).unwrap(),
      PathBuf::from("/x/config/fish/completions")
    );
    assert_eq!(
      xdg.folder(Shell::Bash).unwrap(),
      PathBuf::from("/b/completions")
    );
    assert_eq!(
      xdg.folder(Shell::Zsh).unwrap(),
      PathBuf::from("/x/data/zsh/site-functions")
    );
    assert_eq!(
      xdg.folder(Shell::PowerShell).unwrap(),
      PathBuf::from("/x/data/powershell")
    );
    let windows = Folders {
      xdg_data_home: None,
      ..xdg
    };
    assert_eq!(
      windows.folder(Shell::PowerShell).unwrap(),
      PathBuf::from("/w/mtindo/completions")
    );
  }

  #[test]
  fn test_the_header_names_the_version_and_shell() {
    for shell in Shell::ALL {
      let script = script(shell);
      let mut lines = script.lines();
      let mut first = lines.next().unwrap();
      if shell == Shell::Zsh {
        assert_eq!(first, "#compdef mto");
        first = lines.next().unwrap();
      }
      assert_eq!(format!("{}\n", first), header(shell));
      assert!(script.ends_with('\n'));
      assert_eq!(script, super::script(shell), "the same bytes every time");
    }
  }

  #[test]
  fn test_the_shell_is_found_in_order() {
    let found = |parent, variable, ps| detect(parent, variable, ps, false);
    assert_eq!(
      found(Some("-zsh"), Some("/bin/bash"), false),
      Some((Shell::Zsh, Found::Parent))
    );
    assert_eq!(
      found(Some("/opt/homebrew/bin/fish"), None, false),
      Some((Shell::Fish, Found::Parent))
    );
    assert_eq!(
      found(Some("cargo"), Some("/bin/bash"), false),
      Some((Shell::Bash, Found::ShellVariable))
    );
    assert_eq!(found(Some("cargo"), Some("/bin/sh"), true), None);
    assert_eq!(
      detect(None, None, true, true),
      Some((Shell::PowerShell, Found::PsModulePath))
    );
    assert_eq!(shell_of("pwsh.exe"), Some(Shell::PowerShell));
    assert_eq!(
      shell_of("C:\\Windows\\powershell.exe"),
      Some(Shell::PowerShell)
    );
  }
}
