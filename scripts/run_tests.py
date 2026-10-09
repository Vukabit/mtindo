#!/usr/bin/env python3
"""Runs mtindo's gates and test suites, and reports each one as passed, failed or skipped.

Usage:
    scripts/run_tests.py                     gates + tests (the default)
    scripts/run_tests.py --full              everything: gates, tests, heavy, fuzz, bench
    scripts/run_tests.py --group heavy       only the given group (repeatable)
    scripts/run_tests.py --target snapshots  only one task, by id (see --list)
    scripts/run_tests.py --bless             regenerate regression fixtures, then show what changed
    scripts/run_tests.py --list              show the plan without running anything
    scripts/run_tests.py -h                  every group, task id, flag and environment variable

Exit codes: 0 when nothing failed (skips do not fail a run), 1 when anything failed,
2 for bad arguments, 130 when interrupted.
"""

import argparse
import os
import re
import pathlib
import shutil
import subprocess
import sys
import threading
import time

project_root = pathlib.Path(__file__).parent.parent.resolve()
sys.path.insert(0, str(project_root))
os.chdir(project_root)

from scripts.common.logger import FileLogger

IS_TTY = sys.stdout.isatty()

COLOR_RED = "\033[91m"
COLOR_GREEN = "\033[92m"
COLOR_YELLOW = "\033[93m"
COLOR_CYAN = "\033[96m"
COLOR_DIM = "\033[90m"
COLOR_RESET = "\033[0m"
COLOR_BOLD = "\033[1m"

if IS_TTY == False:
    COLOR_RED = ""
    COLOR_GREEN = ""
    COLOR_YELLOW = ""
    COLOR_CYAN = ""
    COLOR_DIM = ""
    COLOR_RESET = ""
    COLOR_BOLD = ""

GROUPS = ["gates", "tests", "heavy", "fuzz", "bench"]
DEFAULT_GROUPS = ["gates", "tests"]

# Shown by -h. Every group must have an entry; the help gate checks it.
GROUP_INFO = {
    "gates": "Repository rules: clippy, dependencies, layering, rustfmt, mto on itself, help text",
    "tests": "Every test suite in tests/, plus library, binary and xtask unit tests",
    "heavy": "15 real-world crates must still compile after formatting; needs git and network access",
    "fuzz": "Each fuzz target for --fuzz-time seconds; needs nightly Rust and cargo-fuzz",
    "bench": "Criterion benchmarks; slow, and report numbers rather than pass/fail",
}

ENV_BLESS = "MTINDO_BLESS"
ENV_HEAVY = "MTINDO_HEAVY_TESTS"

# Environment variables the runner sets for tasks. Shown by -h.
ENV_VARS = [
    (ENV_BLESS, "set to 1 for the snapshots task with --bless: rewrite expected outputs"),
    (ENV_HEAVY, "set to 1 for domain_13_rw: run the real-world crate check instead of skipping it"),
]

DEFAULT_FUZZ_TIME = 10

# Human titles for the test files, in run order. A test file missing from this table
# still runs (under its file name) and triggers a warning, so nothing is skipped silently.
TEST_REGISTRY = [
    ("lib", "Library and binary unit tests", "tests"),
    ("xtask", "xtask unit tests: dependency and layering checks", "tests"),
    ("domain_1_unit", "Unit: config values, imports, item metadata", "tests"),
    ("domain_2_golden", "Golden layouts", "tests"),
    ("snapshots", "Regression fixtures", "tests"),
    ("integration", "Block extraction", "tests"),
    ("domain_3_proptest", "Property tests: no panics, no lost lines, idempotence", "tests"),
    ("domain_6_io", "File I/O: backups, atomic writes, permissions, symlinks", "tests"),
    ("domain_7_ecosystem", "Config lookup and ignored directories", "tests"),
    ("domain_8_cli", "Command line: check, dry run, exit codes, stdin", "tests"),
    ("domain_9_config_hierarchy", "Config hierarchy", "tests"),
    ("domain_10_polyrepo", "Multiple repos, rollback, symlink cycles", "tests"),
    ("domain_11_matrix", "Every settings combination is idempotent", "tests"),
    ("domain_12_scale", "500-file repository still compiles after formatting", "tests"),
    ("domain_13_rw", "15 real crates still compile after formatting (network)", "heavy"),
]

FAILURE_TAIL_LINES = 60
STATE_FILE = pathlib.Path("scripts") / "logs" / ".last_success"

logger = FileLogger("run_tests")
current_process = None
tool_cache = {}


