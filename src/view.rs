//! Design-pixel layout and paint. The ring retains its original reference coordinates.

use crate::geom::{hit_test, point_at, Hit, Pt, DESIGN_H, DESIGN_W, HANDLE_R, RING_CENTER, RING_RADIUS, RING_STROKE};
use crate::gfx::{Align, Canvas, Color, Gfx};
use crate::model::{duration, fmt_hhmm, format_date, Schedule, Task, DAY, DAY_F};

const TEXT: Color = Color::rgb(0x0D0D0D);
const MUTED: Color = Color::rgb(0x666666);
const LINE: Color = Color::rgb(0xE5E5E5);
const RED: Color = Color::rgb(0xD55F57);
const WHITE: Color = Color::rgb(0xFFFFFF);
const SWATCH: [u32; 5] = [0x438778, 0xC38C54, 0x748ABD, 0xB184AC, 0x87918B];

const POP_W: f32 = 292.0;
const POP_H: f32 = 204.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    None,
    Name,
    Start,
    End,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TimeField {
    Start,
    End,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hot {
    None,
    Ring(Hit),
    Chip(u64),
    Field(Focus),
    Clock(TimeField),
    Color(usize),
    Delete,
    Undo,
    Hour(i32),
    Minute(i32),
    Popup,
}

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(self, point: Pt) -> bool {
        point.x >= self.x && point.y >= self.y && point.x < self.x + self.w && point.y < self.y + self.h
    }

    pub fn right(self) -> f32 {
        self.x + self.w
    }

    pub fn center(self) -> Pt {
        Pt::new(self.x + self.w * 0.5, self.y + self.h * 0.5)
    }
}

pub struct UiLayout {
    pub chips: Vec<(u64, Rect)>,
    pub name: Rect,
    pub start: Rect,
    pub end: Rect,
    pub clock_start: Rect,
    pub clock_end: Rect,
    pub toolbar: Option<ToolbarLayout>,
    pub undo: Rect,
    pub popup: Option<PopupLayout>,
}

pub struct ToolbarLayout {
    pub colors: [Rect; 5],
    pub delete: Rect,
    from: Pt,
    to: Pt,
}

const TOOL_RADIUS: f32 = 166.0;

pub struct PopupLayout {
    pub panel: Rect,
    pub hours: [Rect; 24],
    pub minutes: [Rect; 4],
}

pub struct Scene<'a> {
    pub schedule: &'a Schedule,
    pub now_min: f32,
    pub date: (i32, u32, u32),
    pub name_text: &'a str,
    pub start_text: &'a str,
    pub end_text: &'a str,
    pub focus: Focus,
    pub caret: usize,
    pub anchor: usize,
    pub caret_on: bool,
    pub hover: Hot,
    pub popup: Option<TimeField>,
    pub time_error: Option<TimeField>,
    pub deleted_name: Option<&'a str>,
}

pub fn canvas_for(client_w: f32, client_h: f32) -> Canvas {
    let k = (client_w / DESIGN_W).min(client_h / DESIGN_H).max(0.01);
    Canvas {
        k,
        origin: ((client_w - DESIGN_W * k) * 0.5, (client_h - DESIGN_H * k) * 0.5),
    }
}

pub fn to_design(canvas: Canvas, x: f32, y: f32) -> Pt {
    Pt::new((x - canvas.origin.0) / canvas.k, (y - canvas.origin.1) / canvas.k)
}

pub fn layout(
    gfx: &mut Gfx,
    schedule: &Schedule,
    popup: Option<(TimeField, Pt)>,
    canvas: Canvas,
    toolbar_visible: bool,
) -> UiLayout {
    let tasks = schedule.tasks_by_start();
    let mut widths = Vec::with_capacity(tasks.len());
    for task in &tasks {
        widths.push(chip_width(gfx, task));
    }
    let gap = 18.0;
    let total = widths.iter().sum::<f32>() + gap * widths.len().saturating_sub(1) as f32;
    let mut x = 494.5 - total * 0.5;
    let mut chips = Vec::with_capacity(tasks.len());
    for (task, width) in tasks.iter().zip(widths) {
        chips.push((task.id, Rect::new(x, 757.0 - 21.0, width, 42.0)));
        x += width + gap;
    }

    let name = Rect::new(RING_CENTER.x - 170.0, 364.0, 340.0, 48.0);
    let start_w = gfx.measure("00:00", 16.0, false);
    let gap = gfx.measure("  —  ", 16.0, false);
    let start = Rect::new(RING_CENTER.x - start_w - gap * 0.5 - 8.0, 414.0, start_w + 16.0, 32.0);
    let end = Rect::new(RING_CENTER.x + gap * 0.5 - 8.0, 414.0, start_w + 16.0, 32.0);
    UiLayout {
        chips,
        name,
        start,
        end,
        clock_start: Rect::new(start.x - 26.0, 416.0, 24.0, 28.0),
        clock_end: Rect::new(end.right() + 2.0, 416.0, 24.0, 28.0),
        toolbar: schedule.selected_task().filter(|_| toolbar_visible).and_then(|task| {
            let reserved = toolbar_reserves(gfx, task, name, start, end);
            toolbar_layout(task, &reserved)
        }),
        undo: Rect::new(RING_CENTER.x - 190.0, 833.0, 380.0, 34.0),
        popup: popup.map(|(field, anchor)| popup_layout(field, anchor, canvas)),
    }
}

