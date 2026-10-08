#!/usr/bin/env python3

import sys
import time
import subprocess
import threading
import argparse
import pathlib

project_root = pathlib.Path(__file__).parent.parent.resolve()
sys.path.insert(0, str(project_root))

from scripts.common.logger import FileLogger

COLOR_RED = "\033[91m"
COLOR_GREEN = "\033[92m"
COLOR_YELLOW = "\033[93m"
COLOR_CYAN = "\033[96m"
COLOR_RESET = "\033[0m"
COLOR_BOLD = "\033[1m"

if sys.stdout.isatty() == False:
    COLOR_RED = ""
    COLOR_GREEN = ""
    COLOR_YELLOW = ""
    COLOR_CYAN = ""
    COLOR_RESET = ""
    COLOR_BOLD = ""

logger = FileLogger("run_tests")

class Spinner:
    def __init__(self, message):
        self.message = message
        self.is_running = False
        self.thread = None
        self.start_time = 0.0
        self.frames = []
        self.frames.append("⠋")
        self.frames.append("⠙")
        self.frames.append("⠹")
        self.frames.append("⠸")
        self.frames.append("⠼")
        self.frames.append("⠴")
        self.frames.append("⠦")
        self.frames.append("⠧")
        self.frames.append("⠇")
        self.frames.append("⠏")

    def spin(self):
        idx = 0
        while self.is_running == True:
            frame = self.frames[idx]
            sys.stdout.write("\r\033[K")
            sys.stdout.write(COLOR_CYAN)
            sys.stdout.write(frame)
            sys.stdout.write(COLOR_RESET)
            sys.stdout.write(" ")
            sys.stdout.write(self.message)
            sys.stdout.flush()
            
            idx = idx + 1
            if idx >= len(self.frames):
                idx = 0
            time.sleep(0.1)

    def start(self):
        self.is_running = True
        self.start_time = time.time()
        self.thread = threading.Thread(target=self.spin)
        self.thread.start()

    def stop(self, success, details):
        self.is_running = False
        self.thread.join()
        elapsed = time.time() - self.start_time
        
        sys.stdout.write("\r\033[K")
        if success == True:
            sys.stdout.write(COLOR_GREEN)
            sys.stdout.write("✓")
            
            log_msg = "Task Passed: " + self.message
            logger.info(log_msg)
        else:
            sys.stdout.write(COLOR_RED)
            sys.stdout.write("✗")
            
            log_msg = "Task Failed: " + self.message
            logger.error(log_msg)
        
        sys.stdout.write(COLOR_RESET)
        sys.stdout.write(" ")
        sys.stdout.write(self.message)
        sys.stdout.write(" (")
        sys.stdout.write(str(round(elapsed, 2)))
        sys.stdout.write("s)\n")
        
        if success == False:
            if details != "":
                sys.stdout.write(details)
                sys.stdout.write("\n")
                
                logger.error("Failure Details:\n" + details)
        
        sys.stdout.flush()

def run_command(name, command_args):
    command_string = " ".join(command_args)
    log_msg = "Executing: " + command_string
    logger.info(log_msg)
    
    spinner = Spinner(name)
    spinner.start()
    
    process = subprocess.Popen(
        command_args,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True
    )
    
    output_data = process.communicate()[0]
    exit_code = process.returncode
    
    debug_msg = "Raw Output for " + name + ":\n" + output_data
    logger.debug(debug_msg)
    
    if exit_code == 0:
        spinner.stop(True, "")
        return True
    else:
        spinner.stop(False, output_data)
        return False

class FuzzWorker(threading.Thread):
    def __init__(self, target_name, time_limit):
        super().__init__()
        self.target_name = target_name
        self.time_limit = time_limit
        self.success = False
        self.output = ""

    def run(self):
        command = []
        command.append("cargo")
        command.append("+nightly")
        command.append("fuzz")
        command.append("run")
        command.append(self.target_name)
        command.append("--")
        time_arg = "-max_total_time=" + str(self.time_limit)
        command.append(time_arg)
        
        command_string = " ".join(command)
        log_msg = "Fuzzing Thread Started: " + command_string
        logger.info(log_msg)
        
        process = subprocess.Popen(
            command,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True
        )
        self.output = process.communicate()[0]
        
        debug_msg = "Fuzz Output for " + self.target_name + ":\n" + self.output
        logger.debug(debug_msg)
        
        if process.returncode == 0:
            self.success = True
        else:
            self.success = False