class Task:
    def __init__(self, task_id, title, group, command, env=None, requires=None):
        self.task_id = task_id
        self.title = title
        self.group = group
        self.command = command
        self.env = env
        self.requires = requires
        self.status = "PENDING"
        self.seconds = 0.0
        self.note = ""
        self.output = ""


class Spinner:
    def __init__(self, message):
        self.message = message
        self.is_running = False
        self.thread = None
        self.frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]

    def spin(self):
        idx = 0
        while self.is_running == True:
            sys.stdout.write("\r\033[K" + COLOR_CYAN + self.frames[idx] + COLOR_RESET + " " + self.message)
            sys.stdout.flush()
            idx = (idx + 1) % len(self.frames)
            time.sleep(0.1)

    def start(self):
        if IS_TTY == False:
            return
        self.is_running = True
        self.thread = threading.Thread(target=self.spin, daemon=True)
        self.thread.start()

    def stop(self):
        if self.thread == None:
            return
        self.is_running = False
        self.thread.join()
        sys.stdout.write("\r\033[K")
        sys.stdout.flush()


def run_process(command, env_extra):
    global current_process
    env = dict(os.environ)
    if env_extra != None:
        env.update(env_extra)
    current_process = subprocess.Popen(
        command,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        env=env,
    )
    output = current_process.communicate()[0]
    code = current_process.returncode
    current_process = None
    return code, output


def tool_available(tool):
    if tool in tool_cache:
        return tool_cache[tool]
    available = False
    if tool == "git":
        available = shutil.which("git") != None
    elif tool == "rustfmt":
        available = run_process(["cargo", "fmt", "--version"], None)[0] == 0
    elif tool == "clippy":
        available = run_process(["cargo", "clippy", "--version"], None)[0] == 0
    elif tool == "cargo-fuzz":
        available = run_process(["cargo", "+nightly", "fuzz", "--version"], None)[0] == 0
    elif tool == "rustc":
        available = shutil.which("rustc") != None
    tool_cache[tool] = available
    logger.info("Tool check: " + tool + " available=" + str(available))
    return available


def check_help_text():
    """Fails if -h leaves out any group, task id, flag or environment variable."""
    code, output = run_process([sys.executable, "scripts/run_tests.py", "-h"], None)
    if code != 0:
        return code, output
    missing = []
    for group in GROUPS:
        if group not in GROUP_INFO:
            missing.append("group " + group + " has no GROUP_INFO entry")
        if re.search(r"^\s+" + re.escape(group) + r"\s", output, re.MULTILINE) == None:
            missing.append("group " + group)
    for task in build_tasks(HelpDefaults(), False):
        if re.search(r"^\s+" + re.escape(task.task_id) + r"\s", output, re.MULTILINE) == None:
            missing.append("task " + task.task_id)
    for action in build_parser()._actions:
        for option in action.option_strings:
            if option not in output:
                missing.append("flag " + option)
    for entry in ENV_VARS:
        if entry[0] not in output:
            missing.append("environment variable " + entry[0])
    if len(missing) == 0:
        return 0, "-h lists every group, task, flag and environment variable."
    return 1, "Missing from -h:\n  " + "\n  ".join(missing)