pub fn hit(layout: &UiLayout, schedule: &Schedule, point: Pt, undo_visible: bool) -> Hot {
    if let Some(popup) = &layout.popup {
        for (hour, rect) in popup.hours.iter().enumerate() {
            if rect.contains(point) {
                return Hot::Hour(hour as i32);
            }
        }
        for (index, rect) in popup.minutes.iter().enumerate() {
            if rect.contains(point) {
                return Hot::Minute([0, 15, 30, 45][index]);
            }
        }
        if popup.panel.contains(point) {
            return Hot::Popup;
        }
    }
    if undo_visible && layout.undo.contains(point) {
        return Hot::Undo;
    }
    if let Some(toolbar) = &layout.toolbar {
        if toolbar.delete.contains(point) {
            return Hot::Delete;
        }
        for (index, rect) in toolbar.colors.iter().enumerate() {
            if rect.contains(point) {
                return Hot::Color(index);
            }
        }
    }
    if schedule.selected_task().is_some() {
        if layout.clock_start.contains(point) {
            return Hot::Clock(TimeField::Start);
        }
        if layout.clock_end.contains(point) {
            return Hot::Clock(TimeField::End);
        }
        for (id, rect) in &layout.chips {
            if rect.contains(point) {
                return Hot::Chip(*id);
            }
        }
        if layout.name.contains(point) {
            return Hot::Field(Focus::Name);
        }
        if layout.start.contains(point) {
            return Hot::Field(Focus::Start);
        }
        if layout.end.contains(point) {
            return Hot::Field(Focus::End);
        }
    }
    Hot::Ring(hit_test(schedule.tasks(), schedule.selected_id(), point))
}

pub fn paint(gfx: &mut Gfx, scene: &Scene<'_>, layout: &UiLayout) {
    paint_header(gfx, scene);
    paint_ring(gfx, scene, layout);
    paint_legend(gfx, scene, layout);
    text_center(gfx, "点击空白圆环添加 · 拖动色段移动 · 拖动白点调整时长", 494.5, 811.0, 14.5, false, MUTED);
    if let Some(toolbar) = &layout.toolbar {
        paint_toolbar(gfx, scene, toolbar);
    }
    if let Some(name) = scene.deleted_name {
        let rect = layout.undo;
        gfx.fill(rect.x, rect.y, rect.w, rect.h, 12.0, WHITE);
        gfx.stroke(rect.x, rect.y, rect.w, rect.h, 12.0, 1.0, LINE);
        gfx.text(&format!("已删除「{name}」"), rect.x + 12.0, rect.y, rect.w - 82.0, rect.h, 13.0, false, MUTED, Align::Left, true);
        text_right(gfx, "撤销", rect.right() - 14.0, rect.center().y, 13.0, false, TEXT);
    }
    if let Some(popup) = &layout.popup {
        paint_popup(gfx, scene, popup);
    }
}

pub fn field_rect(layout: &UiLayout, focus: Focus) -> Option<Rect> {
    match focus {
        Focus::Name => Some(layout.name),
        Focus::Start => Some(layout.start),
        Focus::End => Some(layout.end),
        Focus::None => None,
    }
}

fn paint_header(gfx: &mut Gfx, scene: &Scene<'_>) {
    let (year, month, day) = scene.date;
    text_left(gfx, "dayring", 129.0, 63.0, 26.0, true, TEXT);
    text_right(gfx, &format_date(year, month, day), 846.0, 66.0, 15.0, false, MUTED);
}

