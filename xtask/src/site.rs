//! `cargo xtask site`: the public docs as one site, built by mdBook into `target/site/`.
//! Nothing of it is kept by hand: the pages are `docs/` (its working notes aside), and the
//! table of contents is `docs/README.md`'s own, in its order: its headings become parts,
//! its links chapters, and a linked index page (`design/README.md`) brings the pages it
//! links, under it. A link that leaves `docs/` (a record, the source) becomes a link to the
//! file on GitHub, where it can be read.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use crate::common::process;

const SITE: &str = "target/site";

/// Where the book's sources are put together, and where the site is built.
const SOURCES: &str = "target/site-src";

pub fn run(root: &Path, _m: &clap::ArgMatches) -> ExitCode {
  match build(root) {
    Ok(pages) => {
      println!(
        "site: {} page(s) in {}; open {}/index.html",
        pages, SITE, SITE
      );
      ExitCode::SUCCESS
    }
    Err(e) => {
      println!("site: {}", e);
      ExitCode::from(1)
    }
  }
}

fn build(root: &Path) -> Result<usize, String> {
  crate::flow::require(root, "site")?;
  let flow = crate::flow::config::load(root)?;
  let docs = root.join("docs");
  let sources = root.join(SOURCES);
  let _ = fs::remove_dir_all(&sources);
  let src = sources.join("src");
  let mut pages = 0;
  for file in files(&docs) {
    let relative = file.strip_prefix(&docs).unwrap_or(&file);
    let to = src.join(relative);
    if let Some(parent) = to.parent() {
      fs::create_dir_all(parent).map_err(|e| format!("{}: {}", parent.display(), e))?;
    }
    if file.extension().is_some_and(|e| e == "md") {
      let text = fs::read_to_string(&file).map_err(|e| format!("{}: {}", file.display(), e))?;
      let base = format!("https://github.com/{}/blob/main", flow.release.repository);
      fs::write(
        &to,
        outward_links(without_front_matter(&text), relative, &base),
      )
      .map_err(|e| format!("{}: {}", to.display(), e))?;
      pages += 1;
    } else {
      fs::copy(&file, &to).map_err(|e| format!("{}: {}", file.display(), e))?;
    }
  }
  let index =
    fs::read_to_string(docs.join("README.md")).map_err(|e| format!("docs/README.md: {}", e))?;
  let base = format!("https://github.com/{}/blob/main", flow.release.repository);
  // The root's pages the index names, in the book as project/NAME.md.
  fs::create_dir_all(src.join("project")).map_err(|e| format!("project: {}", e))?;
  for name in project_pages(&index) {
    let text = fs::read_to_string(root.join(&name)).map_err(|e| format!("{}: {}", name, e))?;
    fs::write(
      src.join("project").join(&name),
      root_page_links(&text, &base),
    )
    .map_err(|e| format!("project/{}: {}", name, e))?;
    pages += 1;
  }
  let summary = summary(
    &index,
    |page| fs::read_to_string(docs.join(page)).ok(),
    |dir| {
      let mut names: Vec<String> = fs::read_dir(docs.join(dir))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
      names.sort();
      names
    },
  );
  // Every public page is in the book: mdBook builds only the contents' pages, so one
  // the index does not reach would be missing from the site without a word.
  let missing: Vec<String> = files(&docs)
    .iter()
    .filter(|f| f.extension().is_some_and(|e| e == "md"))
    .filter_map(|f| f.strip_prefix(&docs).ok())
    .map(|f| f.display().to_string().replace('\\', "/"))
    .filter(|page| page != "README.md" && !summary.contains(&format!("({})", page)))
    .collect();
  if !missing.is_empty() {
    return Err(format!(
      "docs/README.md reaches no chapter for: {} (link it, or link it from an index it links)",
      missing.join(", ")
    ));
  }
  fs::write(src.join("SUMMARY.md"), summary).map_err(|e| format!("SUMMARY.md: {}", e))?;
  fs::write(
    sources.join("book.toml"),
    format!(
      "[book]\ntitle = \"mtindo\"\nlanguage = \"en\"\nsrc = \"src\"\n\n[build]\nbuild-dir = \"{}\"\n\n[output.html]\ngit-repository-url = \"https://github.com/{}\"\n",
      root.join(SITE).display(),
      flow.release.repository
    ),
  )
  .map_err(|e| format!("book.toml: {}", e))?;
  let built = process::output(
    Command::new("mdbook")
      .arg("build")
      .arg(&sources)
      .stdout(Stdio::piped())
      .stderr(Stdio::piped()),
    process::LONG,
  )
  .map_err(|e| format!("cannot run mdbook: {}", e))?;
  if built.code != 0 {
    return Err(format!(
      "mdbook failed:\n{}",
      String::from_utf8_lossy(&built.stderr).trim_end()
    ));
  }
  Ok(pages)
}