def build_tasks(args, announce=True):
    tasks = []

    tasks.append(Task("clippy", "Clippy, warnings are errors", "gates",
                      ["cargo", "clippy", "--workspace", "--all-targets", "--quiet", "--", "-D", "warnings"],
                      requires=["clippy"]))
    tasks.append(Task("deps", "No runtime dependencies", "gates",
                      ["cargo", "xtask", "gates", "deps"]))
    tasks.append(Task("layering", "Modules only use lower layers", "gates",
                      ["cargo", "xtask", "gates", "layering"]))
    tasks.append(Task("rustfmt", "rustfmt agrees with the repo layout", "gates",
                      ["cargo", "fmt", "--all", "--check"], requires=["rustfmt"]))
    tasks.append(Task("self-format", "mto formats itself", "gates",
                      ["cargo", "run", "--quiet", "--bin", "mto", "--", "--check", "."]))
    if pathlib.Path("fuzz").exists() == True:
        tasks.append(Task("fuzz-build", "Fuzz crate compiles", "gates",
                          ["cargo", "check", "--quiet", "--manifest-path", "fuzz/Cargo.toml"]))
    tasks.append(Task("help", "-h lists every group, task, flag and variable", "gates", check_help_text))

    tasks.append(Task("build", "Build test binaries", "tests",
                      ["cargo", "test", "--workspace", "--no-run", "--quiet"]))

    test_files = []
    tests_dir = pathlib.Path("tests")
    if tests_dir.exists() == True:
        for file_path in sorted(tests_dir.iterdir()):
            if file_path.suffix == ".rs":
                test_files.append(file_path.stem)

    registered = []
    for entry in TEST_REGISTRY:
        task_id = entry[0]
        registered.append(task_id)
        if task_id == "lib":
            command = ["cargo", "test", "--lib", "--bins", "--quiet"]
        elif task_id == "xtask":
            command = ["cargo", "test", "--package", "xtask", "--quiet"]
        elif task_id in test_files:
            command = ["cargo", "test", "--test", task_id, "--quiet"]
        else:
            continue
        env = None
        requires = None
        if task_id == "snapshots":
            requires = ["rustc"]
            if args.bless == True:
                env = {ENV_BLESS: "1"}
        if task_id == "domain_13_rw":
            env = {ENV_HEAVY: "1"}
            requires = ["git"]
        tasks.append(Task(task_id, entry[1], entry[2], command, env=env, requires=requires))

    for task_id in test_files:
        if task_id not in registered:
            if announce == True:
                warning = "tests/" + task_id + ".rs is not in TEST_REGISTRY; running it under its file name."
                sys.stdout.write(COLOR_YELLOW + "warning: " + COLOR_RESET + warning + "\n")
                logger.info("Warning: " + warning)
            tasks.append(Task(task_id, task_id, "tests", ["cargo", "test", "--test", task_id, "--quiet"]))

    fuzz_dir = pathlib.Path("fuzz") / "fuzz_targets"
    if fuzz_dir.exists() == True:
        for file_path in sorted(fuzz_dir.iterdir()):
            if file_path.suffix == ".rs":
                target = file_path.stem
                command = ["cargo", "+nightly", "fuzz", "run", target, "--", "-max_total_time=" + str(args.fuzz_time)]
                tasks.append(Task(target, "Fuzz " + target + " for " + str(args.fuzz_time) + "s", "fuzz",
                                  command, requires=["cargo-fuzz"]))

    if pathlib.Path("benches").exists() == True:
        tasks.append(Task("bench", "Benchmarks", "bench", ["cargo", "bench", "--quiet"]))

    return tasks


def select_tasks(tasks, args):
    groups = DEFAULT_GROUPS
    explicit = False
    if args.full == True:
        groups = GROUPS
    elif args.group != None and len(args.group) > 0:
        groups = args.group
        explicit = True

    for task in tasks:
        if args.target != "":
            if task.task_id == args.target:
                task.status = "PENDING"
            else:
                task.status = "EXCLUDED"
        elif task.group in groups:
            task.status = "PENDING"
        elif explicit == True or task.group in DEFAULT_GROUPS:
            task.status = "EXCLUDED"
        else:
            task.status = "SKIP"
            task.note = "use --group " + task.group + " or --full"
    return groups


def run_task(task):
    if task.requires != None:
        for tool in task.requires:
            if tool_available(tool) == False:
                task.status = "SKIP"
                task.note = tool + " is not available"
                return

    logger.info("Running [" + task.task_id + "]: " + describe_command(task))
    spinner = Spinner(task.title)
    spinner.start()
    started = time.time()
    if callable(task.command) == True:
        code, output = task.command()
    else:
        code, output = run_process(task.command, task.env)
    task.seconds = time.time() - started
    spinner.stop()

    task.output = output
    logger.debug("Output of [" + task.task_id + "]:\n" + output)
    if code == 0:
        task.status = "PASS"
        logger.info("Passed [" + task.task_id + "] in " + format_seconds(task.seconds))
    else:
        task.status = "FAIL"
        task.note = "exit code " + str(code)
        logger.error("Failed [" + task.task_id + "] with exit code " + str(code))


def describe_command(task):
    if callable(task.command) == True:
        return "(built-in check)"
    text = " ".join(task.command)
    if task.env != None:
        for key in task.env:
            text = key + "=" + task.env[key] + " " + text
    return text


def format_seconds(seconds):
    return str(round(seconds, 2)) + "s"