fn paint_ring(gfx: &mut Gfx, scene: &Scene<'_>, layout: &UiLayout) {
    for hour in 0..24 {
        let minutes = hour as f32 * 60.0;
        let major = hour % 3 == 0;
        let (inner, outer, width, color) = if major {
            (248.0, 266.0, 1.7, Color::rgb(0xA0A0A0))
        } else {
            (252.0, 261.0, 1.15, Color::rgb(0xD4D4D4))
        };
        let a = point_at(RING_CENTER, inner, minutes);
        let b = point_at(RING_CENTER, outer, minutes);
        gfx.line(a.x, a.y, b.x, b.y, width, color);
        if major {
            let label = point_at(RING_CENTER, 292.0, minutes);
            text_center(gfx, &format!("{hour:02}"), label.x, label.y, 15.0, false, MUTED);
        }
    }

    let selected = scene.schedule.selected_id();
    for task in scene.schedule.tasks() {
        if Some(task.id) != selected {
            paint_arc(gfx, task, false);
        }
    }
    if let Some(task) = scene.schedule.selected_task() {
        paint_arc(gfx, task, true);
        for minutes in [task.start_min, task.end_min] {
            let point = point_at(RING_CENTER, RING_RADIUS, minutes as f32);
            gfx.circle(point.x, point.y, HANDLE_R, WHITE);
            gfx.circle_stroke(point.x, point.y, HANDLE_R, 1.3, Color::rgb(0xD8D8D8));
        }
    }

    let now = scene.now_min;
    let inner = point_at(RING_CENTER, RING_RADIUS - RING_STROKE * 0.48, now);
    let outer = point_at(RING_CENTER, RING_RADIUS + RING_STROKE * 0.52, now);
    gfx.line(inner.x, inner.y, outer.x, outer.y, 2.0, RED);
    gfx.circle(outer.x, outer.y, 4.6, RED);
    paint_center(gfx, scene, layout);
}

fn paint_arc(gfx: &mut Gfx, task: &Task, selected: bool) {
    let from = point_at(RING_CENTER, RING_RADIUS, task.start_min as f32);
    let to = point_at(RING_CENTER, RING_RADIUS, task.end_min as f32);
    let large = duration(task.start_min, task.end_min) > DAY / 2;
    if selected {
        gfx.arc(
            (from.x, from.y),
            (to.x, to.y),
            RING_RADIUS,
            RING_STROKE + 18.0,
            halo(task.color),
            large,
        );
    }
    gfx.arc(
        (from.x, from.y),
        (to.x, to.y),
        RING_RADIUS,
        RING_STROKE,
        arc_fill(task.color, selected),
        large,
    );
}

fn paint_center(gfx: &mut Gfx, scene: &Scene<'_>, layout: &UiLayout) {
    if scene.schedule.selected_task().is_some() {
        text_center(gfx, "当前选中", RING_CENTER.x, 349.0, 14.0, false, MUTED);
        if scene.focus == Focus::Name {
            paint_inline_field(gfx, layout.name, scene.name_text, scene, Focus::Name);
        } else {
            let name = if scene.name_text.is_empty() { "未命名" } else { scene.name_text };
            gfx.text(name, layout.name.x, layout.name.y, layout.name.w, layout.name.h, 36.0, true, TEXT, Align::Center, true);
            paint_field_hint(gfx, layout.name, scene.hover == Hot::Field(Focus::Name));
        }
        for (rect, text, focus, field) in [
            (layout.start, scene.start_text, Focus::Start, TimeField::Start),
            (layout.end, scene.end_text, Focus::End, TimeField::End),
        ] {
            if scene.focus == focus {
                paint_inline_field(gfx, rect, text, scene, focus);
            } else {
                text_center(gfx, text, rect.center().x, rect.center().y, 16.0, false, MUTED);
                paint_field_hint(gfx, rect, scene.hover == Hot::Field(focus));
            }
            let clock = if field == TimeField::Start { layout.clock_start } else { layout.clock_end };
            if scene.focus == focus || scene.hover == Hot::Field(focus)
                || scene.hover == Hot::Clock(field) || scene.popup == Some(field) {
                paint_clock(gfx, clock.center(), 6.5);
            }
        }
        text_center(gfx, "—", RING_CENTER.x, 430.0, 16.0, false, MUTED);
        if scene.time_error.is_some() {
            text_center(gfx, "请输入有效时间，如 09:30", RING_CENTER.x, 462.0, 12.0, false, RED);
        } else {
            gfx.line(RING_CENTER.x - 16.0, 462.0, RING_CENTER.x + 16.0, 462.0, 1.0, Color::rgb(0xE4E4E4));
        }
    } else {
        text_center(gfx, "暂无安排", RING_CENTER.x, 388.0, 28.0, true, TEXT);
        text_center(gfx, "点击圆环添加安排", RING_CENTER.x, 430.0, 16.0, false, MUTED);
        gfx.line(RING_CENTER.x - 16.0, 462.0, RING_CENTER.x + 16.0, 462.0, 1.0, Color::rgb(0xE4E4E4));
    }
    let now = (scene.now_min.rem_euclid(DAY_F)) as i32;
    text_center(gfx, &format!("现在 {}", fmt_hhmm(now)), RING_CENTER.x, 494.0, 15.0, false, MUTED);
}