fn capitalized(text: &str) -> String {
  let mut chars = text.chars();
  match chars.next() {
    Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    None => String::new(),
  }
}

/// The repository path a link from `page` leads to, when it leaves `docs/`.
fn escaping(page: &Path, target: &str) -> Option<String> {
  if target.contains("://") || target.starts_with('#') || target.starts_with('/') {
    return None;
  }
  let (path, anchor) = match target.split_once('#') {
    Some((p, a)) => (p, format!("#{}", a)),
    None => (target, String::new()),
  };
  // Folders from the docs root down to the page's own, then the link's steps.
  let mut parts: Vec<String> = page
    .parent()
    .map(|p| {
      p.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect()
    })
    .unwrap_or_default();
  let mut depth: i64 = parts.len() as i64;
  for step in path.split('/') {
    match step {
      "" | "." => {}
      ".." => {
        depth -= 1;
        if parts.pop().is_none() {
          // Above docs/: a step out of it, into the repository.
          parts.push("..".to_string());
        }
      }
      name => {
        depth += 1;
        parts.push(name.to_string());
      }
    }
  }
  if depth >= 0 && !parts.iter().any(|p| p == "..") {
    return None;
  }
  let inside: Vec<&String> = parts.iter().filter(|p| *p != "..").collect();
  Some(format!(
    "{}{}",
    inside
      .iter()
      .map(|s| s.as_str())
      .collect::<Vec<_>>()
      .join("/"),
    anchor
  ))
}

/// The public files under `docs`: all but the working notes and the system's droppings.
fn files(docs: &Path) -> Vec<PathBuf> {
  let mut out = Vec::new();
  let mut stack = vec![docs.to_path_buf()];
  while let Some(dir) = stack.pop() {
    for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
      let path = entry.path();
      let name = entry.file_name().to_string_lossy().into_owned();
      if name.starts_with('.') || path == docs.join("internal") {
        continue;
      }
      if path.is_dir() {
        stack.push(path);
      } else {
        out.push(path);
      }
    }
  }
  out.sort();
  out
}

/// Every `[title](target)` in a line.
fn inline_links(line: &str) -> Vec<(String, String)> {
  let mut out = Vec::new();
  let mut rest = line;
  while let Some(open) = rest.find('[') {
    let after = &rest[open + 1..];
    let Some(close) = after.find("](") else {
      break;
    };
    let target_start = &after[close + 2..];
    let Some(end) = target_start.find(')') else {
      break;
    };
    out.push((after[..close].to_string(), target_start[..end].to_string()));
    rest = &target_start[end + 1..];
  }
  out
}

/// The `[title](page)` a list item starts with.
fn list_link(line: &str) -> Option<(String, String)> {
  let rest = line
    .trim_start()
    .strip_prefix("* ")
    .or_else(|| line.trim_start().strip_prefix("- "))?;
  inline_links(rest)
    .into_iter()
    .next()
    .filter(|_| rest.starts_with('['))
}

/// `text` with every relative link that leaves `docs/` pointed at the file on GitHub
/// (`base`), resolved from the page's own folder (`page`, relative to `docs/`).
fn outward_links(text: &str, page: &Path, base: &str) -> String {
  let mut out = String::with_capacity(text.len());
  let mut rest = text;
  while let Some(at) = rest.find("](") {
    out.push_str(&rest[..at + 2]);
    let after = &rest[at + 2..];
    let end = after.find(')').unwrap_or(after.len());
    let target = &after[..end];
    match escaping(page, target) {
      Some(repo_path) => out.push_str(&format!("{}/{}", base, repo_path)),
      None => out.push_str(target),
    }
    rest = &after[end..];
  }
  out.push_str(rest);
  out
}