def print_task_line(task):
    if task.status == "PASS":
        mark = COLOR_GREEN + "✓" + COLOR_RESET
    elif task.status == "FAIL":
        mark = COLOR_RED + "✗" + COLOR_RESET
    else:
        mark = COLOR_YELLOW + "-" + COLOR_RESET
    line = mark + " " + task.title
    if task.status == "PASS" or task.status == "FAIL":
        line = line + COLOR_DIM + " (" + format_seconds(task.seconds) + ")" + COLOR_RESET
    if task.status == "SKIP":
        line = line + COLOR_DIM + " (skipped: " + task.note + ")" + COLOR_RESET
    sys.stdout.write(line + "\n")

    if task.status == "FAIL" and task.output.strip() != "":
        lines = task.output.rstrip().splitlines()
        if len(lines) > FAILURE_TAIL_LINES:
            hidden = len(lines) - FAILURE_TAIL_LINES
            sys.stdout.write(COLOR_DIM + "    ... " + str(hidden) + " earlier lines in scripts/logs/run_tests.log" + COLOR_RESET + "\n")
            lines = lines[-FAILURE_TAIL_LINES:]
        for output_line in lines:
            sys.stdout.write("    " + output_line + "\n")
    sys.stdout.flush()


def git_output(git_args):
    if tool_available("git") == False:
        return 1, ""
    return run_process(["git"] + git_args, None)


def current_commit():
    code, output = git_output(["rev-parse", "HEAD"])
    if code != 0:
        return ""
    return output.strip()


def tree_is_dirty():
    code, output = git_output(["status", "--porcelain"])
    if code != 0:
        return True
    return output.strip() != ""


def last_good_commit():
    if STATE_FILE.exists() == False:
        return ""
    return STATE_FILE.read_text(encoding="utf-8").strip()


def record_success(commit, dirty):
    if commit == "":
        return
    if dirty == True:
        logger.info("All tasks passed, but the working tree has uncommitted changes; known-good commit not updated.")
        return
    STATE_FILE.parent.mkdir(parents=True, exist_ok=True)
    STATE_FILE.write_text(commit + "\n", encoding="utf-8")
    logger.info("Recorded known-good commit " + commit)


def print_changes_since_last_good(commit, dirty):
    last_good = last_good_commit()
    if last_good == "":
        return
    if last_good == commit and dirty == False:
        sys.stdout.write("\nThis commit passed before with a clean tree; the failure likely comes from the environment.\n")
        return
    code, stat = git_output(["diff", "--stat", last_good])
    if code != 0:
        return
    code_untracked, untracked = git_output(["ls-files", "--others", "--exclude-standard"])
    sys.stdout.write("\n" + COLOR_YELLOW + "Changes since the last passing commit (" + last_good[:7] + "), including uncommitted work:" + COLOR_RESET + "\n")
    sys.stdout.write(stat.rstrip() + "\n")
    if code_untracked == 0 and untracked.strip() != "":
        count = len(untracked.strip().splitlines())
        sys.stdout.write("  plus " + str(count) + " untracked file(s)\n")
    full_code, full_diff = git_output(["diff", last_good])
    if full_code == 0:
        logger.error("Full diff against " + last_good[:7] + ":\n" + full_diff)
    sys.stdout.write(COLOR_DIM + "Full diff in scripts/logs/run_tests.log" + COLOR_RESET + "\n")


def print_blessed_changes():
    code, output = git_output(["status", "--short", "--", "tests/fixtures"])
    sys.stdout.write("\n" + COLOR_YELLOW + "--bless rewrote the expected outputs. Review every change before committing:" + COLOR_RESET + "\n")
    if code == 0 and output.strip() != "":
        sys.stdout.write(output.rstrip() + "\n")
    else:
        sys.stdout.write("  no fixture changed\n")


def print_plan(tasks, groups):
    sys.stdout.write(COLOR_BOLD + "Plan (groups: " + ", ".join(groups) + ")" + COLOR_RESET + "\n")
    for task in tasks:
        if task.status == "PENDING":
            marker = COLOR_GREEN + "run " + COLOR_RESET
        elif task.status == "SKIP":
            marker = COLOR_YELLOW + "skip" + COLOR_RESET
        else:
            marker = COLOR_DIM + "off " + COLOR_RESET
        sys.stdout.write("  " + marker + "  " + task.group.ljust(6) + "  " + task.task_id.ljust(26) + task.title + "\n")


def print_summary(tasks, elapsed):
    passed = 0
    failed = 0
    skipped = 0
    for task in tasks:
        if task.status == "PASS":
            passed = passed + 1
        elif task.status == "FAIL":
            failed = failed + 1
        elif task.status == "SKIP":
            skipped = skipped + 1

    sys.stdout.write("========================================\n")
    counts = str(passed) + " passed, " + str(failed) + " failed, " + str(skipped) + " skipped in " + format_seconds(elapsed)
    if failed == 0:
        sys.stdout.write(COLOR_GREEN + COLOR_BOLD + "SUCCESS: " + counts + COLOR_RESET + "\n")
    else:
        sys.stdout.write(COLOR_RED + COLOR_BOLD + "FAILURE: " + counts + COLOR_RESET + "\n")
        for task in tasks:
            if task.status == "FAIL":
                sys.stdout.write("  " + COLOR_RED + "✗" + COLOR_RESET + " " + task.task_id + "  (rerun: scripts/run_tests.py --target " + task.task_id + ")\n")
    logger.info("Run complete: " + counts)
    return failed