fn paint_legend(gfx: &mut Gfx, scene: &Scene<'_>, layout: &UiLayout) {
    let selected = scene.schedule.selected_id();
    for (id, rect) in &layout.chips {
        let Some(task) = scene.schedule.tasks().iter().find(|task| task.id == *id) else {
            continue;
        };
        if Some(task.id) == selected {
            gfx.fill(rect.x, rect.y, rect.w, rect.h, 16.0, WHITE);
            gfx.stroke(rect.x, rect.y, rect.w, rect.h, 16.0, 1.0, LINE);
        }
        let dot = Pt::new(rect.x + 16.0, rect.center().y);
        gfx.circle(dot.x, dot.y, 5.5, Color::rgb(SWATCH[task.color % SWATCH.len()]));
        text_left(
            gfx,
            &format!("{}  {}", task.name, fmt_hhmm(task.start_min)),
            dot.x + 12.0,
            rect.center().y,
            14.5,
            false,
            MUTED,
        );
    }
}

fn toolbar_reserves(gfx: &mut Gfx, task: &Task, name: Rect, start: Rect, end: Rect) -> [Rect; 5] {
    let title = if task.name.is_empty() { "未命名" } else { &task.name };
    let title_w = gfx.measure(title, 36.0, true).min(name.w);
    let label_w = gfx.measure("当前选中", 14.0, false);
    let now_w = gfx.measure("现在 00:00", 15.0, false);
    [
        Rect::new(RING_CENTER.x - label_w * 0.5 - 6.0, 333.0, label_w + 12.0, 32.0),
        Rect::new(RING_CENTER.x - title_w * 0.5 - 6.0, name.y - 3.0, title_w + 12.0, name.h + 6.0),
        // Include the clocks beside the two time fields, which can appear on hover.
        Rect::new(start.x - 30.0, start.y - 4.0, end.right() - start.x + 60.0, start.h + 8.0),
        Rect::new(RING_CENTER.x - 20.0, 459.0, 40.0, 6.0),
        Rect::new(RING_CENTER.x - now_w * 0.5 - 6.0, 477.0, now_w + 12.0, 34.0),
    ]
}

const TOOL_FROM: f32 = -66.0;
const TOOL_TO: f32 = 76.0;
const TOOL_CLEARANCE: f32 = 22.0; // Enclose the painted arc and full click bounds, plus clearance.
const ANGLE_EPS: f32 = 0.0001;

fn expanded(rect: Rect, padding: f32) -> Rect {
    Rect::new(rect.x - padding, rect.y - padding, rect.w + padding * 2.0, rect.h + padding * 2.0)
}

// Angles where the toolbar centerline crosses a rectangle. These boundaries split
// the circular orbit into intervals with a constant inside/outside classification.
fn circle_rect_intervals(rect: Rect) -> Vec<(f32, f32)> {
    use std::f32::consts::{PI, TAU};
    let mut cuts = vec![0.0, TAU];
    for x in [rect.x, rect.right()] {
        let sin = (x - RING_CENTER.x) / TOOL_RADIUS;
        if (-1.0..=1.0).contains(&sin) {
            let angle = sin.asin();
            cuts.extend([angle.rem_euclid(TAU), (PI - angle).rem_euclid(TAU)]);
        }
    }
    for y in [rect.y, rect.y + rect.h] {
        let cos = (RING_CENTER.y - y) / TOOL_RADIUS;
        if (-1.0..=1.0).contains(&cos) {
            let angle = cos.acos();
            cuts.extend([angle, (TAU - angle).rem_euclid(TAU)]);
        }
    }
    cuts.sort_by(f32::total_cmp);
    cuts.dedup_by(|a, b| (*a - *b).abs() < 0.000001);
    cuts.windows(2).filter_map(|pair| {
        let point = point_at(RING_CENTER, TOOL_RADIUS, (pair[0] + pair[1]) * 0.5 / TAU * DAY_F);
        rect.contains(point).then_some((pair[0], pair[1]))
    }).collect()
}

// Find the nearest legal anchor on a circle, including intervals that wrap midnight.
// Expand the obstacles by the entire painted arc, rather than checking only buttons.
fn toolbar_anchor(preferred: f32, reserved: &[Rect]) -> Option<f32> {
    use std::f32::consts::TAU;
    let mut blocked = Vec::<(f32, f32)>::new();
    for rect in reserved {
        for (from, to) in circle_rect_intervals(expanded(*rect, TOOL_CLEARANCE)) {
            let begin = from - TOOL_TO / TOOL_RADIUS - ANGLE_EPS;
            let finish = to - TOOL_FROM / TOOL_RADIUS + ANGLE_EPS;
            let span = finish - begin;
            if span >= TAU { return None; }
            let begin = begin.rem_euclid(TAU);
            let finish = begin + span;
            if finish <= TAU {
                blocked.push((begin, finish));
            } else {
                blocked.extend([(begin, TAU), (0.0, finish - TAU)]);
            }
        }
    }
    blocked.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged = Vec::<(f32, f32)>::new();
    for (from, to) in blocked {
        if let Some(last) = merged.last_mut().filter(|last| last.1 >= from) {
            last.1 = last.1.max(to);
        } else {
            merged.push((from, to));
        }
    }
    let mut available = Vec::new();
    let mut cursor = 0.0;
    for (from, to) in merged {
        if from > cursor { available.push((cursor, from)); }
        cursor = to;
    }
    if cursor < TAU { available.push((cursor, TAU)); }
    let preferred = preferred.rem_euclid(TAU);
    if available.iter().any(|(from, to)| preferred >= *from && preferred <= *to) {
        return Some(preferred);
    }
    let mut best = None;
    let mut best_distance = f32::INFINITY;
    for (from, to) in available {
        for target in [preferred - TAU, preferred, preferred + TAU] {
            let candidate = target.clamp(from, to);
            let distance = (candidate - target).abs();
            if distance < best_distance {
                best = Some(candidate.rem_euclid(TAU));
                best_distance = distance;
            }
        }
    }
    best
}

