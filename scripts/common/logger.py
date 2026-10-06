import pathlib
import time

class FileLogger:
    def __init__(self, script_name):
        self.script_name = script_name
        self.logs_dir = pathlib.Path("scripts") / "logs"
        self.max_bytes = 5 * 1024 * 1024
        self.backup_count = 3
        
        if self.logs_dir.exists() == False:
            self.logs_dir.mkdir(parents=True)
            
        log_filename = self.script_name + ".log"
        self.log_path = self.logs_dir / log_filename
        
        start_message = "=== INITIALIZED LOG FOR " + self.script_name + " ==="
        self.info(start_message)

    def _rotate(self):
        if self.log_path.exists() == False:
            return
            
        file_size = self.log_path.stat().st_size
        if file_size < self.max_bytes:
            return
            
        for i in range(self.backup_count - 1, 0, -1):
            source_name = self.script_name + ".log." + str(i)
            source_path = self.logs_dir / source_name
            
            target_name = self.script_name + ".log." + str(i + 1)
            target_path = self.logs_dir / target_name
            
            if source_path.exists() == True:
                source_path.rename(target_path)
                
        target_name = self.script_name + ".log.1"
        target_path = self.logs_dir / target_name
        self.log_path.rename(target_path)

    def _write(self, level, message):
        self._rotate()
        
        time_struct = time.localtime()
        time_string = time.strftime("%Y-%m-%d %H:%M:%S", time_struct)
        
        log_entry = "[" + time_string + "] "
        log_entry = log_entry + "[" + level + "] "
        log_entry = log_entry + message
        log_entry = log_entry + "\n"
        
        log_file = open(self.log_path, "a", encoding="utf-8")
        log_file.write(log_entry)
        log_file.close()

    def info(self, message):
        self._write("INFO", message)

    def error(self, message):
        self._write("ERROR", message)
        
    def debug(self, message):
        self._write("DEBUG", message)