class HelpDefaults:
    """Stands in for parsed arguments when listing tasks for -h."""
    bless = False
    fuzz_time = DEFAULT_FUZZ_TIME


def help_epilog():
    lines = []
    lines.append("groups (default: " + ", ".join(DEFAULT_GROUPS) + "):")
    for group in GROUPS:
        lines.append("  " + group.ljust(7) + GROUP_INFO.get(group, ""))
    lines.append("")
    lines.append("tasks (id, group, what it checks; run one with --target ID):")
    for task in build_tasks(HelpDefaults(), False):
        lines.append("  " + task.task_id.ljust(26) + task.group.ljust(7) + task.title)
    lines.append("")
    lines.append("environment variables the runner sets:")
    for entry in ENV_VARS:
        lines.append("  " + entry[0].ljust(20) + entry[1])
    lines.append("")
    lines.append("files:")
    lines.append("  scripts/logs/run_tests.log   full output of every task")
    lines.append("  scripts/logs/.last_success   last commit that passed a default or full run on a clean tree")
    lines.append("")
    lines.append("exit codes:")
    lines.append("  0 nothing failed (skips never fail a run), 1 something failed, 2 bad arguments, 130 interrupted")
    return "\n".join(lines)


def build_parser():
    parser = argparse.ArgumentParser(
        description="Run mtindo's gates and test suites; report each as passed, failed or skipped with a reason.",
        epilog=help_epilog(),
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("--group", action="append", choices=GROUPS,
                        help="Run only this group; repeatable. Default: gates and tests.")
    parser.add_argument("--full", action="store_true",
                        help="Run every group, including heavy (network), fuzz (nightly) and bench.")
    parser.add_argument("--target", type=str, default="", metavar="ID",
                        help="Run one task by id, whatever its group (ids are listed below).")
    parser.add_argument("--fuzz-time", type=int, default=DEFAULT_FUZZ_TIME, metavar="SECONDS",
                        help="Seconds to fuzz each target. Default: " + str(DEFAULT_FUZZ_TIME) + ".")
    parser.add_argument("--bless", action="store_true",
                        help="Regenerate the regression fixtures' expected outputs, then list what changed.")
    parser.add_argument("--list", action="store_true",
                        help="Show which tasks would run, without running them.")
    return parser


def parse_arguments():
    return build_parser().parse_args()


def main():
    args = parse_arguments()
    tasks = build_tasks(args)
    groups = select_tasks(tasks, args)

    if args.target != "":
        found = False
        for task in tasks:
            if task.task_id == args.target:
                found = True
        if found == False:
            sys.stderr.write("Unknown task id '" + args.target + "'. Use --list to see the ids.\n")
            return 2

    if args.list == True:
        print_plan(tasks, groups)
        return 0

    commit = current_commit()
    dirty = tree_is_dirty()
    revision = "no git"
    if commit != "":
        revision = commit[:7]
        if dirty == True:
            revision = revision + " + uncommitted changes"

    logger.info("Starting run: groups=" + ",".join(groups) + " target=" + args.target + " revision=" + revision)
    sys.stdout.write(COLOR_BOLD + "mtindo checks" + COLOR_RESET + COLOR_DIM + "  (" + ", ".join(groups) + " | " + revision + ")" + COLOR_RESET + "\n")
    sys.stdout.write("========================================\n")

    started = time.time()
    for task in tasks:
        if task.status == "EXCLUDED":
            continue
        if task.status == "PENDING":
            run_task(task)
        print_task_line(task)
    elapsed = time.time() - started

    if args.bless == True:
        print_blessed_changes()

    failed = print_summary(tasks, elapsed)
    if failed == 0:
        ran_default_groups = "gates" in groups and "tests" in groups
        if args.target == "" and args.bless == False and ran_default_groups == True:
            record_success(commit, dirty)
        return 0

    print_changes_since_last_good(commit, dirty)
    return 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        if current_process != None:
            current_process.kill()
        sys.stdout.write("\n" + COLOR_YELLOW + "Interrupted." + COLOR_RESET + "\n")
        logger.info("Run interrupted by the user")
        sys.exit(130)