fn toolbar_layout(task: &Task, reserved: &[Rect]) -> Option<ToolbarLayout> {
    use std::f32::consts::TAU;
    let midpoint = task.start_min as f32 + duration(task.start_min, task.end_min) as f32 * 0.5;
    // The unequal color/delete spacing has a geometric center five pixels after
    // the old zero offset. Align the visible arc midpoint with the task midpoint.
    let preferred = midpoint / DAY_F * TAU - (TOOL_FROM + TOOL_TO) * 0.5 / TOOL_RADIUS;
    let anchor = toolbar_anchor(preferred, reserved)?;
    let at = |offset: f32| point_at(RING_CENTER, TOOL_RADIUS, (anchor + offset / TOOL_RADIUS) / TAU * DAY_F);
    let colors = [-66.0, -41.0, -16.0, 9.0, 34.0].map(|offset| {
        let point = at(offset);
        Rect::new(point.x - 11.0, point.y - 11.0, 22.0, 22.0)
    });
    let delete = at(TOOL_TO);
    Some(ToolbarLayout {
        colors,
        delete: Rect::new(delete.x - 16.0, delete.y - 12.0, 32.0, 24.0),
        from: at(TOOL_FROM),
        to: at(TOOL_TO),
    })
}

fn paint_toolbar(gfx: &mut Gfx, scene: &Scene<'_>, toolbar: &ToolbarLayout) {
    let from = (toolbar.from.x, toolbar.from.y);
    let to = (toolbar.to.x, toolbar.to.y);
    gfx.arc(from, to, TOOL_RADIUS, 28.0, LINE, false);
    gfx.arc(from, to, TOOL_RADIUS, 26.0, WHITE, false);
    let selected_color = scene.schedule.selected_task().map(|task| task.color);
    for (index, rect) in toolbar.colors.iter().enumerate() {
        let center = rect.center();
        if Some(index) == selected_color || scene.hover == Hot::Color(index) {
            gfx.circle_stroke(center.x, center.y, 10.0, 1.2, Color::rgb(0x1A1A1A));
        }
        gfx.circle(center.x, center.y, 8.0, Color::rgb(SWATCH[index]));
    }
    let center = toolbar.delete.center();
    text_center(gfx, "删除", center.x, center.y, 12.0, false, if scene.hover == Hot::Delete { RED } else { MUTED });
}

fn paint_field_hint(gfx: &mut Gfx, rect: Rect, hovered: bool) {
    if hovered {
        gfx.line(rect.x + 8.0, rect.y + rect.h - 1.0, rect.right() - 8.0, rect.y + rect.h - 1.0, 1.0, LINE);
    }
}

pub fn field_style(focus: Focus) -> (f32, bool) {
    if focus == Focus::Name { (36.0, true) } else { (16.0, false) }
}

pub fn field_text_x(gfx: &mut Gfx, rect: Rect, focus: Focus, text: &str, caret: usize) -> f32 {
    let (size, bold) = field_style(focus);
    let width = gfx.measure(text, size, bold);
    let available = rect.w - 16.0;
    if width <= available {
        return rect.x + (rect.w - width) * 0.5;
    }
    let before_caret = gfx.measure(&text[..caret.min(text.len())], size, bold);
    let shift = (before_caret - available + 8.0).max(0.0).min(width - available);
    rect.x + 8.0 - shift
}

