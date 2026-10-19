//! Groups for the Templates and Media Board pages: items laid out under a
//! header per group, and a right-click menu to move an item to a group (or
//! a new one).

use eframe::egui::{self, Align2, Painter, Pos2, Rect, Ui, pos2, vec2};

use super::theme::*;
use crate::i18n::l;

/// The groups in display order: ungrouped first, then as they first appear.
pub fn order(groups: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    if groups.iter().any(|g| g.is_empty()) {
        out.push(String::new());
    }
    for g in groups {
        if !g.is_empty() && !out.iter().any(|o| o == g) {
            out.push(g.to_string());
        }
    }
    out
}

/// Where each item and group header goes. `groups[i]` is item i's group.
/// Headers only appear once some item has a group.
pub struct Grid {
    pub headers: Vec<(Rect, String, usize)>,
    pub cells: Vec<Rect>,
    pub height: f32,
}

pub fn grid(groups: &[&str], origin: Pos2, cols: usize, cell: egui::Vec2, gap: f32) -> Grid {
    let headed = groups.iter().any(|g| !g.is_empty());
    let header_h = if headed { 34.0 } else { 0.0 };
    let mut cells = vec![Rect::NOTHING; groups.len()];
    let mut headers = vec![];
    let mut y = origin.y;
    for g in order(groups) {
        let members: Vec<usize> = (0..groups.len()).filter(|&i| groups[i] == g).collect();
        if headed {
            headers.push((Rect::from_min_size(pos2(origin.x, y), vec2(cols as f32 * (cell.x + gap) - gap, header_h - 8.0)), g.clone(), members.len()));
            y += header_h;
        }
        for (k, &i) in members.iter().enumerate() {
            cells[i] = Rect::from_min_size(pos2(origin.x + (cell.x + gap) * (k % cols) as f32, y + (cell.y + gap) * (k / cols) as f32), cell);
        }
        y += members.len().div_ceil(cols).max(1) as f32 * (cell.y + gap) + 6.0;
    }
    Grid { headers, cells, height: y - origin.y }
}

pub fn draw_headers(p: &Painter, g: &Grid) {
    for (r, name, n) in &g.headers {
        let label = if name.is_empty() { l("Ungrouped", "Sin grupo").to_string() } else { name.clone() };
        let t = p.text(r.left_center(), Align2::LEFT_CENTER, label, semibold(14.0), if name.is_empty() { MUTED } else { TEXT });
        p.text(pos2(t.right() + 10.0, r.center().y), Align2::LEFT_CENTER, n.to_string(), body(12.0), FAINT);
        p.line_segment([pos2(t.right() + 32.0, r.center().y), pos2(r.right(), r.center().y)], egui::Stroke::new(1.0, egui::Color32::from_rgba_premultiplied(50, 50, 56, 60)));
    }
}

/// The "Move to group" menu (inside a context menu). Returns the group
/// chosen ("" = ungrouped).
pub fn menu(ui: &mut Ui, current: &str, all: &[String], input: &mut String) -> Option<String> {
    let mut out = None;
    ui.label(egui::RichText::new(l("Move to group", "Mover al grupo")).color(MUTED).size(12.0));
    if ui.selectable_label(current.is_empty(), l("Ungrouped", "Sin grupo")).clicked() {
        out = Some(String::new());
    }
    for g in all.iter().filter(|g| !g.is_empty()) {
        if ui.selectable_label(current == g, g).clicked() {
            out = Some(g.clone());
        }
    }
    ui.separator();
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(input).hint_text(l("New group", "Grupo nuevo")).desired_width(130.0));
        if ui.add_enabled(!input.trim().is_empty(), egui::Button::new(l("Add", "Crear"))).clicked() {
            out = Some(input.trim().to_string());
            input.clear();
        }
    });
    if out.is_some() {
        ui.close();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_in_order() {
        assert_eq!(order(&["b", "", "a", "b"]), ["", "b", "a"]);
        let g = grid(&["", "x", ""], pos2(0.0, 0.0), 2, vec2(10.0, 10.0), 2.0);
        assert_eq!(g.headers.len(), 2);
        assert!(g.cells[1].top() > g.cells[2].top(), "group x comes after the ungrouped row");
        let flat = grid(&["", ""], pos2(0.0, 0.0), 2, vec2(10.0, 10.0), 2.0);
        assert!(flat.headers.is_empty());
    }
}
