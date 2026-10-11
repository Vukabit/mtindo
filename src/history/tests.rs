use super::*;

/// Makes every stored object old enough for pruning to remove.
fn age_objects(history: &History) {
  let old = SystemTime::now() - GRACE * 2;
  for shard in fs::read_dir(history.dir().join("objects")).unwrap() {
    for file in fs::read_dir(shard.unwrap().path()).unwrap() {
      let file = OpenOptions::new()
        .write(true)
        .open(file.unwrap().path())
        .unwrap();
      file.set_modified(old).unwrap();
    }
  }
}

fn history(name: &str) -> History {
  let dir = std::env::temp_dir().join(format!("mtindo-history-{}-{}", name, std::process::id()));
  let _ = fs::remove_dir_all(&dir);
  History::new(dir)
}

fn record(history: &History, entries: &[(&str, &[u8], &[u8])]) -> String {
  let mut recorder = history.recorder("mto -R", Path::new("/work"), None);
  for (path, before, after) in entries {
    let entry = Entry {
      path: PathBuf::from(path),
      before: history.store(before).unwrap(),
      after: history.store(after).unwrap(),
    };
    recorder.record(&entry).unwrap();
  }
  assert!(recorder.finish().unwrap() || entries.is_empty());
  recorder.id().to_string()
}

#[test]
fn test_a_record_cut_short_keeps_its_whole_lines() {
  let text = format!(
    "{}\nid x\ncommand mto\ncwd /\nfile aa bb /f.rs\nfile cc dd /g",
    MAGIC
  );
  let run = parse(&text).unwrap();
  assert_eq!(run.entries.len(), 1);
  assert!(parse("not a record\n").is_none());
}

#[test]
fn test_a_run_that_writes_nothing_leaves_no_record() {
  let history = history("empty");
  let mut recorder = history.recorder("mto", Path::new("/"), None);
  assert!(!recorder.finish().unwrap());
  assert!(history.ids().unwrap().is_empty());
}

#[test]
fn test_concurrent_runs_record_apart() {
  let history = history("concurrent");
  let ids: Vec<String> = std::thread::scope(|scope| {
    let handles: Vec<_> = (0..8)
      .map(|i| {
        let history = &history;
        scope.spawn(move || {
          let before = format!("before {}", i % 2);
          let after = format!("after {}", i);
          record(
            history,
            &[("/same.rs", before.as_bytes(), after.as_bytes())],
          )
        })
      })
      .collect();
    handles.into_iter().map(|h| h.join().unwrap()).collect()
  });
  assert_eq!(history.ids().unwrap().len(), 8);
  for id in ids {
    let run = history.read(&id).unwrap();
    assert!(history.load(&run.entries[0].before).is_ok());
  }
  let _ = fs::remove_dir_all(history.dir());
}

#[test]
fn test_contents_are_stored_once() {
  let history = history("dedup");
  record(
    &history,
    &[("/a.rs", b"same", b"x"), ("/b.rs", b"same", b"y")],
  );
  let shard = &sha256::hex(b"same")[..2];
  let files = fs::read_dir(history.dir().join("objects").join(shard))
    .unwrap()
    .count();
  assert_eq!(files, 1);
  let _ = fs::remove_dir_all(history.dir());
}

#[test]
fn test_only_a_sweep_finds_what_a_run_cut_short_left() {
  let history = history("sweep");
  record(&history, &[("/a.rs", b"a", b"b")]);
  // Stored by a run that ended before recording it.
  history.store(b"orphan").unwrap();
  age_objects(&history);
  let quick = history.prune(Limits::default(), false).unwrap();
  assert_eq!(quick, Pruned::default());
  let swept = history.prune(Limits::default(), true).unwrap();
  assert_eq!(swept.objects, 1);
  assert!(history.load(&sha256::hex(b"orphan")).is_err());
  assert!(history.load(&sha256::hex(b"a")).is_ok());
  let _ = fs::remove_dir_all(history.dir());
}

#[test]
fn test_pruning_by_size_drops_the_oldest_first() {
  let history = history("size");
  let big = vec![b'x'; 4000];
  record(&history, &[("/a.rs", &big, b"1")]);
  std::thread::sleep(Duration::from_millis(2));
  let second = record(&history, &[("/b.rs", b"2", b"3")]);
  let pruned = history
    .prune(
      Limits {
        max_bytes: 1000,
        ..Limits::default()
      },
      true,
    )
    .unwrap();
  assert_eq!(pruned.runs, 1);
  assert_eq!(history.ids().unwrap(), [second]);
  let _ = fs::remove_dir_all(history.dir());
}