fn paint_inline_field(gfx: &mut Gfx, rect: Rect, text: &str, scene: &Scene<'_>, focus: Focus) {
    let error = matches!((focus, scene.time_error), (Focus::Start, Some(TimeField::Start)) | (Focus::End, Some(TimeField::End)));
    gfx.fill(rect.x, rect.y, rect.w, rect.h, 7.0, WHITE);
    gfx.stroke(rect.x, rect.y, rect.w, rect.h, 7.0, 1.0, if error { RED } else { LINE });
    let (size, bold) = field_style(focus);
    let caret = scene.caret.min(text.len());
    let anchor = scene.anchor.min(text.len());
    let x = field_text_x(gfx, rect, focus, text, caret);
    gfx.clip(rect.x + 6.0, rect.y + 2.0, rect.w - 12.0, rect.h - 4.0);
    if caret != anchor {
        let (start, end) = if caret < anchor { (caret, anchor) } else { (anchor, caret) };
        let x0 = x + gfx.measure(&text[..start], size, bold);
        let x1 = x + gfx.measure(&text[..end], size, bold);
        gfx.fill(x0, rect.y + 5.0, (x1 - x0).max(1.0), rect.h - 10.0, 3.0, Color::rgb(0xE8E8E8));
    }
    gfx.text(text, x, rect.y, 4_000.0, rect.h, size, bold, TEXT, Align::Left, false);
    if scene.caret_on {
        let caret_x = x + gfx.measure(&text[..caret], size, bold);
        gfx.line(caret_x, rect.y + 5.0, caret_x, rect.y + rect.h - 5.0, 1.15, TEXT);
    }
    gfx.unclip();
}

fn paint_popup(gfx: &mut Gfx, scene: &Scene<'_>, popup: &PopupLayout) {
    let Some(field) = scene.popup else {
        return;
    };
    let panel = popup.panel;
    gfx.fill(panel.x, panel.y, panel.w, panel.h, 12.0, WHITE);
    gfx.stroke(panel.x, panel.y, panel.w, panel.h, 12.0, 1.0, LINE);
    let label = match field {
        TimeField::Start => "开始时间",
        TimeField::End => "结束时间",
    };
    text_left(gfx, label, panel.x + 12.0, panel.y + 18.0, 13.0, false, MUTED);
    let current = scene.schedule.selected_task().map(|task| match field {
        TimeField::Start => task.start_min,
        TimeField::End => task.end_min,
    });
    let Some(current) = current else {
        return;
    };
    for (hour, rect) in popup.hours.iter().enumerate() {
        let on = current / 60 == hour as i32;
        paint_cell(gfx, *rect, &format!("{hour:02}"), on);
    }
    for (index, rect) in popup.minutes.iter().enumerate() {
        let minute = [0, 15, 30, 45][index];
        let on = current % 60 == minute;
        paint_cell(gfx, *rect, &format!("{minute:02}"), on);
    }
}

fn paint_cell(gfx: &mut Gfx, rect: Rect, text: &str, on: bool) {
    if on {
        gfx.fill(rect.x, rect.y, rect.w, rect.h, 8.0, Color::rgb(0xF4F4F4));
        gfx.stroke(rect.x, rect.y, rect.w, rect.h, 8.0, 1.0, LINE);
    }
    let center = rect.center();
    text_center(gfx, text, center.x, center.y, 13.0, false, if on { TEXT } else { MUTED });
}

fn paint_clock(gfx: &mut Gfx, center: Pt, radius: f32) {
    let color = Color::rgb(0x8A8A8A);
    let width = (radius * 0.16).max(1.0);
    gfx.circle_stroke(center.x, center.y, radius, width, color);
    gfx.line(center.x, center.y, center.x, center.y - radius * 0.55, width, color);
    gfx.line(center.x, center.y, center.x + radius * 0.42, center.y + radius * 0.12, width, color);
}

fn popup_layout(_field: TimeField, anchor: Pt, canvas: Canvas) -> PopupLayout {
    let visible = visible_design(canvas);
    let mut x = anchor.x - 36.0;
    let mut y = anchor.y + 16.0;
    if x + POP_W > visible.right() - 8.0 {
        x = visible.right() - POP_W - 8.0;
    }
    if y + POP_H > visible.y + visible.h - 8.0 {
        y = (anchor.y - POP_H - 10.0).max(visible.y + 8.0);
    }
    x = x.max(visible.x + 8.0);
    y = y.max(visible.y + 8.0);
    let panel = Rect::new(x, y, POP_W, POP_H);
    let hours_top = panel.y + 34.0;
    let mut hours = [Rect::new(0.0, 0.0, 0.0, 0.0); 24];
    for hour in 0..24 {
        let col = hour % 6;
        let row = hour / 6;
        hours[hour] = Rect::new(
            panel.x + 12.0 + col as f32 * 44.0,
            hours_top + row as f32 * 34.0,
            36.0,
            26.0,
        );
    }
    let minutes_top = hours_top + 4.0 * 34.0 - 8.0 + 6.0;
    let mut minutes = [Rect::new(0.0, 0.0, 0.0, 0.0); 4];
    for index in 0..4 {
        minutes[index] = Rect::new(panel.x + 12.0 + index as f32 * 44.0, minutes_top, 36.0, 26.0);
    }
    PopupLayout { panel, hours, minutes }
}

