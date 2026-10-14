//! Screens' layout trees cut into rectangles, one per widget.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

use crate::config::{LayoutSpec, Size};

/// A size as a constraint: `12`, `"30%"`, `"min:10"`, `"max:40"`, `"fill"`.
pub fn constraint(size: &Size) -> Option<Constraint> {
  Some(match size {
    Size::Cells(n) => Constraint::Length(*n),
    Size::Text(t) => {
      if t == "fill" {
        Constraint::Fill(1)
      } else if let Some(p) = t.strip_suffix('%') {
        Constraint::Percentage(p.parse().ok()?)
      } else if let Some(n) = t.strip_prefix("min:") {
        Constraint::Min(n.parse().ok()?)
      } else if let Some(n) = t.strip_prefix("max:") {
        Constraint::Max(n.parse().ok()?)
      } else {
        Constraint::Length(t.parse().ok()?)
      }
    }
  })
}

/// Each widget of `layout` with the rectangle it gets in `area`, in order.
pub fn place(layout: &LayoutSpec, area: Rect, out: &mut Vec<(String, Rect)>) {
  match layout {
    LayoutSpec::Widget(id) => out.push((id.clone(), area)),
    LayoutSpec::Split {
      split,
      sizes,
      children,
    } => {
      let direction = if split == "columns" {
        Direction::Horizontal
      } else {
        Direction::Vertical
      };
      let constraints: Vec<Constraint> = sizes
        .iter()
        .map(|s| constraint(s).unwrap_or(Constraint::Fill(1)))
        .collect();
      let areas = Layout::default()
        .direction(direction)
        .constraints(constraints)
        .split(area);
      for (child, rect) in children.iter().zip(areas.iter()) {
        place(child, *rect, out);
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_a_layout_tree_is_cut_as_asked() {
    let layout = LayoutSpec::Split {
      split: "columns".into(),
      sizes: vec![Size::Cells(40), Size::Text("fill".into())],
      children: vec![
        LayoutSpec::Widget("left".into()),
        LayoutSpec::Split {
          split: "rows".into(),
          sizes: vec![Size::Text("50%".into()), Size::Text("fill".into())],
          children: vec![
            LayoutSpec::Widget("top".into()),
            LayoutSpec::Widget("bottom".into()),
          ],
        },
      ],
    };
    let mut out = Vec::new();
    place(&layout, Rect::new(0, 0, 100, 30), &mut out);
    assert_eq!(out[0], ("left".to_string(), Rect::new(0, 0, 40, 30)));
    assert_eq!(out[1], ("top".to_string(), Rect::new(40, 0, 60, 15)));
    assert_eq!(out[2], ("bottom".to_string(), Rect::new(40, 15, 60, 15)));
    assert!(constraint(&Size::Text("wide".into())).is_none());
  }
}