#[test]
fn test_pruning_keeps_contents_another_run_shares() {
  let history = history("prune");
  let old = record(&history, &[("/a.rs", b"shared", b"only-old")]);
  std::thread::sleep(Duration::from_millis(2));
  let new = record(&history, &[("/b.rs", b"shared", b"only-new")]);
  age_objects(&history);
  let pruned = history
    .prune(
      Limits {
        keep_runs: 1,
        ..Limits::default()
      },
      true,
    )
    .unwrap();
  assert_eq!(pruned.runs, 1);
  assert_eq!(pruned.objects, 1, "only `only-old` goes");
  assert_eq!(history.ids().unwrap(), std::slice::from_ref(&new));
  assert!(history.read(&old).is_err());
  assert_eq!(history.load(&sha256::hex(b"shared")).unwrap(), b"shared");
  assert!(history.load(&sha256::hex(b"only-old")).is_err());
  let _ = fs::remove_dir_all(history.dir());
}

#[test]
fn test_pruning_spares_young_contents_and_the_newest_run() {
  let history = history("young");
  record(&history, &[("/a.rs", b"a", b"b")]);
  // Stored by a run that has not recorded it yet.
  history.store(b"in flight").unwrap();
  let pruned = history
    .prune(
      Limits {
        max_bytes: 1,
        ..Limits::default()
      },
      true,
    )
    .unwrap();
  assert_eq!(pruned, Pruned::default());
  assert_eq!(history.ids().unwrap().len(), 1);
  assert!(history.load(&sha256::hex(b"in flight")).is_ok());
  let _ = fs::remove_dir_all(history.dir());
}

#[test]
fn test_records_read_back_with_odd_paths() {
  let history = history("odd");
  let id = record(&history, &[("/a b/new\nline\\x.rs", b"old", b"new")]);
  let run = history.read(&id).unwrap();
  assert_eq!(run.command, "mto -R");
  assert_eq!(run.cwd, PathBuf::from("/work"));
  assert_eq!(run.entries.len(), 1);
  assert_eq!(run.entries[0].path, PathBuf::from("/a b/new\nline\\x.rs"));
  assert_eq!(history.load(&run.entries[0].before).unwrap(), b"old");
  assert_eq!(history.load(&run.entries[0].after).unwrap(), b"new");
  let _ = fs::remove_dir_all(history.dir());
}

#[test]
fn test_runs_resolve_by_last_and_by_handle() {
  let history = history("resolve");
  let first = record(&history, &[("/a.rs", b"1", b"2")]);
  // Ids sort by milliseconds; keep the two apart.
  std::thread::sleep(Duration::from_millis(2));
  let second = record(&history, &[("/a.rs", b"2", b"3")]);
  assert_eq!(history.resolve("last").unwrap(), second);
  assert_eq!(history.resolve("last~1").unwrap(), first);
  assert!(history.resolve("last~2").is_err());
  let ids = history.ids().unwrap();
  let handle = id::handle(&first, &ids);
  assert_eq!(history.resolve(&handle).unwrap(), first);
  assert_eq!(history.resolve(&first).unwrap(), first);
  assert!(history.resolve("zzzz").is_err());
  assert!(history.resolve("last~x").is_err());
  let _ = fs::remove_dir_all(history.dir());
}

#[test]
fn test_state_compares_the_file_with_both_hashes() {
  let history = history("state");
  fs::create_dir_all(history.dir()).unwrap();
  let path = history.dir().join("f.rs");
  let entry = Entry {
    path: path.clone(),
    before: sha256::hex(b"before"),
    after: sha256::hex(b"after"),
  };
  assert_eq!(Run::state(&entry), State::Missing);
  fs::write(&path, "after").unwrap();
  assert_eq!(Run::state(&entry), State::AsWritten);
  fs::write(&path, "before").unwrap();
  assert_eq!(Run::state(&entry), State::AsBefore);
  fs::write(&path, "edited").unwrap();
  assert_eq!(Run::state(&entry), State::Changed);
  let _ = fs::remove_dir_all(history.dir());
}