fn visible_design(canvas: Canvas) -> Rect {
    Rect::new(
        -canvas.origin.0 / canvas.k,
        -canvas.origin.1 / canvas.k,
        canvas_client_w(canvas) / canvas.k,
        canvas_client_h(canvas) / canvas.k,
    )
}

fn canvas_client_w(canvas: Canvas) -> f32 {
    canvas.origin.0 * 2.0 + DESIGN_W * canvas.k
}

fn canvas_client_h(canvas: Canvas) -> f32 {
    canvas.origin.1 * 2.0 + DESIGN_H * canvas.k
}

fn chip_width(gfx: &mut Gfx, task: &Task) -> f32 {
    16.0 + 12.0 + gfx.measure(&format!("{}  {}", task.name, fmt_hhmm(task.start_min)), 14.5, false) + 14.0
}

fn arc_fill(index: usize, selected: bool) -> Color {
    let hex = SWATCH[index % SWATCH.len()];
    if selected {
        return Color::rgb(hex);
    }
    let mix = |channel: u32| ((channel * 212 + 255 * 43) / 255) as u8;
    Color {
        r: mix((hex >> 16) & 255) as f32 / 255.0,
        g: mix((hex >> 8) & 255) as f32 / 255.0,
        b: mix(hex & 255) as f32 / 255.0,
        a: 1.0,
    }
}

fn halo(index: usize) -> Color {
    let hex = SWATCH[index % SWATCH.len()];
    let mix = |channel: u32| (channel as f32 * 0.15 + 255.0 * 0.85).round() as u8;
    Color {
        r: mix((hex >> 16) & 255) as f32 / 255.0,
        g: mix((hex >> 8) & 255) as f32 / 255.0,
        b: mix(hex & 255) as f32 / 255.0,
        a: 1.0,
    }
}

fn text_left(gfx: &mut Gfx, text: &str, x: f32, y: f32, size: f32, bold: bool, color: Color) {
    gfx.text(text, x, y - size, 900.0, size * 2.0, size, bold, color, Align::Left, false);
}

fn text_center(gfx: &mut Gfx, text: &str, x: f32, y: f32, size: f32, bold: bool, color: Color) {
    gfx.text(text, x - 450.0, y - size, 900.0, size * 2.0, size, bold, color, Align::Center, false);
}