def get_fuzz_targets():
    fuzz_targets = []
    fuzz_dir = pathlib.Path("fuzz") / "fuzz_targets"
    
    if fuzz_dir.exists() == True:
        for file_path in fuzz_dir.iterdir():
            if file_path.suffix == ".rs":
                target_name = file_path.stem
                fuzz_targets.append(target_name)
                
    return fuzz_targets

def discover_standard_tasks():
    tasks = []
    tests_dir = pathlib.Path("tests")
    
    if tests_dir.exists() == True:
        file_list = []
        for file_path in tests_dir.iterdir():
            if file_path.suffix == ".rs":
                file_list.append(file_path)
                
        def sort_key(path_obj):
            name = path_obj.stem
            if name.startswith("domain_"):
                parts = name.split("_")
                if len(parts) > 1:
                    if parts[1].isdigit() == True:
                        return int(parts[1])
            return 999
            
        file_list.sort(key=sort_key)
        
        for file_path in file_list:
            target_name = file_path.stem
            if target_name.startswith("domain_"):
                name_parts = target_name.split("_")
                
                domain_num = name_parts[1]
                domain_words = name_parts[2:]
                domain_desc = " ".join(domain_words).title()
                
                task_title = "Domain " + domain_num + ": " + domain_desc
                command_args = ["cargo", "test", "--test", target_name, "--quiet"]
                
                tasks.append((task_title, target_name, command_args))
                
    benches_dir = pathlib.Path("benches")
    if benches_dir.exists() == True:
        bench_command = ["cargo", "bench", "--quiet"]
        tasks.append(("Benchmarks", "mtindo_bench", bench_command))
        
    return tasks

def get_current_git_hash():
    process = subprocess.Popen(
        ["git", "rev-parse", "HEAD"],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True
    )
    output = process.communicate()[0]
    
    if process.returncode == 0:
        return output.strip()
    return ""

def save_success_state(commit_hash):
    state_file = pathlib.Path("scripts") / "logs" / ".last_success"
    file_handle = open(state_file, "w", encoding="utf-8")
    file_handle.write(commit_hash)
    file_handle.close()

def get_last_success_state():
    state_file = pathlib.Path("scripts") / "logs" / ".last_success"
    if state_file.exists() == True:
        file_handle = open(state_file, "r", encoding="utf-8")
        content = file_handle.read().strip()
        file_handle.close()
        return content
    return ""

def print_failure_diff(last_good_hash):
    if last_good_hash == "":
        return
        
    sys.stdout.write("\n")
    sys.stdout.write("========================================\n")
    sys.stdout.write(COLOR_YELLOW)
    sys.stdout.write("STATE DRIFT DETECTED\n")
    sys.stdout.write(COLOR_RESET)
    
    hash_short = last_good_hash[:7]
    sys.stdout.write("Generating diff against last known-good state (")
    sys.stdout.write(hash_short)
    sys.stdout.write(")...\n")
    sys.stdout.write("========================================\n\n")
    
    process = subprocess.Popen(
        ["git", "diff", last_good_hash, "HEAD"],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True
    )
    output = process.communicate()[0]
    
    if output.strip() == "":
        sys.stdout.write("No uncommitted changes found. The failure is likely caused by environment state.\n")
    else:
        sys.stdout.write(output)
        
    sys.stdout.write("\n")
    logger.error("Failure Diff against " + hash_short + ":\n" + output)

def parse_arguments():
    parser = argparse.ArgumentParser(description="mtindo test orchestrator")
    
    parser.add_argument(
        "--target",
        type=str,
        default="",
        help="Run a specific test name exactly (e.g., domain_1_unit)."
    )
    
    parser.add_argument(
        "--group",
        type=str,
        default="all",
        choices=["all", "standard", "fuzz"],
        help="Target a specific domain group. Default is 'all'."
    )
    
    parser.add_argument(
        "--fuzz-time",
        type=int,
        default=10,
        help="Execution time limit per fuzz target in seconds."
    )
    
    return parser.parse_args()