/// The pages of the repository's root the docs' index links (`../NAME.md`).
fn project_pages(index: &str) -> Vec<String> {
  index
    .lines()
    .filter_map(list_link)
    .filter_map(|(_, page)| page.strip_prefix("../").map(str::to_string))
    .filter(|name| !name.contains('/') && name.ends_with(".md"))
    .collect()
}

/// A root page's links as the book has it at `project/NAME.md`: one into `docs/` points at
/// that page of the book, any other at the file on GitHub (`base`).
fn root_page_links(text: &str, base: &str) -> String {
  let mut out = String::with_capacity(text.len());
  let mut rest = text;
  while let Some(at) = rest.find("](") {
    out.push_str(&rest[..at + 2]);
    let after = &rest[at + 2..];
    let end = after.find(')').unwrap_or(after.len());
    let target = &after[..end];
    if target.contains("://") || target.starts_with('#') || target.starts_with('/') {
      out.push_str(target);
    } else if let Some(page) = target.strip_prefix("docs/") {
      out.push_str(&format!("../{}", page));
    } else if target.ends_with(".md") && !target.contains('/') {
      // Another root page: in the book beside this one when the index names it.
      out.push_str(target);
    } else {
      out.push_str(&format!("{}/{}", base, target.trim_start_matches("./")));
    }
    rest = &after[end..];
  }
  out.push_str(rest);
  out
}

/// The book's table of contents from the docs' index: `## Part` headings, `* [Title](page)`
/// links as chapters (what follows the link dropped), and an index page linked in a
/// sentence (`the [design pages](design/README.md)`) as a chapter where it stands. An index
/// page brings every page of its folder under it: those it links, in its order (a table's
/// links too), then the rest by name. A page of the repository's root it links (`../
/// CONTRIBUTING.md`) is a chapter as `project/NAME.md`, where [`build`] puts it. `folder`
/// lists a folder's pages.
fn summary(
  index: &str,
  read: impl Fn(&str) -> Option<String>,
  folder: impl Fn(&str) -> Vec<String>,
) -> String {
  let mut out = String::from("# Summary\n\n[mtindo documentation](README.md)\n");
  let chapter = |out: &mut String, title: &str, page: &str| {
    out.push_str(&format!("- [{}]({})\n", title, page));
    if let Some(dir) = page.strip_suffix("README.md") {
      let mut pages: Vec<String> = Vec::new();
      for (_, linked) in read(page)
        .unwrap_or_default()
        .lines()
        .flat_map(inline_links)
      {
        let linked = linked.split('#').next().unwrap_or("").to_string();
        if linked.ends_with(".md")
          && !linked.contains('/')
          && linked != "README.md"
          && !pages.contains(&linked)
        {
          pages.push(linked);
        }
      }
      for name in folder(dir) {
        if name != "README.md" && !pages.contains(&name) {
          pages.push(name);
        }
      }
      // Not a page that is a chapter of its own already.
      pages.retain(|name| !out.contains(&format!("({}{})", dir, name)));
      for name in pages {
        let title = read(&format!("{}{}", dir, name))
          .and_then(|text| {
            text
              .lines()
              .find_map(|l| l.strip_prefix("# "))
              .map(str::to_string)
          })
          .unwrap_or_else(|| name.trim_end_matches(".md").to_string());
        out.push_str(&format!("  - [{}]({}{})\n", title, dir, name));
      }
    }
  };
  for line in index.lines() {
    if let Some(part) = line.strip_prefix("## ") {
      out.push_str(&format!("\n# {}\n\n", part.trim()));
    } else if let Some((title, page)) = list_link(line) {
      if let Some(name) = page
        .strip_prefix("../")
        .filter(|n| !n.contains('/') && n.ends_with(".md"))
      {
        out.push_str(&format!("- [{}](project/{})\n", title, name));
      } else if !page.contains("://") && !page.starts_with("../") {
        chapter(&mut out, &title, &page);
      }
    } else {
      for (title, page) in inline_links(line) {
        if page.ends_with("README.md")
          && !page.starts_with("../")
          && !page.contains("://")
          && !out.contains(&format!("({})", page))
        {
          chapter(&mut out, &capitalized(&title), &page);
        }
      }
    }
  }
  out
}