fn text_right(gfx: &mut Gfx, text: &str, x: f32, y: f32, size: f32, bold: bool, color: Color) {
    gfx.text(text, x - 450.0, y - size, 450.0, size * 2.0, size, bold, color, Align::Right, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn design_canvas_fits_and_centers() {
        let exact = canvas_for(DESIGN_W, DESIGN_H);
        assert!((exact.k - 1.0).abs() < 1e-4);
        assert!(exact.origin.0.abs() < 1e-3 && exact.origin.1.abs() < 1e-3);

        let fitted = canvas_for(980.0, 1040.0);
        let expect = (1040.0 / DESIGN_H).min(980.0 / DESIGN_W);
        assert!((fitted.k - expect).abs() < 1e-4);
        let used_h = DESIGN_H * fitted.k;
        assert!((fitted.origin.1 - (1040.0 - used_h) * 0.5).abs() < 1e-3);
        let back = to_design(fitted, fitted.origin.0, fitted.origin.1);
        assert!(back.x.abs() < 1e-3 && back.y.abs() < 1e-3);
    }

    fn test_reserves(gfx: &mut Gfx, task: &Task) -> [Rect; 5] {
        let canvas = canvas_for(DESIGN_W, DESIGN_H);
        let base = layout(gfx, &Schedule::demo(), None, canvas, false);
        toolbar_reserves(gfx, task, base.name, base.start, base.end)
    }

    fn arc_midpoint(toolbar: &ToolbarLayout) -> f32 {
        crate::geom::minutes_at(RING_CENTER, Pt::new(
            (toolbar.from.x + toolbar.to.x) * 0.5,
            (toolbar.from.y + toolbar.to.y) * 0.5,
        ))
    }

    fn circular_distance(a: f32, b: f32) -> f32 {
        let delta = (a - b).rem_euclid(DAY_F);
        delta.min(DAY_F - delta)
    }

    #[test]
    fn reading_toolbar_stays_aligned_instead_of_shifting_towards_midnight() {
        let mut gfx = Gfx::new().unwrap();
        let task = Task { id: 2, name: "阅读".into(), start_min: 19 * 60 + 45, end_min: 22 * 60 + 15, color: 3 };
        let reserved = test_reserves(&mut gfx, &task);
        let toolbar = toolbar_layout(&task, &reserved).unwrap();
        assert!(circular_distance(arc_midpoint(&toolbar), 21.0 * 60.0) < 0.01);
    }

    #[test]
    fn short_title_toolbar_tracks_task_continuously_through_midnight() {
        let mut gfx = Gfx::new().unwrap();
        let mut task = Task { id: 2, name: "阅读".into(), start_min: 0, end_min: 150, color: 3 };
        let reserved = test_reserves(&mut gfx, &task);
        let mut previous = None;
        for start in 0..=DAY {
            task.start_min = start % DAY;
            task.end_min = (start + 150) % DAY;
            let toolbar = toolbar_layout(&task, &reserved).unwrap();
            let midpoint = arc_midpoint(&toolbar);
            assert!(circular_distance(midpoint, start as f32 + 75.0) < 0.01);
            if let Some(last) = previous {
                assert!((circular_distance(last, midpoint) - 1.0).abs() < 0.01);
            }
            previous = Some(midpoint);
        }
    }

    #[test]
    fn inner_toolbar_keeps_entire_background_and_hit_areas_clear() {
        let mut gfx = Gfx::new().unwrap();
        for name in ["阅读", "", "这是一段很长很长的任务名称，会显示省略号"] {
            let mut task = Task { id: 2, name: name.into(), start_min: 0, end_min: 60, color: 0 };
            let reserved = test_reserves(&mut gfx, &task);
            for start in (0..DAY).step_by(15) {
                task.start_min = start;
                task.end_min = (start + 60) % DAY;
                let toolbar = toolbar_layout(&task, &reserved).expect("center information leaves a safe inner arc");
                for rect in toolbar.colors.iter().chain([&toolbar.delete]) {
                    let center = rect.center();
                    assert!((center.dist(RING_CENTER) - TOOL_RADIUS).abs() < 0.01);
                    for region in reserved {
                        assert!(rect.x >= region.right() || rect.right() <= region.x || rect.y >= region.y + region.h || rect.y + rect.h <= region.y);
                    }
                    for corner in [Pt::new(rect.x, rect.y), Pt::new(rect.right(), rect.y), Pt::new(rect.x, rect.y + rect.h), Pt::new(rect.right(), rect.y + rect.h)] {
                        assert!(corner.dist(RING_CENTER) < RING_RADIUS - RING_STROKE * 0.5 - 3.0);
                    }
                }
                let from = crate::geom::minutes_at(RING_CENTER, toolbar.from);
                let span = (TOOL_TO - TOOL_FROM) / TOOL_RADIUS / std::f32::consts::TAU * DAY_F;
                for index in 0..=200 {
                    let point = point_at(RING_CENTER, TOOL_RADIUS, from + span * index as f32 / 200.0);
                    assert!(reserved.iter().all(|region| !expanded(*region, 14.0).contains(point)));
                }
            }
        }
    }

    #[test]
    fn impossible_placement_has_no_arbitrary_midnight_fallback() {
        let task = Task { id: 2, name: "阅读".into(), start_min: 1185, end_min: 1335, color: 3 };
        assert!(toolbar_layout(&task, &[Rect::new(0.0, 0.0, DESIGN_W, DESIGN_H)]).is_none());
    }

    #[test]
    fn toolbar_controls_hit_their_painted_positions() {
        let mut gfx = Gfx::new().unwrap();
        let mut schedule = Schedule::demo();
        let task = schedule.selected_task_mut().unwrap();
        task.name = "阅读".into();
        task.start_min = 19 * 60 + 45;
        task.end_min = 22 * 60 + 15;
        let layout = layout(&mut gfx, &schedule, None, canvas_for(760.0, 632.0), true);
        let toolbar = layout.toolbar.as_ref().unwrap();
        for (index, rect) in toolbar.colors.iter().enumerate() {
            assert_eq!(hit(&layout, &schedule, rect.center(), false), Hot::Color(index));
        }
        assert_eq!(hit(&layout, &schedule, toolbar.delete.center(), false), Hot::Delete);
    }

    #[test]
    fn hidden_controls_and_empty_center_do_not_intercept_ring_creation() {
        let mut gfx = Gfx::new().unwrap();
        let canvas = canvas_for(DESIGN_W, DESIGN_H);
        let mut schedule = Schedule::demo();
        let layout = layout(&mut gfx, &schedule, None, canvas, false);
        assert!(layout.toolbar.is_none());
        let blank = point_at(RING_CENTER, RING_RADIUS, 18.5 * 60.0);
        assert_eq!(hit(&layout, &schedule, blank, false), Hot::Ring(Hit::Empty));
        while schedule.delete_selected().is_some() {}
        assert_eq!(hit(&layout, &schedule, layout.name.center(), false), Hot::Ring(Hit::Miss));
        assert_eq!(hit(&layout, &schedule, layout.clock_start.center(), false), Hot::Ring(Hit::Miss));
        assert_eq!(hit(&layout, &schedule, layout.undo.center(), true), Hot::Undo);
    }
}