def main():
    args = parse_arguments()
    cargo_path = pathlib.Path("Cargo.toml")
    
    if cargo_path.exists() == False:
        sys.stderr.write("FATAL: Execute this script from the project root directory.\n")
        sys.exit(1)
        
    logger.info("Starting mtindo 110% Coverage Pipeline")
    
    arg_msg = "Arguments passed: target=" + args.target + ", group=" + args.group + ", fuzz-time=" + str(args.fuzz_time)
    logger.info(arg_msg)

    current_hash = get_current_git_hash()
    standard_tasks = discover_standard_tasks()
    
    tasks_to_run = []
    if args.group == "all" or args.group == "standard":
        for task in standard_tasks:
            target_key = task[1]
            if args.target == "" or args.target == target_key:
                tasks_to_run.append(task)
                
    sys.stdout.write(COLOR_BOLD)
    sys.stdout.write("Executing mtindo 110% Coverage Pipeline\n")
    sys.stdout.write(COLOR_RESET)
    sys.stdout.write("========================================\n")
    
    start_total = time.time()
    failed_count = 0
    
    for task in tasks_to_run:
        task_title = task[0]
        task_command = task[2]
        
        success = run_command(task_title, task_command)
        if success == False:
            failed_count = failed_count + 1
    
    if args.group == "all" or args.group == "fuzz":
        fuzz_targets = get_fuzz_targets()
        
        if len(fuzz_targets) > 0 and (args.target == "" or args.target in fuzz_targets):
            
            fuzz_targets_to_run = []
            for t in fuzz_targets:
                if args.target == "" or args.target == t:
                    fuzz_targets_to_run.append(t)
            
            if len(fuzz_targets_to_run) > 0:
                spinner_msg = "Domain 4: Fuzzing (" + str(len(fuzz_targets_to_run)) + " targets concurrently, " + str(args.fuzz_time) + "s limit)"
                fuzz_spinner = Spinner(spinner_msg)
                fuzz_spinner.start()
                
                workers = []
                for target_name in fuzz_targets_to_run:
                    worker = FuzzWorker(target_name, args.fuzz_time)
                    worker.start()
                    workers.append(worker)
                
                for worker in workers:
                    worker.join()
                
                fuzz_spinner.stop(True, "")
                
                for worker in workers:
                    status_prefix = "  -> " + worker.target_name + ": "
                    if worker.success == True:
                        sys.stdout.write(status_prefix)
                        sys.stdout.write(COLOR_GREEN)
                        sys.stdout.write("CLEAN\n")
                        sys.stdout.write(COLOR_RESET)
                        
                        clean_msg = worker.target_name + " Fuzzing completed cleanly."
                        logger.info(clean_msg)
                    else:
                        failed_count = failed_count + 1
                        sys.stdout.write(status_prefix)
                        sys.stdout.write(COLOR_RED)
                        sys.stdout.write("PANIC DETECTED\n")
                        sys.stdout.write(COLOR_RESET)
                        sys.stdout.write(worker.output)
                        sys.stdout.write("\n")
                        
                        panic_msg = worker.target_name + " Fuzzing encountered a panic."
                        logger.error(panic_msg)
    
    elapsed_total = time.time() - start_total
    
    if failed_count == 0:
        if current_hash != "":
            save_success_state(current_hash)
            
        sys.stdout.write("========================================\n")
        sys.stdout.write(COLOR_GREEN)
        sys.stdout.write(COLOR_BOLD)
        sys.stdout.write("SUCCESS: All domains passed in ")
        sys.stdout.write(str(round(elapsed_total, 2)))
        sys.stdout.write("s\n")
        sys.stdout.write(COLOR_RESET)
    else:
        last_good = get_last_success_state()
        if last_good != "" and last_good != current_hash:
            print_failure_diff(last_good)
            
        sys.stdout.write("========================================\n")
        sys.stdout.write(COLOR_RED)
        sys.stdout.write(COLOR_BOLD)
        sys.stdout.write("FAILURE: ")
        sys.stdout.write(str(failed_count))
        sys.stdout.write(" domains failed. Total time: ")
        sys.stdout.write(str(round(elapsed_total, 2)))
        sys.stdout.write("s\n")
        sys.stdout.write(COLOR_RESET)
    
    summary_msg = "Pipeline Execution Complete. Failed domains: " + str(failed_count)
    logger.info(summary_msg)
    
    if failed_count > 0:
        sys.exit(1)
    else:
        sys.exit(0)

if __name__ == "__main__":
    main()