/// `text` without the YAML front matter a generated page opens with (between two `---`
/// lines): data for scripts, which a site would show as a run of text.
fn without_front_matter(text: &str) -> &str {
  let Some(rest) = text.strip_prefix("---\n") else {
    return text;
  };
  match rest.find("\n---\n") {
    Some(end) => rest[end + 5..].trim_start_matches('\n'),
    None => text,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_a_root_page_links_into_the_book_or_to_github() {
    let base = "https://github.com/o/r/blob/main";
    assert_eq!(
      root_page_links(
        "[g](docs/guide/layout.md), [f](flow.toml), [m](MAINTAINING.md), [w](https://x)",
        base
      ),
      "[g](../guide/layout.md), [f](https://github.com/o/r/blob/main/flow.toml), [m](MAINTAINING.md), [w](https://x)"
    );
    assert_eq!(
      project_pages("* [C](../CONTRIBUTING.md): x\n* [L](guide/layout.md)\n"),
      vec!["CONTRIBUTING.md"]
    );
  }

  #[test]
  fn test_front_matter_is_left_off_the_site() {
    assert_eq!(
      without_front_matter("---\ntitle: x\nkeys:\n  total: 18\n---\n\n# Settings\n\nText.\n"),
      "# Settings\n\nText.\n"
    );
    assert_eq!(
      without_front_matter("# Plain\n\n---\n\nrule\n"),
      "# Plain\n\n---\n\nrule\n"
    );
  }

  #[test]
  fn test_links_out_of_the_docs_go_to_github() {
    let base = "https://github.com/o/r/blob/main";
    let page = Path::new("generated/metrics/compatibility.md");
    let text = "the [record](../../../records/results/compatibility.txt), a [page](meaning.md), [up](../commands.md#x), [web](https://x.org)";
    assert_eq!(
      outward_links(text, page, base),
      "the [record](https://github.com/o/r/blob/main/records/results/compatibility.txt), a [page](meaning.md), [up](../commands.md#x), [web](https://x.org)"
    );
    assert_eq!(
      outward_links(
        "[readme](../README.md#install)",
        Path::new("README.md"),
        base
      ),
      "[readme](https://github.com/o/r/blob/main/README.md#install)"
    );
  }

  #[test]
  fn test_the_contents_follow_the_docs_index() {
    let index = "# mtindo documentation\n\nSee the [README](../README.md).\n\n## Project\n\n* [CONTRIBUTING.md](../CONTRIBUTING.md): how.\n\n## Guide\n\n* [Layout](guide/layout.md): turning layout on.\n* [Formatting](guide/formatting.md): the style,\n  wrapped.\n\n## Reference\n\n* [Metrics](generated/metrics/README.md): as measured.\n\n## Design\n\nHow it works: the [design pages](design/README.md).\n";
    let read = |page: &str| match page {
      "generated/metrics/README.md" => {
        Some("| [compatibility](compatibility.md) | x |\n".to_string())
      }
      "generated/metrics/compatibility.md" => Some("# Compatibility\n".to_string()),
      "design/README.md" => {
        Some("* [Overview](overview.md): the whole.\n* [Elsewhere](../x.md)\n".to_string())
      }
      "design/overview.md" => Some("# Overview\n".to_string()),
      _ => None,
    };
    let folder = |dir: &str| match dir {
      "generated/metrics/" => vec![
        "README.md".to_string(),
        "compatibility.md".to_string(),
        "dictionary.md".to_string(),
      ],
      _ => Vec::new(),
    };
    let summary = summary(index, read, folder);
    assert_eq!(
      summary,
      "# Summary\n\n[mtindo documentation](README.md)\n\n# Project\n\n- [CONTRIBUTING.md](project/CONTRIBUTING.md)\n\n# Guide\n\n- [Layout](guide/layout.md)\n- [Formatting](guide/formatting.md)\n\n# Reference\n\n- [Metrics](generated/metrics/README.md)\n  - [Compatibility](generated/metrics/compatibility.md)\n  - [dictionary](generated/metrics/dictionary.md)\n\n# Design\n\n- [Design pages](design/README.md)\n  - [Overview](design/overview.md)\n"
    );
  }
}
